//! wgpuによる描画(地形メッシュ・水域・作図・航跡・覆域)。DETAILED_DESIGN.md 6節。
//! 描画は3つのパスで、いずれも内部解像度はcanvasの2倍(スーパーサンプリング)+4倍MSAA:
//! (1) 水域→地形→絶対座標の作図・マーカー・航跡、(2) カメラ固定の作図(あれば。深度を作り直して手前に重ねる)、
//! (3) canvasの解像度へ縮小(`targets::Downsample`)。深度は反転Z(`camera.rs`)。
//!
//! 構成: このファイルは`TerrainRenderer`(状態と描画のパス)。uniform・頂点属性は`uniforms`、
//! 描画先のテクスチャと縮小は`targets`、パイプラインの生成は`pipelines`、作図等の頂点バッチは`overlay`、
//! 視錐台カリングは`frustum`。

mod fade;
mod frustum;
mod model_batch;
mod overlay;
mod pipelines;
mod targets;
mod uniforms;

use std::collections::HashMap;
use std::ops::Range;

use wgpu::util::DeviceExt;

use self::fade::{fade_progress, Fades, MAX_FADE_ENTRIES};
use self::frustum::{is_outside_frustum, position_bounds};
use self::model_batch::ModelBatch;
use self::overlay::VertexBatch;
use self::pipelines::{terrain_shader, Pipelines};
use self::targets::{Downsample, RenderTargets};
use self::uniforms::{
    screen_matrix, uniform_layout, CameraUniform, DrawUniform, UniformSlot, VIEW_DRAW_LIGHT,
    WORLD_DRAW_LIGHT,
};
use super::camera::Camera;
use super::drawing_geometry::DrawingBatches;
use super::loader::MeshKey;
use super::mesh::{TerrainMesh, TerrainVertex};
use super::models::types::{ModelInstance, ModelMesh};
use super::vertex::DrawVertex;

/// 地形メッシュ1個分のGPUバッファ(タイル全体、またはチャンク1個)。メッシュごとに頂点・
/// インデックスを別々に持ち、解像度レベルの切り替え(`set_mesh`)を1個ずつ行えるようにしてある。
struct MeshGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    num_indices: u32,
    /// 頂点位置を囲む直方体(最小の角, 最大の角。ENU座標)。視錐台の外のメッシュを描かないために使う
    /// (`is_outside_frustum`)。原点変更で頂点位置が変わる(`update_mesh_vertices`)ので更新する。
    bounds: ([f32; 3], [f32; 3]),
}

impl MeshGpu {
    /// パイプライン・bind groupを設定済みの`pass`へ、このメッシュを`instances`の範囲で描く。
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, instances: Range<u32>) {
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.num_indices, 0, instances);
    }
}

/// このフレームでクロスフェードするメッシュ1個(`fade_draws`)。`entry`は割合の表の値(`terrain.wgsl`の`FadeTable`)。
struct FadeDraw<'a> {
    mesh: &'a MeshGpu,
    entry: [f32; 4],
}

/// 現在時刻(ミリ秒)。クロスフェードの経過時間に使う。
fn now_ms() -> f64 {
    js_sys::Date::now()
}

pub struct TerrainRenderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipelines: Pipelines,
    meshes: HashMap<MeshKey, MeshGpu>,
    /// 解像度レベルの切り替え中のメッシュ(クロスフェード。`fade`)と、その割合の表(`terrain.wgsl`の`FadeTable`)。
    fades: Fades<MeshGpu>,
    fade_table: UniformSlot,
    /// 地形・水域・ドームのカメラ(`CameraUniform`)。
    camera: UniformSlot,
    /// スーパーサンプリングの内部解像度の深度・MSAA・解決先。canvasの大きさが変わるたびに作り直す。
    targets: RenderTargets,
    /// 内部解像度→canvas解像度への縮小。
    downsample: Downsample,
    // レーダー観測点のマーカー(画面サイズ固定のピン)と、2D地図モードの覆域(塗り+輪郭線)。どちらも作図と同じ
    // `DrawVertex`・`draw.wgsl`で描く(マーカーは深度テストあり、覆域は深度テストなし。`terrain::markers`)。
    markers: VertexBatch,
    coverage_2d: VertexBatch,
    // 航跡(トラック。`terrain::tracks`): 向きつきのシンボル・航跡・高度線。マーカーと同じく`draw_blend`で描く。
    tracks: VertexBatch,
    // 3Dモデル(`terrain::models`)。不透明で深度を書くので、作図の不透明な図形と同じ位置(覆域ドームより前)で描く。
    models: ModelBatch,
    // 見通し範囲の覆域ドーム(半球状の面、TriangleList)用。地形・マーカーの奥に透けて見える
    // よう、アルファブレンド有効・深度書き込み無効のパイプラインにしてある(fs_dome参照)。
    dome_vertex_buffer: Option<wgpu::Buffer>,
    num_dome_vertices: u32,
    // 陰影(ヒルシェード)を付けるか(`set_hillshade`)。描画のたびにuniformへ書く。
    hillshade: bool,
    // 水域レイヤー(WGS84楕円体の海抜0mの面)の楕円体の係数。現在の原点(地形メッシュの原点)基準で、
    // 原点変更のたびに`set_ellipsoid_origin`で更新する。
    ellipsoid: ([[f32; 4]; 3], [f32; 4]),
    // 作図(`terrain::drawing`)。面も太い線もTriangleListで、`draw.wgsl`が描く。不透明は深度を書き、
    // 半透明は深度を書かずにアルファブレンドする。画面座標だけ深度テストなし(描く順で重ねる)。
    // 座標の種類ごとのuniform: 絶対座標(カメラのview_proj)・視点空間(射影のみ)・画面(ピクセル座標)。
    draw_world: UniformSlot,
    draw_view: UniformSlot,
    draw_screen: UniformSlot,
    world_opaque: VertexBatch,
    world_blend: VertexBatch,
    view_opaque: VertexBatch,
    view_blend: VertexBatch,
    screen_batch: VertexBatch,
}

/// canvasからsurfaceを作る。`SurfaceTarget::Canvas`はwasm32にしか無いため、ネイティブ(単体テスト)では
/// 描画自体が成り立たないものとしてエラーを返す(これでライブラリ全体が`cargo test`でビルドできる)。
#[cfg(target_arch = "wasm32")]
fn create_canvas_surface(
    instance: &wgpu::Instance,
    canvas: web_sys::HtmlCanvasElement,
) -> Result<wgpu::Surface<'static>, String> {
    instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
        .map_err(|e| format!("create_surface failed: {e}"))
}

#[cfg(not(target_arch = "wasm32"))]
fn create_canvas_surface(
    _instance: &wgpu::Instance,
    _canvas: web_sys::HtmlCanvasElement,
) -> Result<wgpu::Surface<'static>, String> {
    Err("canvasへの描画はwasm32でのみ利用できます".to_string())
}

/// canvasにWebGPU(wgpu)のsurfaceを作り、デバイスを得て設定する(sRGB形式を優先、垂直同期)。
async fn init_surface(
    canvas: web_sys::HtmlCanvasElement,
) -> Result<
    (
        wgpu::Surface<'static>,
        wgpu::Device,
        wgpu::Queue,
        wgpu::SurfaceConfiguration,
    ),
    String,
> {
    let width = canvas.width().max(1);
    let height = canvas.height().max(1);

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let surface = create_canvas_surface(&instance, canvas)?;

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            ..Default::default()
        })
        .await
        .map_err(|e| format!("request_adapter failed: {e}"))?;

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .map_err(|e| format!("request_device failed: {e}"))?;

    let surface_caps = surface.get_capabilities(&adapter);
    let mut config = surface
        .get_default_config(&adapter, width, height)
        .ok_or_else(|| "surface is not supported by adapter".to_string())?;
    // sRGB形式があればそれを使う。無ければ`get_default_config`が選んだ形式のまま。
    if let Some(srgb) = surface_caps.formats.iter().copied().find(|f| f.is_srgb()) {
        config.format = srgb;
    }
    config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
    config.present_mode = wgpu::PresentMode::Fifo;
    surface.configure(&device, &config);
    Ok((surface, device, queue, config))
}

impl TerrainRenderer {
    pub async fn new(canvas: web_sys::HtmlCanvasElement) -> Result<Self, String> {
        let (surface, device, queue, config) = init_surface(canvas).await?;
        let (width, height) = (config.width, config.height);

        let shader = terrain_shader(&device);
        let targets = RenderTargets::new(&device, config.format, width, height);
        let downsample = Downsample::new(
            &device,
            &shader,
            config.format,
            &targets.supersample_color_view,
        );

        // 頂点(view_proj・陰影)と、水域レイヤーのフラグメント(視線・楕円体)が読む。
        let camera_bind_group_layout = uniform_layout(
            &device,
            "camera_bind_group_layout",
            wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        );
        let camera = UniformSlot::new(
            &device,
            &camera_bind_group_layout,
            "camera_uniform",
            std::mem::size_of::<CameraUniform>(),
        );

        let pipelines = Pipelines::new(&device, config.format, &shader, &camera_bind_group_layout);
        let fade_table = UniformSlot::new(
            &device,
            &pipelines.fade_bind_group_layout,
            "fade_table",
            MAX_FADE_ENTRIES * std::mem::size_of::<[f32; 4]>(),
        );
        let draw_space = |label| {
            UniformSlot::new(
                &device,
                &pipelines.draw_bind_group_layout,
                label,
                std::mem::size_of::<DrawUniform>(),
            )
        };
        let draw_world = draw_space("draw_world_uniform");
        let draw_view = draw_space("draw_view_uniform");
        let draw_screen = draw_space("draw_screen_uniform");
        let models = ModelBatch::new(&device, config.format, &pipelines.draw_bind_group_layout);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipelines,
            meshes: HashMap::new(),
            fades: Fades::new(),
            fade_table,
            camera,
            targets,
            downsample,
            markers: VertexBatch::new("marker_vertex_buffer"),
            coverage_2d: VertexBatch::new("coverage_2d_vertex_buffer"),
            tracks: VertexBatch::new("tracks_vertex_buffer"),
            models,
            dome_vertex_buffer: None,
            num_dome_vertices: 0,
            hillshade: false,
            ellipsoid: ([[0.0; 4]; 3], [0.0; 4]),
            draw_world,
            draw_view,
            draw_screen,
            world_opaque: VertexBatch::new("draw_world_opaque"),
            world_blend: VertexBatch::new("draw_world_blend"),
            view_opaque: VertexBatch::new("draw_view_opaque"),
            view_blend: VertexBatch::new("draw_view_blend"),
            screen_batch: VertexBatch::new("draw_screen"),
        })
    }

    /// 作図(`terrain::drawing`)の頂点列を更新する。作図の一覧・原点・地形のLOD・canvasの大きさが
    /// 変わるたびに`drawing_geometry::build`で作り直して呼ぶ。
    pub fn update_drawings(&mut self, batches: &DrawingBatches) {
        let (device, queue) = (&self.device, &self.queue);
        self.world_opaque.set(device, queue, &batches.world.opaque);
        self.world_blend.set(device, queue, &batches.world.blend);
        self.view_opaque.set(device, queue, &batches.view.opaque);
        self.view_blend.set(device, queue, &batches.view.blend);
        self.screen_batch.set(device, queue, &batches.screen);
    }

    /// canvasの内部解像度(ピクセル)。作図の画面座標(`Position::Screen`)の角の位置を決めるのに使う。
    pub fn canvas_size_px(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// レーダー観測点のマーカー(画面サイズ固定のピン)の頂点データを更新する。原点変更・マーカー追加/
    /// 削除/選択変更のたびに呼び直す想定(`terrain/markers.rs`が頂点データを作る)。
    pub fn update_markers(&mut self, vertices: &[DrawVertex]) {
        self.markers.set(&self.device, &self.queue, vertices);
    }

    /// 2D地図モードの覆域(塗り+輪郭線)の頂点データを更新する。深度テストなしで描く(`terrain/markers.rs`)。
    pub fn update_coverage_2d(&mut self, vertices: &[DrawVertex]) {
        self.coverage_2d.set(&self.device, &self.queue, vertices);
    }

    /// 航跡(トラック)の頂点データ(シンボル・航跡・高度線)を更新する。トラックの受信・原点変更・地形のLOD切り替え・
    /// 2D/3D切り替えのたびに`terrain::tracks::build_track_geometry`で作り直して呼ぶ。
    pub fn update_tracks(&mut self, vertices: &[DrawVertex]) {
        self.tracks.set(&self.device, &self.queue, vertices);
    }

    /// 3Dモデル(`terrain::models`)を1つ登録する(`key`はモデルの識別子。アプリが登録したURL)。同じキーがあれば置き換える。
    pub fn set_model(&mut self, key: &str, mesh: &ModelMesh) {
        self.models.set_model(&self.device, key, mesh);
    }

    /// 3Dモデルの登録を外す(GPUバッファは解放される)。
    pub fn remove_model(&mut self, key: &str) {
        self.models.remove_model(key);
    }

    /// このフレームに描く3Dモデルのインスタンス(キーごとの、1機ごとの変換行列と色)を差し替える。
    /// `instances`に無いモデルは何も描かない。カメラ・トラックが変わるたびに呼ぶ(毎フレームでよい)。
    pub fn update_model_instances(&mut self, instances: &HashMap<String, Vec<ModelInstance>>) {
        self.models
            .set_instances(&self.device, &self.queue, instances);
    }

    /// 選択中マーカーの覆域ドーム(半球状の面、TriangleList)の頂点データを更新する。
    /// `update_markers`と同じタイミングで呼び直す想定。
    pub fn update_dome(&mut self, vertices: &[TerrainVertex]) {
        if vertices.is_empty() {
            self.dome_vertex_buffer = None;
            self.num_dome_vertices = 0;
            return;
        }
        self.dome_vertex_buffer = Some(self.device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("dome_vertex_buffer"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            },
        ));
        self.num_dome_vertices = vertices.len() as u32;
    }

    /// メッシュ1個のGPUバッファを作る。三角形が1つも無い(全部海)なら`None`。
    fn upload_mesh(&self, mesh: &TerrainMesh) -> Option<MeshGpu> {
        if mesh.indices.is_empty() {
            return None;
        }
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("terrain_mesh_vertex_buffer"),
                contents: bytemuck::cast_slice(&mesh.vertices),
                // COPY_DST: 原点変更時にupdate_mesh_vertices()で頂点データを書き換えられるようにする。
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("terrain_mesh_index_buffer"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        Some(MeshGpu {
            vertex_buffer,
            index_buffer,
            num_indices: mesh.indices.len() as u32,
            bounds: position_bounds(&mesh.vertices),
        })
    }

    /// メッシュ1個を登録する。同じキーが既にあれば(解像度レベルの切り替え)、すぐに置き換える。
    pub fn set_mesh(&mut self, key: MeshKey, mesh: &TerrainMesh) {
        self.fades.cancel(key);
        match self.upload_mesh(mesh) {
            Some(gpu) => {
                self.meshes.insert(key, gpu);
            }
            None => {
                self.meshes.remove(&key);
            }
        }
    }

    /// `set_mesh`と同じだが、古いメッシュを`FADE_DURATION_MS`だけ残し、新旧を混ぜながら入れ替える
    /// (クロスフェード)。同じキーが無ければ(タイル全体→チャンクの切り替えなど、別のキーが同時に
    /// `remove_mesh_faded`されるとき)、新しいメッシュだけが混ざりながら出てくる。
    /// 描画のたびに時間が進むので、`is_fading`の間は描き直し続けること。
    pub fn set_mesh_faded(&mut self, key: MeshKey, mesh: &TerrainMesh) {
        if !self.fades.has_room() {
            self.set_mesh(key, mesh);
            return;
        }
        let now = now_ms();
        let old = self.meshes.remove(&key);
        match self.upload_mesh(mesh) {
            Some(gpu) => {
                self.meshes.insert(key, gpu);
                self.fades.replace(key, old, now);
            }
            None => {
                // 新しいメッシュに三角形が無い(全部海)。古いメッシュだけが消えていく。
                self.fades.cancel(key);
                if let Some(old) = old {
                    self.fades.remove(key, old, now);
                }
            }
        }
    }

    /// メッシュ1個を取り除く(GPUバッファは解放される)。
    fn remove_mesh(&mut self, key: MeshKey) {
        self.fades.cancel(key);
        self.meshes.remove(&key);
    }

    /// `remove_mesh`と同じだが、`FADE_DURATION_MS`かけて混ざりながら消す(`set_mesh_faded`と対で使う)。
    pub fn remove_mesh_faded(&mut self, key: MeshKey) {
        if !self.fades.has_room() {
            self.remove_mesh(key);
            return;
        }
        if let Some(old) = self.meshes.remove(&key) {
            self.fades.remove(key, old, now_ms());
        } else {
            self.fades.cancel(key);
        }
    }

    /// クロスフェード中か。中は時間が進むので、描き直し続ける(でないと途中の状態で止まる)。
    pub fn is_fading(&self) -> bool {
        self.fades.is_active()
    }

    /// 原点変更時など、頂点数は変わらないまま座標(位置)だけを更新したいときに使う。
    /// DETAILED_DESIGN.md 3.3節: 原点を変更したら頂点バッファを再計算・再アップロードする
    /// (タイルデータの再フェッチは不要)。登録されていないキーは何もしない。
    pub fn update_mesh_vertices(&mut self, key: MeshKey, vertices: &[TerrainVertex]) {
        // 頂点位置が変わるので、クロスフェード中の古い側(古い原点のまま)は続けずに終わらせる。
        self.fades.cancel(key);
        if let Some(mesh) = self.meshes.get_mut(&key) {
            self.queue
                .write_buffer(&mesh.vertex_buffer, 0, bytemuck::cast_slice(vertices));
            mesh.bounds = position_bounds(vertices);
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        // 大きさが変わっていなければ何もしない(ResizeObserverやタブの再表示で同じ大きさが何度も
        // 通知されるが、そのたびにsurfaceの再設定と大きなテクスチャ3枚の作り直しをするのは無駄)。
        if width == 0 || height == 0 || (width == self.config.width && height == self.config.height)
        {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.targets = RenderTargets::new(&self.device, self.config.format, width, height);
        // supersample_color_viewを作り直したので、それを参照しているbind groupも作り直す。
        self.downsample
            .rebind(&self.device, &self.targets.supersample_color_view);
    }

    pub fn aspect_ratio(&self) -> f32 {
        self.config.width as f32 / self.config.height.max(1) as f32
    }

    /// 陰影(ヒルシェード)を付けるかを切り替える。次の`render`から反映される(メッシュの作り直しは不要)。
    pub fn set_hillshade(&mut self, enabled: bool) {
        self.hillshade = enabled;
    }

    /// 水域レイヤー(WGS84楕円体の海抜0mの面)の基準を、地形メッシュの現在の原点に合わせる。
    /// 頂点位置を新しい原点のENU座標で作り直すとき(初期化・原点変更)に、同時に呼ぶこと。
    pub fn set_ellipsoid_origin(&mut self, transform: &super::geodesy::EnuTransform) {
        self.ellipsoid = transform.ellipsoid_shader_params();
        // 頂点位置を作り直す(古い原点のメッシュは使えなくなる)ので、クロスフェード中のものは終わらせる。
        self.fades.clear();
    }

    pub fn render(&mut self, camera: &Camera) -> Result<(), String> {
        let now = now_ms();
        self.fades.finish(now);
        let [eye, forward, right, up] = camera.water_ray_basis();
        let (ellipsoid_m, ellipsoid_g) = self.ellipsoid;
        let camera_uniform = CameraUniform {
            view_proj: camera.view_proj_matrix().to_cols_array_2d(),
            shading: [if self.hillshade { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            eye,
            forward,
            right,
            up,
            ellipsoid_m,
            ellipsoid_g,
        };
        self.camera
            .write(&self.queue, bytemuck::bytes_of(&camera_uniform));

        // 作図のuniform。絶対座標=地形と同じカメラ、視点空間=射影だけ(カメラから見た座標をそのまま射影)、
        // 画面=ピクセル座標。
        let (width, height) = (self.config.width as f32, self.config.height as f32);
        let viewport = [width, height, 0.0, 0.0];
        for (space, view_proj, light) in [
            (
                &self.draw_world,
                camera.view_proj_matrix(),
                WORLD_DRAW_LIGHT,
            ),
            (&self.draw_view, camera.projection_matrix(), VIEW_DRAW_LIGHT),
            (&self.draw_screen, screen_matrix(width, height), [0.0; 4]),
        ] {
            let uniform = DrawUniform {
                view_proj: view_proj.to_cols_array_2d(),
                viewport,
                light,
            };
            space.write(&self.queue, bytemuck::bytes_of(&uniform));
        }
        // カメラ固定の作図(視点空間・画面)は、地形の奥行きとは別に、地形の手前へ重ねて描く。
        // 深度バッファを作り直す必要があるので、地形のパスとは別のパスにする。
        let has_overlay = !(self.view_opaque.is_empty()
            && self.view_blend.is_empty()
            && self.screen_batch.is_empty());

        let (frame, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => (t, false),
            // 描画はできるが、surfaceの設定が現状に合っていない。描いたあとに設定し直す。
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => (t, true),
            // 一時的に表示できない状態(タブやウインドウが隠れている等)。このフレームは描かず、
            // 次のフレームでやり直す(エラーではない)。
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(())
            }
            // 設定が古くなった。設定し直して、次のフレームで復帰する。
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            other => return Err(format!("get_current_texture failed: {other:?}")),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("terrain_encoder"),
            });

        let fade_draws = self.fade_draws(camera, now);
        let entries: Vec<[f32; 4]> = fade_draws.iter().map(|d| d.entry).collect();
        if !entries.is_empty() {
            self.fade_table
                .write(&self.queue, bytemuck::cast_slice(&entries));
        }
        self.encode_main_pass(&mut encoder, camera, has_overlay, &fade_draws);
        if has_overlay {
            self.encode_overlay_pass(&mut encoder);
        }
        self.encode_downsample_pass(&mut encoder, &view);

        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(frame);
        if suboptimal {
            self.surface.configure(&self.device, &self.config);
        }
        Ok(())
    }

    /// このフレームでクロスフェードするメッシュ(視錐台に入るもの)と、割合の表の値。並びが表の番号になる。
    fn fade_draws(&self, camera: &Camera, now_ms: f64) -> Vec<FadeDraw<'_>> {
        let mut draws = Vec::new();
        if !self.fades.is_active() {
            return draws;
        }
        let view_proj = camera.view_proj_matrix();
        if self.fades.has_incoming() {
            for (key, mesh) in &self.meshes {
                let Some(progress) = self.fades.incoming_progress(key, now_ms) else {
                    continue;
                };
                if !is_outside_frustum(&view_proj, mesh.bounds) {
                    // x=新しい側が出る割合、y=0(反転なし)。
                    draws.push(FadeDraw {
                        mesh,
                        entry: [progress, 0.0, 0.0, 0.0],
                    });
                }
            }
        }
        for outgoing in self.fades.outgoing() {
            if !is_outside_frustum(&view_proj, outgoing.mesh.bounds) {
                // 古い側は、新しい側が出ない残りの画素(y=1で反転)。
                let progress = fade_progress(outgoing.start_ms, now_ms);
                draws.push(FadeDraw {
                    mesh: &outgoing.mesh,
                    entry: [progress, 1.0, 0.0, 0.0],
                });
            }
        }
        // 割合の表に収まる分だけ(`MAX_FADE_ENTRIES`。超える分は`has_room`で始めさせないが、念のため)。
        draws.truncate(MAX_FADE_ENTRIES);
        draws
    }

    /// メインのパス: 水域→地形→絶対座標の作図・ドーム・マーカー・航跡。MSAAカラーへ描き、
    /// (カメラ固定の作図が無ければ)そのままスーパーサンプリングの解決先へ解決する。
    fn encode_main_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        camera: &Camera,
        has_overlay: bool,
        fade_draws: &[FadeDraw],
    ) {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("terrain_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.targets.msaa_view,
                // スワップチェーンへ直接ではなく、スーパーサンプリングの内部解像度テクスチャへ
                // 解決する(この後のdownsampleパスで実際のcanvas解像度へ縮小する)。
                // カメラ固定の作図があるときは、続くオーバーレイのパスが解決する。
                resolve_target: if has_overlay {
                    None
                } else {
                    Some(&self.targets.supersample_color_view)
                },
                depth_slice: None,
                ops: wgpu::Operations {
                    // 地形が描かれない領域(空・海・データ範囲の外側)は黒。
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    // resolve_targetへ解決した後はこのMSAAテクスチャ自体は不要なので
                    // 保存しない(Discard)。オーバーレイのパスへ引き継ぐときだけ保存する。
                    store: if has_overlay {
                        wgpu::StoreOp::Store
                    } else {
                        wgpu::StoreOp::Discard
                    },
                },
            })],
            depth_stencil_attachment: Some(self.cleared_depth()),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        // 水域レイヤーを最初に描く(楕円体との交点の深度を書く。続く地形メッシュは深度テストで、
        // 地球本体の向こう側は隠れ、手前(と余裕の範囲)は水域の上に描かれる)。
        render_pass.set_pipeline(&self.pipelines.water);
        render_pass.set_bind_group(0, self.camera.bind_group(), &[]);
        render_pass.draw(0..3, 0..1);

        render_pass.set_pipeline(&self.pipelines.terrain);
        render_pass.set_bind_group(0, self.camera.bind_group(), &[]);
        // 視錐台の外のメッシュは描かない(全タイルをチャンクで常駐させているので、画面外の
        // 大量の頂点を毎フレーム処理しないため)。
        let view_proj = camera.view_proj_matrix();
        let any_incoming = self.fades.has_incoming();
        for (key, mesh) in &self.meshes {
            if is_outside_frustum(&view_proj, mesh.bounds)
                // 出てくる途中のメッシュは、次のクロスフェード用の描画でまとめて描く。
                || (any_incoming && self.fades.is_incoming(key))
            {
                continue;
            }
            mesh.draw(&mut render_pass, 0..1);
        }
        // クロスフェード中のメッシュ(新旧)。割合の表の何番目かを`first_instance`で渡す(`terrain.wgsl`の`vs_fade`)。
        if !fade_draws.is_empty() {
            render_pass.set_pipeline(&self.pipelines.terrain_fade);
            render_pass.set_bind_group(0, self.camera.bind_group(), &[]);
            render_pass.set_bind_group(1, self.fade_table.bind_group(), &[]);
            for (index, draw) in fade_draws.iter().enumerate() {
                let index = index as u32;
                draw.mesh.draw(&mut render_pass, index..index + 1);
            }
        }

        // 絶対座標の作図。不透明なものは覆域ドームより先に(深度を書く)、半透明なものはドームの後に描く。
        self.world_opaque.draw(
            &mut render_pass,
            &self.pipelines.draw_opaque,
            &self.draw_world,
        );
        // 3Dモデル(不透明。深度を書く)。
        self.models.draw(&mut render_pass, &self.draw_world);

        // 覆域ドーム(半透明)は他の不透明な描画がすべて終わった後に描く。
        // `update_dome`は空の頂点列ならバッファを持たない。
        if let Some(dome_buffer) = self.dome_vertex_buffer.as_ref() {
            render_pass.set_pipeline(&self.pipelines.dome);
            render_pass.set_bind_group(0, self.camera.bind_group(), &[]);
            render_pass.set_vertex_buffer(0, dome_buffer.slice(..));
            render_pass.draw(0..self.num_dome_vertices, 0..1);
        }

        self.world_blend.draw(
            &mut render_pass,
            &self.pipelines.draw_blend,
            &self.draw_world,
        );

        // 2Dの覆域(塗り+輪郭線)は深度テストなしで、地形の上に重ねる。続いて観測点のマーカー(ピン)。
        self.coverage_2d.draw(
            &mut render_pass,
            &self.pipelines.draw_screen,
            &self.draw_world,
        );
        self.markers.draw(
            &mut render_pass,
            &self.pipelines.draw_blend,
            &self.draw_world,
        );
        self.tracks.draw(
            &mut render_pass,
            &self.pipelines.draw_blend,
            &self.draw_world,
        );
    }

    /// カメラ固定の作図のパス: 地形を描いた色をそのまま(Load)引き継ぎ、深度だけ作り直して(地形とは
    /// 隠し合わない)、視点空間の図形(図形どうしは奥行きで隠し合う)→画面座標の図形(追加順)の順に描く。
    fn encode_overlay_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut overlay_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("drawing_overlay_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.targets.msaa_view,
                resolve_target: Some(&self.targets.supersample_color_view),
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Discard,
                },
            })],
            depth_stencil_attachment: Some(self.cleared_depth()),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.view_opaque.draw(
            &mut overlay_pass,
            &self.pipelines.draw_opaque,
            &self.draw_view,
        );
        self.view_blend.draw(
            &mut overlay_pass,
            &self.pipelines.draw_blend,
            &self.draw_view,
        );
        self.screen_batch.draw(
            &mut overlay_pass,
            &self.pipelines.draw_screen,
            &self.draw_screen,
        );
    }

    /// 深度バッファを0.0で消して使い、パスの後には残さない深度アタッチメント
    /// (反転Z(reversed-Z): 「最も遠い」を表す深度値は0.0。camera.rs参照)。
    fn cleared_depth(&self) -> wgpu::RenderPassDepthStencilAttachment<'_> {
        wgpu::RenderPassDepthStencilAttachment {
            view: &self.targets.depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0.0),
                store: wgpu::StoreOp::Discard,
            }),
            stencil_ops: None,
        }
    }

    /// スーパーサンプリングのダウンサンプルパス: 内部解像度で描いたsupersample_color_viewを、実際の
    /// canvas解像度のスワップチェーンへ線形フィルタで縮小して描く(画面いっぱいの三角形1枚、頂点バッファなし)。
    fn encode_downsample_pass(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let mut downsample_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("downsample_render_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.downsample.draw(&mut downsample_pass);
    }
}

#[cfg(test)]
mod tests {
    use super::targets::{supersample_size, SUPERSAMPLE_MAX_DIMENSION};
    use super::uniforms::{DRAW_VERTEX_ATTRIBUTES, TERRAIN_VERTEX_ATTRIBUTES};
    use super::*;
    use crate::terrain::camera::{CameraPreset, OrbitCamera, ViewMode};
    use crate::terrain::models::types::ModelVertex;
    use glam::{Mat4, Vec3, Vec4};
    use std::mem::{offset_of, size_of};

    const TERRAIN_WGSL: &str = include_str!("../terrain.wgsl");
    const DRAW_WGSL: &str = include_str!("../draw.wgsl");
    const MODEL_WGSL: &str = include_str!("../models/model.wgsl");

    // ---- 純関数 ----

    #[test]
    fn supersample_size_doubles_and_clamps_each_side() {
        assert_eq!(supersample_size(700, 500), (1400, 1000));
        assert_eq!(supersample_size(0, 0), (2, 2)); // 0は1として扱う
                                                    // 幅と高さを独立に上限(4096)へ丸める。
        assert_eq!(
            supersample_size(3000, 100),
            (SUPERSAMPLE_MAX_DIMENSION, 200)
        );
    }

    #[test]
    fn position_bounds_encloses_all_vertices() {
        let v = |p: [f32; 3]| TerrainVertex::unlit(p, [0.0; 3]);
        let (min, max) =
            position_bounds(&[v([1.0, -2.0, 3.0]), v([-4.0, 5.0, 0.5]), v([0.0, 0.0, 9.0])]);
        assert_eq!(min, [-4.0, -2.0, 0.5]);
        assert_eq!(max, [1.0, 5.0, 9.0]);
        // 頂点が無ければ空の直方体(どの点も含まない)。
        let (min, max) = position_bounds(&[]);
        assert!(min[0] > max[0]);
    }

    #[test]
    fn screen_matrix_maps_pixels_to_clip_space_with_y_down() {
        let m = screen_matrix(800.0, 600.0);
        let map = |x: f32, y: f32| m * Vec4::new(x, y, 0.0, 1.0);
        let close = |a: Vec4, b: [f32; 3]| {
            (a.x - b[0]).abs() < 1e-6 && (a.y - b[1]).abs() < 1e-6 && a.w == 1.0
        };
        assert!(close(map(0.0, 0.0), [-1.0, 1.0, 0.5])); // 左上
        assert!(close(map(800.0, 600.0), [1.0, -1.0, 0.5])); // 右下
        assert!(close(map(400.0, 300.0), [0.0, 0.0, 0.5])); // 中心
        assert_eq!(map(10.0, 10.0).z, 0.5); // 深度は一定
    }

    fn view_proj(mode: ViewMode, distance: f32) -> Mat4 {
        let mut o = OrbitCamera::preset(CameraPreset::Overview, 0.0);
        o.mode = mode;
        o.distance = distance;
        o.to_camera(1.5).view_proj_matrix()
    }

    fn cube(center: Vec3, half: f32) -> ([f32; 3], [f32; 3]) {
        (
            (center - Vec3::splat(half)).to_array(),
            (center + Vec3::splat(half)).to_array(),
        )
    }

    #[test]
    fn frustum_test_culls_boxes_that_are_clearly_outside() {
        let vp = view_proj(ViewMode::ThreeD, 10_000.0);
        // 注視点まわりの直方体は見える。
        assert!(!is_outside_frustum(&vp, cube(Vec3::ZERO, 100.0)));
        // 視野の左右・下・カメラの後ろ・far面の外は見えない。
        assert!(is_outside_frustum(
            &vp,
            cube(Vec3::new(500_000.0, 0.0, 0.0), 100.0)
        ));
        assert!(is_outside_frustum(
            &vp,
            cube(Vec3::new(0.0, 500_000.0, 0.0), 100.0)
        ));
        assert!(is_outside_frustum(
            &vp,
            cube(Vec3::new(-8_000.0, -8_000.0, 6_000.0), 10.0)
        )); // 視点の後ろ
        assert!(is_outside_frustum(
            &vp,
            cube(Vec3::new(-20_000_000.0, -20_000_000.0, 0.0), 100.0)
        ));
        // 視錐台をまたぐ大きな直方体は外ではない。
        assert!(!is_outside_frustum(&vp, cube(Vec3::ZERO, 5_000_000.0)));
    }

    // 保守的であること: 視野の中に見えている点を含む直方体を「外」と判定してはいけない。
    #[test]
    fn frustum_test_never_culls_a_visible_box() {
        for mode in [ViewMode::ThreeD, ViewMode::TwoD] {
            let vp = view_proj(mode, 50_000.0);
            let mut checked = 0;
            for xi in -10..=10 {
                for yi in -10..=10 {
                    for zi in [-2000.0, 0.0, 3000.0] {
                        let center = Vec3::new(xi as f32 * 8_000.0, yi as f32 * 8_000.0, zi);
                        let clip = vp * center.extend(1.0);
                        let inside = clip.w > 0.0
                            && clip.x.abs() <= clip.w
                            && clip.y.abs() <= clip.w
                            && clip.z >= 0.0
                            && clip.z <= clip.w;
                        if inside {
                            checked += 1;
                            assert!(
                                !is_outside_frustum(&vp, cube(center, 50.0)),
                                "{mode:?} {center}"
                            );
                        }
                    }
                }
            }
            assert!(
                checked > 20,
                "{mode:?}: only {checked} visible sample points"
            );
        }
    }

    // ---- WGSLの検証と、Rust側とのレイアウトの一致 ----

    fn parse(src: &str) -> naga::Module {
        naga::front::wgsl::parse_str(src)
            .unwrap_or_else(|e| panic!("WGSL parse error: {}", e.emit_to_string(src)))
    }

    fn struct_of<'m>(module: &'m naga::Module, name: &str) -> (u32, &'m [naga::StructMember]) {
        module
            .types
            .iter()
            .find_map(|(_, ty)| match (&ty.name, &ty.inner) {
                (Some(n), naga::TypeInner::Struct { members, span }) if n == name => {
                    Some((*span, members.as_slice()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("struct {name} not found in WGSL"))
    }

    fn offsets(members: &[naga::StructMember]) -> Vec<(String, u32)> {
        members
            .iter()
            .map(|m| (m.name.clone().unwrap_or_default(), m.offset))
            .collect()
    }

    #[test]
    fn wgsl_modules_parse_and_validate() {
        for (name, src) in [
            ("terrain.wgsl", TERRAIN_WGSL),
            ("draw.wgsl", DRAW_WGSL),
            ("model.wgsl", MODEL_WGSL),
        ] {
            let module = parse(src);
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
        }
    }

    #[test]
    fn camera_uniform_matches_the_wgsl_struct() {
        let module = parse(TERRAIN_WGSL);
        let (span, members) = struct_of(&module, "CameraUniform");
        assert_eq!(span as usize, size_of::<CameraUniform>());
        let expected = [
            ("view_proj", offset_of!(CameraUniform, view_proj)),
            ("shading", offset_of!(CameraUniform, shading)),
            ("eye", offset_of!(CameraUniform, eye)),
            ("forward", offset_of!(CameraUniform, forward)),
            ("right", offset_of!(CameraUniform, right)),
            ("up", offset_of!(CameraUniform, up)),
            ("ellipsoid_m", offset_of!(CameraUniform, ellipsoid_m)),
            ("ellipsoid_g", offset_of!(CameraUniform, ellipsoid_g)),
        ];
        let want: Vec<(String, u32)> = expected
            .iter()
            .map(|(n, o)| (n.to_string(), *o as u32))
            .collect();
        assert_eq!(offsets(members), want);
    }

    #[test]
    fn draw_uniform_matches_the_wgsl_struct() {
        // 3Dモデルのシェーダーも、作図と同じuniform(`DrawUniform`)を読む。
        for src in [DRAW_WGSL, MODEL_WGSL] {
            let module = parse(src);
            let (span, members) = struct_of(&module, "DrawUniform");
            assert_eq!(span as usize, size_of::<DrawUniform>());
            let want: Vec<(String, u32)> = [
                ("view_proj", offset_of!(DrawUniform, view_proj)),
                ("viewport", offset_of!(DrawUniform, viewport)),
                ("light", offset_of!(DrawUniform, light)),
            ]
            .iter()
            .map(|(n, o)| (n.to_string(), *o as u32))
            .collect();
            assert_eq!(offsets(members), want);
        }
    }

    #[test]
    fn model_vertex_and_instance_layouts_match_structs_and_wgsl() {
        use super::model_batch::{MODEL_INSTANCE_ATTRIBUTES, MODEL_VERTEX_ATTRIBUTES};
        // 頂点バッファ(頂点・インスタンス)それぞれで、属性が隙間なく構造体を覆い、オフセットが構造体と一致する。
        let vertex_offsets: Vec<u64> = MODEL_VERTEX_ATTRIBUTES.iter().map(|a| a.offset).collect();
        assert_eq!(
            vertex_offsets,
            [
                offset_of!(ModelVertex, position) as u64,
                offset_of!(ModelVertex, normal) as u64,
                offset_of!(ModelVertex, color) as u64
            ]
        );
        let last = MODEL_VERTEX_ATTRIBUTES.last().unwrap();
        assert_eq!(
            last.offset + last.format.size(),
            size_of::<ModelVertex>() as u64
        );
        let instance_offsets: Vec<u64> =
            MODEL_INSTANCE_ATTRIBUTES.iter().map(|a| a.offset).collect();
        assert_eq!(
            instance_offsets,
            [0, 16, 32, 48, offset_of!(ModelInstance, tint) as u64]
        );
        let last = MODEL_INSTANCE_ATTRIBUTES.last().unwrap();
        assert_eq!(
            last.offset + last.format.size(),
            size_of::<ModelInstance>() as u64
        );
        // シェーダーの入力(location 0〜7)と、2本のバッファの属性が対応する。
        let all: Vec<wgpu::VertexAttribute> = MODEL_VERTEX_ATTRIBUTES
            .iter()
            .chain(MODEL_INSTANCE_ATTRIBUTES.iter())
            .copied()
            .collect();
        check_vertex_inputs(MODEL_WGSL, &all);
    }

    /// `vs_main`の頂点入力(location・型)を、Rust側の頂点属性と突き合わせる。
    fn check_vertex_inputs(src: &str, attributes: &[wgpu::VertexAttribute]) {
        let module = parse(src);
        let entry = module
            .entry_points
            .iter()
            .find(|e| e.name == "vs_main")
            .expect("vs_main");
        let input = module.types[entry.function.arguments[0].ty].clone();
        let naga::TypeInner::Struct { members, .. } = input.inner else {
            panic!("vs_main takes a struct")
        };

        assert_eq!(members.len(), attributes.len());
        for member in &members {
            let Some(naga::Binding::Location { location, .. }) = member.binding else {
                panic!("{:?} has no @location", member.name)
            };
            let attribute = attributes
                .iter()
                .find(|a| a.shader_location == location)
                .unwrap_or_else(|| panic!("no vertex attribute for @location({location})"));
            // 浮動小数のベクトルとして読む形式(Float32xN・Snorm16x2)と、WGSLのvecN<f32>が同じ次元。
            let components = match attribute.format {
                wgpu::VertexFormat::Float32x2 | wgpu::VertexFormat::Snorm16x2 => {
                    naga::VectorSize::Bi
                }
                wgpu::VertexFormat::Float32x3 => naga::VectorSize::Tri,
                wgpu::VertexFormat::Float32x4 => naga::VectorSize::Quad,
                other => panic!("unexpected vertex format {other:?}"),
            };
            match &module.types[member.ty].inner {
                naga::TypeInner::Vector { size, scalar } => {
                    assert_eq!(*size, components, "@location({location})");
                    assert_eq!(
                        scalar.kind,
                        naga::ScalarKind::Float,
                        "@location({location})"
                    );
                }
                other => panic!("@location({location}) is {other:?}"),
            }
        }
    }

    #[test]
    fn terrain_vertex_layout_matches_struct_and_wgsl() {
        let offsets: Vec<u64> = TERRAIN_VERTEX_ATTRIBUTES.iter().map(|a| a.offset).collect();
        assert_eq!(
            offsets,
            [
                offset_of!(TerrainVertex, position) as u64,
                offset_of!(TerrainVertex, color) as u64,
                offset_of!(TerrainVertex, normal_xy) as u64
            ]
        );
        let last = TERRAIN_VERTEX_ATTRIBUTES.last().unwrap();
        assert_eq!(
            last.offset + last.format.size(),
            size_of::<TerrainVertex>() as u64
        );
        check_vertex_inputs(TERRAIN_WGSL, &TERRAIN_VERTEX_ATTRIBUTES);
    }

    #[test]
    fn draw_vertex_layout_matches_struct_and_wgsl() {
        let offsets: Vec<u64> = DRAW_VERTEX_ATTRIBUTES.iter().map(|a| a.offset).collect();
        assert_eq!(
            offsets,
            [
                offset_of!(DrawVertex, position) as u64,
                offset_of!(DrawVertex, color) as u64,
                offset_of!(DrawVertex, aux) as u64,
                offset_of!(DrawVertex, params) as u64
            ]
        );
        let last = DRAW_VERTEX_ATTRIBUTES.last().unwrap();
        assert_eq!(
            last.offset + last.format.size(),
            size_of::<DrawVertex>() as u64
        );
        check_vertex_inputs(DRAW_WGSL, &DRAW_VERTEX_ATTRIBUTES);
    }
}
