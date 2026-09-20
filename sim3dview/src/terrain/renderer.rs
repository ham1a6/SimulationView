//! wgpuによる地形メッシュの描画。DETAILED_DESIGN.md 6節。
//! フェーズ8時点では単一メッシュ・単一カメラでの描画確認が目的
//! (自由視点カメラはフェーズ10、側面図パネルへの適用もフェーズ10でカメラ機構と合わせて行う)。

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use std::cell::Cell;
use std::collections::HashMap;

use super::camera::Camera;
use super::drawing_geometry::{DrawVertex, DrawingBatches};
use super::loader::MeshKey;
use super::mesh::{TerrainMesh, TerrainVertex};

/// 作図(`terrain::drawing`)用のuniform(`draw.wgsl`の`DrawUniform`)。座標の種類ごとに1つ持つ。
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DrawUniform {
    view_proj: [[f32; 4]; 4],
    /// x,y: 描画先(canvas)の大きさ(px)。
    viewport: [f32; 4],
    /// xyz: 光源の向き(面から光源へ向かう単位ベクトル)。
    light: [f32; 4],
}

/// 絶対座標の作図の光源。地形の陰影(`terrain.wgsl`の`LIGHT_DIR`)と同じ、北西・仰角45度(ENU座標)。
const WORLD_DRAW_LIGHT: [f32; 4] = [-0.5, 0.5, 0.707_106_8, 0.0];
/// カメラ固定(視点空間)の作図の光源。カメラから見て左上手前から当てる(x右・y上・z手前)。
const VIEW_DRAW_LIGHT: [f32; 4] = [-0.348, 0.497, 0.795, 0.0];

/// 作図の座標の種類1つ分のuniformとbind group。
struct DrawSpace {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl DrawSpace {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, label: &str) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: std::mem::size_of::<DrawUniform>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        });
        Self { buffer, bind_group }
    }
}

/// 作図の頂点バッファ1本(TriangleList)。空ならバッファを持たない。
struct VertexBatch {
    buffer: Option<wgpu::Buffer>,
    count: u32,
}

impl VertexBatch {
    fn empty() -> Self {
        Self { buffer: None, count: 0 }
    }

    fn set(&mut self, device: &wgpu::Device, label: &str, vertices: &[DrawVertex]) {
        if vertices.is_empty() {
            *self = Self::empty();
            return;
        }
        self.buffer = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }));
        self.count = vertices.len() as u32;
    }

    fn is_empty(&self) -> bool {
        self.buffer.is_none()
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline, space: &DrawSpace) {
        let Some(buffer) = self.buffer.as_ref() else {
            return;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &space.bind_group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..self.count, 0..1);
    }
}

/// 画面のピクセル座標(左上原点、x右・y下)をクリップ空間へ写す行列(作図のカメラ固定・画面座標用)。
/// 深度は一定(0.5)。深度テストは使わず、描く順で重ねる。
fn screen_matrix(width: f32, height: f32) -> glam::Mat4 {
    glam::Mat4::from_cols(
        glam::Vec4::new(2.0 / width, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, -2.0 / height, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, 0.0, 0.0),
        glam::Vec4::new(-1.0, 1.0, 0.5, 1.0),
    )
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    /// x: 陰影(ヒルシェード)を付けるなら1、付けないなら0(`terrain.wgsl`の`camera.shading`)。
    shading: [f32; 4],
    /// 水域レイヤーの視線の計算用(`Camera::water_ray_basis`): 視点(w=透視なら1)・視線方向・右・上。
    eye: [f32; 4],
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    /// WGS84楕円体(海抜0m)の陰関数の係数(`EnuTransform::ellipsoid_shader_params`)。
    ellipsoid_m: [[f32; 4]; 3],
    ellipsoid_g: [f32; 4],
}

/// マルチサンプルアンチエイリアシング(MSAA)のサンプル数。地形メッシュの解像度を
/// 高解像度化した後、遠景で多数の細かい三角形が1画素に収まりきらず
/// エイリアシング(市松状のちらつき/斑点)を起こすようになったため導入した。
/// 4はWebGPU実装で広くサポートされる標準的な値(8はハードウェアによって非対応。実機で
/// `createTexture({sampleCount: 8, ...})`がエラーになることを確認済み)。
const SAMPLE_COUNT: u32 = 4;

/// スーパーサンプリングの倍率。4倍MSAAだけでは、やや引いた視点・浅い角度で地形の
/// エイリアシング(「背景と同じ色の点が多数表示される/ズーム操作やカメラ操作時に画面が
/// ちかちかする」)を抑えきれなかったため追加した。canvasの`SUPERSAMPLE_FACTOR`倍の
/// 内部解像度で描画(+4倍MSAA)した後、線形フィルタで実際のcanvas解像度へ縮小する
/// (`downsample_pipeline`)。
const SUPERSAMPLE_FACTOR: u32 = 2;
/// スーパーサンプリング後の内部テクスチャの一辺の上限(ピクセル)。WebGPUが保証する
/// 最小の`maxTextureDimension2D`は8192だが、非常に大きなcanvas(4K超のウインドウ等)で
/// 2倍すると際どくなるため、余裕を持って安全側に制限する。
const SUPERSAMPLE_MAX_DIMENSION: u32 = 4096;

/// canvasの実解像度からスーパーサンプリング用の内部解像度を求める。
fn supersample_size(width: u32, height: u32) -> (u32, u32) {
    let w = (width.max(1) * SUPERSAMPLE_FACTOR).min(SUPERSAMPLE_MAX_DIMENSION);
    let h = (height.max(1) * SUPERSAMPLE_FACTOR).min(SUPERSAMPLE_MAX_DIMENSION);
    (w, h)
}

/// 地形メッシュ1個分のGPUバッファ(タイル全体、またはチャンク1個)。メッシュごとに頂点・
/// インデックスを別々に持ち、解像度レベルの切り替え(`set_mesh`)を1個ずつ行えるようにしてある。
struct MeshGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    num_indices: u32,
    /// 頂点位置を囲む直方体(最小の角, 最大の角。ENU座標)。視錐台の外のメッシュを描かないために使う
    /// (`is_outside_frustum`)。原点変更で頂点位置が変わる(`update_mesh_vertices`)ので`Cell`。
    bounds: Cell<([f32; 3], [f32; 3])>,
}

/// 頂点位置を囲む直方体(最小の角, 最大の角)。
fn position_bounds(vertices: &[TerrainVertex]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for v in vertices {
        for axis in 0..3 {
            min[axis] = min[axis].min(v.position[axis]);
            max[axis] = max[axis].max(v.position[axis]);
        }
    }
    (min, max)
}

/// 直方体が視錐台(クリップ空間の-w<=x<=w, -w<=y<=w, 0<=z<=w)の完全に外にあるか。8つの角が
/// すべて同じ面の外側にあれば、直方体全体がその面の外にある(GPUのクリッピングでも何も描かれない
/// ので、この判定で描画を省いても見た目は変わらない)。判定は保守的で、外にあるのに「外でない」と
/// 判定することはあっても、見えているものを「外」とすることはない。
fn is_outside_frustum(view_proj: &glam::Mat4, bounds: ([f32; 3], [f32; 3])) -> bool {
    let (min, max) = bounds;
    // 6面それぞれについて「全部の角が外側」を表すビットを、角ごとのビットとの論理積で求める。
    let mut all_outside = 0b11_1111u8;
    for corner in 0..8 {
        let p = glam::Vec4::new(
            if corner & 1 == 0 { min[0] } else { max[0] },
            if corner & 2 == 0 { min[1] } else { max[1] },
            if corner & 4 == 0 { min[2] } else { max[2] },
            1.0,
        );
        let c = *view_proj * p;
        let mut outside = 0u8;
        outside |= (c.x < -c.w) as u8;
        outside |= ((c.x > c.w) as u8) << 1;
        outside |= ((c.y < -c.w) as u8) << 2;
        outside |= ((c.y > c.w) as u8) << 3;
        outside |= ((c.z < 0.0) as u8) << 4;
        outside |= ((c.z > c.w) as u8) << 5;
        all_outside &= outside;
        if all_outside == 0 {
            return false;
        }
    }
    all_outside != 0
}

pub struct TerrainRenderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    meshes: HashMap<MeshKey, MeshGpu>,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    depth_view: wgpu::TextureView,
    // MSAA用の中間カラーテクスチャ(スーパーサンプリングの内部解像度、SAMPLE_COUNT倍
    // マルチサンプル)。各パイプラインがこのテクスチャへ描画し、render()の最後に
    // supersample_color_viewへ解決(resolve)する。
    msaa_view: wgpu::TextureView,
    // MSAA解決先(スーパーサンプリングの内部解像度、シングルサンプル)。downsample_pipelineが
    // このテクスチャを読み、実際のcanvas解像度(スワップチェーン)へ線形フィルタで縮小する。
    supersample_color_view: wgpu::TextureView,
    downsample_pipeline: wgpu::RenderPipeline,
    downsample_bind_group_layout: wgpu::BindGroupLayout,
    downsample_sampler: wgpu::Sampler,
    downsample_bind_group: wgpu::BindGroup,
    // レーダー観測点マーカー(四角い枠)用(LineList)。地形本体とは別パイプラインだが、
    // 頂点レイアウト・カメラバインドグループは共用する(terrain.wgslのシェーダーは
    // 位置をview_projで変換して色をそのまま出すだけの汎用的な内容のため、線描画にもそのまま使える)。
    line_pipeline: wgpu::RenderPipeline,
    marker_vertex_buffer: Option<wgpu::Buffer>,
    num_marker_vertices: u32,
    // 見通し範囲の覆域ドーム(半球状の面、TriangleList)用。地形・マーカーの奥に透けて見える
    // よう、アルファブレンド有効・深度書き込み無効のパイプラインにしてある(fs_dome参照)。
    dome_pipeline: wgpu::RenderPipeline,
    dome_vertex_buffer: Option<wgpu::Buffer>,
    num_dome_vertices: u32,
    // 陰影(ヒルシェード)を付けるか(`set_hillshade`)。描画のたびにuniformへ書く。
    hillshade: bool,
    // 水域レイヤー(WGS84楕円体の海抜0mの面)。地形メッシュより先に、画面いっぱいの三角形1枚で描く
    // (`terrain.wgsl`の`fs_water`)。楕円体の係数は現在の原点(地形メッシュの原点)基準で、
    // 原点変更のたびに`set_ellipsoid_origin`で更新する(頂点バッファの更新と同じ場面のため`&self`で
    // 呼べるよう`Cell`)。
    water_pipeline: wgpu::RenderPipeline,
    ellipsoid: Cell<([[f32; 4]; 3], [f32; 4])>,
    // 作図(`terrain::drawing`)。面も太い線もTriangleListで、`draw.wgsl`が描く。不透明は深度を書き、
    // 半透明は深度を書かずにアルファブレンドする。画面座標だけ深度テストなし(描く順で重ねる)。
    draw_opaque_pipeline: wgpu::RenderPipeline,
    draw_blend_pipeline: wgpu::RenderPipeline,
    draw_screen_pipeline: wgpu::RenderPipeline,
    // 座標の種類ごとのuniform: 絶対座標(カメラのview_proj)・視点空間(射影のみ)・画面(ピクセル座標)。
    draw_world: DrawSpace,
    draw_view: DrawSpace,
    draw_screen: DrawSpace,
    world_opaque: VertexBatch,
    world_blend: VertexBatch,
    view_opaque: VertexBatch,
    view_blend: VertexBatch,
    screen_batch: VertexBatch,
}

/// 作図のパイプライン1本を作る。`depth_write`/`depth_compare`で不透明・半透明・画面座標を作り分ける。
fn create_draw_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    label: &str,
    depth_write: bool,
    depth_compare: wgpu::CompareFunction,
) -> wgpu::RenderPipeline {
    let vertex_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<DrawVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
            wgpu::VertexAttribute { offset: 12, shader_location: 1, format: wgpu::VertexFormat::Float32x4 },
            wgpu::VertexAttribute { offset: 28, shader_location: 2, format: wgpu::VertexFormat::Float32x3 },
            wgpu::VertexAttribute { offset: 40, shader_location: 3, format: wgpu::VertexFormat::Float32x4 },
        ],
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[Some(vertex_layout)],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            // 面は表裏とも見える(2D図形は裏から見ることがあり、線の帯は向きが一定でない)。
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(depth_write),
            // 反転Z(camera.rs参照)。
            depth_compare: Some(depth_compare),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState { count: SAMPLE_COUNT, mask: !0, alpha_to_coverage_enabled: false },
        multiview_mask: None,
        cache: None,
    })
}

impl TerrainRenderer {
    pub async fn new(canvas: web_sys::HtmlCanvasElement) -> Result<Self, String> {
        let width = canvas.width().max(1);
        let height = canvas.height().max(1);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(|e| format!("create_surface failed: {e}"))?;

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
        let format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let mut config = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| "surface is not supported by adapter".to_string())?;
        config.format = format;
        config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        config.present_mode = wgpu::PresentMode::Fifo;
        surface.configure(&device, &config);

        let (supersample_width, supersample_height) = supersample_size(width, height);
        let depth_view = create_depth_view(&device, supersample_width, supersample_height);
        let msaa_view = create_msaa_view(&device, config.format, supersample_width, supersample_height);
        let supersample_color_view =
            create_supersample_color_view(&device, config.format, supersample_width, supersample_height);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("terrain.wgsl").into()),
        });

        let camera_uniform = CameraUniform {
            view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            shading: [0.0; 4],
            eye: [0.0; 4],
            forward: [0.0, 0.0, -1.0, 0.0],
            right: [1.0, 0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0, 0.0],
            ellipsoid_m: [[0.0; 4]; 3],
            ellipsoid_g: [0.0; 4],
        };
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera_buffer"),
            contents: bytemuck::bytes_of(&camera_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    // 頂点(view_proj・陰影)と、水域レイヤーのフラグメント(視線・楕円体)が読む。
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bind_group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain_pipeline_layout"),
            bind_group_layouts: &[Some(&camera_bind_group_layout)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<super::mesh::TerrainVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 2 * std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Snorm16x2,
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(vertex_layout.clone())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // フェーズ8時点では巻き順(ワインディング)の検証よりも描画確認を優先し、
                // カリングを無効化しておく(誤った巻き順でも常に地形が見える)。
                // 自由視点カメラ実装(フェーズ10)時に正しい巻き順を確認して有効化する。
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                // 反転Z(camera.rs参照): 深度値が大きいほどカメラに近い。「より近ければ
                // 上書きする」という意味は変わらないが、比較の向きはGreaterになる。
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLE_COUNT,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });

        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("marker_line_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(vertex_layout.clone())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                // 反転Z(camera.rs参照)。
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLE_COUNT,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });

        // 水域レイヤー用パイプライン。画面いっぱいの三角形(頂点バッファなし)を、地形メッシュより先に
        // 描く。深度テストはしない(常に描く)が、フラグメントシェーダーが楕円体との交点の深度
        // (`WATER_DEPTH_MARGIN_M`だけ奥へずらしたもの)を書く。続く地形メッシュは通常の深度テストで
        // これと比較され、楕円体の向こう側の地形は隠れ、手前(と余裕の範囲)の地形は水域の上に描かれる。
        // 視線が楕円体に当たらない画素(空)は`discard`してclearの黒(深度0=最遠)のままにする。
        let water_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_fullscreen"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_water"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLE_COUNT,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });

        // 覆域ドーム(半球状の面)用パイプライン。地形・マーカーの奥に透けて見えるよう、
        // アルファブレンドを有効にし、深度は「テストはする(地形より奥にあれば隠れる)が
        // 書き込みはしない(ドーム自身の三角形同士が奥行き順で互いを隠して欠けて見える
        // ことがないようにする、半透明物体の簡易的な描き方)」にしてある。
        let dome_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dome_surface_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(vertex_layout)],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_dome"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                // 反転Z(camera.rs参照)。書き込みはしないが、地形より奥(=depth値が
                // 小さい)なら隠れてほしいので比較の向きはGreaterのまま(通常Zの頃と
                // 意味は変わらない)。
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLE_COUNT,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });

        // スーパーサンプリングのダウンサンプルパイプライン(頂点バッファなし、画面いっぱいの
        // 三角形1枚。terrain.wgslのvs_fullscreen/fs_downsample参照)。
        let downsample_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("downsample_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let downsample_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("downsample_pipeline_layout"),
                bind_group_layouts: &[Some(&downsample_bind_group_layout)],
                immediate_size: 0,
            });
        let downsample_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("downsample_pipeline"),
            layout: Some(&downsample_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_fullscreen"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_downsample"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            // スワップチェーンへ直接(シングルサンプルで)描くパスなので深度・MSAAとも不要。
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let downsample_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("downsample_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let downsample_bind_group = create_downsample_bind_group(
            &device,
            &downsample_bind_group_layout,
            &downsample_sampler,
            &supersample_color_view,
        );

        // 作図用(`draw.wgsl`)。地形とは別のシェーダー・bind group(uniformの中身が違う)。
        let draw_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("draw_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("draw.wgsl").into()),
        });
        let draw_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("draw_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let draw_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("draw_pipeline_layout"),
            bind_group_layouts: &[Some(&draw_bind_group_layout)],
            immediate_size: 0,
        });
        let make_draw_pipeline = |label: &str, depth_write: bool, depth_compare: wgpu::CompareFunction| {
            create_draw_pipeline(
                &device,
                &draw_pipeline_layout,
                &draw_shader,
                config.format,
                label,
                depth_write,
                depth_compare,
            )
        };
        let draw_opaque_pipeline = make_draw_pipeline("draw_opaque_pipeline", true, wgpu::CompareFunction::Greater);
        let draw_blend_pipeline = make_draw_pipeline("draw_blend_pipeline", false, wgpu::CompareFunction::Greater);
        let draw_screen_pipeline = make_draw_pipeline("draw_screen_pipeline", false, wgpu::CompareFunction::Always);
        let draw_world = DrawSpace::new(&device, &draw_bind_group_layout, "draw_world_uniform");
        let draw_view = DrawSpace::new(&device, &draw_bind_group_layout, "draw_view_uniform");
        let draw_screen = DrawSpace::new(&device, &draw_bind_group_layout, "draw_screen_uniform");

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            meshes: HashMap::new(),
            camera_buffer,
            camera_bind_group,
            depth_view,
            msaa_view,
            supersample_color_view,
            downsample_pipeline,
            downsample_bind_group_layout,
            downsample_sampler,
            downsample_bind_group,
            line_pipeline,
            marker_vertex_buffer: None,
            num_marker_vertices: 0,
            dome_pipeline,
            dome_vertex_buffer: None,
            num_dome_vertices: 0,
            hillshade: false,
            water_pipeline,
            ellipsoid: Cell::new(([[0.0; 4]; 3], [0.0; 4])),
            draw_opaque_pipeline,
            draw_blend_pipeline,
            draw_screen_pipeline,
            draw_world,
            draw_view,
            draw_screen,
            world_opaque: VertexBatch::empty(),
            world_blend: VertexBatch::empty(),
            view_opaque: VertexBatch::empty(),
            view_blend: VertexBatch::empty(),
            screen_batch: VertexBatch::empty(),
        })
    }

    /// 作図(`terrain::drawing`)の頂点列を更新する。作図の一覧・原点・地形のLOD・canvasの大きさが
    /// 変わるたびに`drawing_geometry::build`で作り直して呼ぶ。
    pub fn update_drawings(&mut self, batches: &DrawingBatches) {
        self.world_opaque.set(&self.device, "draw_world_opaque", &batches.world.opaque);
        self.world_blend.set(&self.device, "draw_world_blend", &batches.world.blend);
        self.view_opaque.set(&self.device, "draw_view_opaque", &batches.view.opaque);
        self.view_blend.set(&self.device, "draw_view_blend", &batches.view.blend);
        self.screen_batch.set(&self.device, "draw_screen", &batches.screen);
    }

    /// canvasの内部解像度(ピクセル)。作図の画面座標(`Position::Screen`)の角の位置を決めるのに使う。
    pub fn canvas_size_px(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// レーダー観測点マーカー(四角い枠)の頂点データを更新する。原点変更・マーカー追加/
    /// 削除/選択変更のたびに呼び直す想定(`terrain/markers.rs`が頂点データを作る)。
    pub fn update_markers(&mut self, vertices: &[super::mesh::TerrainVertex]) {
        if vertices.is_empty() {
            self.marker_vertex_buffer = None;
            self.num_marker_vertices = 0;
            return;
        }
        self.marker_vertex_buffer = Some(self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("marker_vertex_buffer"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }));
        self.num_marker_vertices = vertices.len() as u32;
    }

    /// 選択中マーカーの覆域ドーム(半球状の面、TriangleList)の頂点データを更新する。
    /// `update_markers`と同じタイミングで呼び直す想定。
    pub fn update_dome(&mut self, vertices: &[super::mesh::TerrainVertex]) {
        if vertices.is_empty() {
            self.dome_vertex_buffer = None;
            self.num_dome_vertices = 0;
            return;
        }
        self.dome_vertex_buffer = Some(self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("dome_vertex_buffer"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }));
        self.num_dome_vertices = vertices.len() as u32;
    }

    /// メッシュ1個を登録する。同じキーが既にあれば(解像度レベルの切り替え)置き換える。
    pub fn set_mesh(&mut self, key: MeshKey, mesh: &TerrainMesh) {
        if mesh.indices.is_empty() {
            self.meshes.remove(&key);
            return;
        }
        let vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain_mesh_vertex_buffer"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            // COPY_DST: 原点変更時にupdate_mesh_vertices()で頂点データを書き換えられるようにする。
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let index_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain_mesh_index_buffer"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        self.meshes.insert(
            key,
            MeshGpu {
                vertex_buffer,
                index_buffer,
                num_indices: mesh.indices.len() as u32,
                bounds: Cell::new(position_bounds(&mesh.vertices)),
            },
        );
    }

    /// メッシュ1個を取り除く(GPUバッファは解放される)。
    pub fn remove_mesh(&mut self, key: MeshKey) {
        self.meshes.remove(&key);
    }

    /// 原点変更時など、頂点数は変わらないまま座標(位置)だけを更新したいときに使う。
    /// DETAILED_DESIGN.md 3.3節: 原点を変更したら頂点バッファを再計算・再アップロードする
    /// (タイルデータの再フェッチは不要)。登録されていないキーは何もしない。
    pub fn update_mesh_vertices(&self, key: MeshKey, vertices: &[TerrainVertex]) {
        if let Some(mesh) = self.meshes.get(&key) {
            self.queue.write_buffer(&mesh.vertex_buffer, 0, bytemuck::cast_slice(vertices));
            mesh.bounds.set(position_bounds(vertices));
        }
    }

    /// 登録済みのメッシュ数(デバッグ・確認用)。
    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        let (supersample_width, supersample_height) = supersample_size(width, height);
        self.depth_view = create_depth_view(&self.device, supersample_width, supersample_height);
        self.msaa_view =
            create_msaa_view(&self.device, self.config.format, supersample_width, supersample_height);
        self.supersample_color_view = create_supersample_color_view(
            &self.device,
            self.config.format,
            supersample_width,
            supersample_height,
        );
        // supersample_color_viewを作り直したので、それを参照しているbind groupも作り直す。
        self.downsample_bind_group = create_downsample_bind_group(
            &self.device,
            &self.downsample_bind_group_layout,
            &self.downsample_sampler,
            &self.supersample_color_view,
        );
    }

    pub fn aspect_ratio(&self) -> f32 {
        self.config.width as f32 / self.config.height.max(1) as f32
    }

    /// canvasの内部解像度の縦幅(ピクセル)。2Dモードのドラッグ操作(パン)で、
    /// 画面上のピクセル移動量をワールド座標(メートル)へ変換するのに使う
    /// (`components/terrain_view.rs`参照)。
    pub fn canvas_height_px(&self) -> u32 {
        self.config.height
    }

    /// 陰影(ヒルシェード)を付けるかを切り替える。次の`render`から反映される(メッシュの作り直しは不要)。
    pub fn set_hillshade(&mut self, enabled: bool) {
        self.hillshade = enabled;
    }

    /// 水域レイヤー(WGS84楕円体の海抜0mの面)の基準を、地形メッシュの現在の原点に合わせる。
    /// 頂点位置を新しい原点のENU座標で作り直すとき(初期化・原点変更)に、同時に呼ぶこと。
    pub fn set_ellipsoid_origin(&self, transform: &super::mesh::EnuTransform) {
        self.ellipsoid.set(transform.ellipsoid_shader_params());
    }

    pub fn render(&self, camera: &Camera) -> Result<(), String> {
        let [eye, forward, right, up] = camera.water_ray_basis();
        let (ellipsoid_m, ellipsoid_g) = self.ellipsoid.get();
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
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));

        // 作図のuniform。絶対座標=地形と同じカメラ、視点空間=射影だけ(カメラから見た座標をそのまま射影)、
        // 画面=ピクセル座標。
        let (width, height) = (self.config.width as f32, self.config.height as f32);
        let viewport = [width, height, 0.0, 0.0];
        for (space, view_proj, light) in [
            (&self.draw_world, camera.view_proj_matrix(), WORLD_DRAW_LIGHT),
            (&self.draw_view, camera.projection_matrix(), VIEW_DRAW_LIGHT),
            (&self.draw_screen, screen_matrix(width, height), [0.0; 4]),
        ] {
            let uniform = DrawUniform { view_proj: view_proj.to_cols_array_2d(), viewport, light };
            self.queue.write_buffer(&space.buffer, 0, bytemuck::bytes_of(&uniform));
        }
        // カメラ固定の作図(視点空間・画面)は、地形の奥行きとは別に、地形の手前へ重ねて描く。
        // 深度バッファを作り直す必要があるので、地形のパスとは別のパスにする。
        let has_overlay = !(self.view_opaque.is_empty() && self.view_blend.is_empty() && self.screen_batch.is_empty());

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
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

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terrain_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_view,
                    // スワップチェーンへ直接ではなく、スーパーサンプリングの内部解像度テクスチャへ
                    // 解決する(この後のdownsampleパスで実際のcanvas解像度へ縮小する)。
                    // カメラ固定の作図があるときは、続くオーバーレイのパスが解決する。
                    resolve_target: if has_overlay { None } else { Some(&self.supersample_color_view) },
                    depth_slice: None,
                    ops: wgpu::Operations {
                        // 地形が描かれない領域(空・海・データ範囲の外側)は黒。
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                        // resolve_targetへ解決した後はこのMSAAテクスチャ自体は不要なので
                        // 保存しない(Discard)。オーバーレイのパスへ引き継ぐときだけ保存する。
                        store: if has_overlay { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        // 反転Z(reversed-Z): 「最も遠い」を表す深度値は0.0(camera.rs参照)。
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // 水域レイヤーを最初に描く(楕円体との交点の深度を書く。続く地形メッシュは深度テストで、
            // 地球本体の向こう側は隠れ、手前(と余裕の範囲)は水域の上に描かれる)。
            render_pass.set_pipeline(&self.water_pipeline);
            render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
            render_pass.draw(0..3, 0..1);

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
            // 視錐台の外のメッシュは描かない(全タイルをチャンクで常駐させているので、画面外の
            // 大量の頂点を毎フレーム処理しないため)。
            let view_proj = camera.view_proj_matrix();
            for mesh in self.meshes.values() {
                if is_outside_frustum(&view_proj, mesh.bounds.get()) {
                    continue;
                }
                render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                render_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..mesh.num_indices, 0, 0..1);
            }

            if let Some(marker_buffer) = self.marker_vertex_buffer.as_ref() {
                if self.num_marker_vertices > 0 {
                    render_pass.set_pipeline(&self.line_pipeline);
                    render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, marker_buffer.slice(..));
                    render_pass.draw(0..self.num_marker_vertices, 0..1);
                }
            }

            // 絶対座標の作図。不透明なものは覆域ドームより先に(深度を書く)、半透明なものはドームの後に描く。
            self.world_opaque.draw(&mut render_pass, &self.draw_opaque_pipeline, &self.draw_world);

            // 覆域ドーム(半透明)は他の不透明な描画がすべて終わった後に描く。
            if let Some(dome_buffer) = self.dome_vertex_buffer.as_ref() {
                if self.num_dome_vertices > 0 {
                    render_pass.set_pipeline(&self.dome_pipeline);
                    render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, dome_buffer.slice(..));
                    render_pass.draw(0..self.num_dome_vertices, 0..1);
                }
            }

            self.world_blend.draw(&mut render_pass, &self.draw_blend_pipeline, &self.draw_world);
        }

        if has_overlay {
            // カメラ固定の作図のパス: 地形を描いた色をそのまま(Load)引き継ぎ、深度だけ作り直して(地形とは
            // 隠し合わない)、視点空間の図形(図形どうしは奥行きで隠し合う)→画面座標の図形(追加順)の順に描く。
            let mut overlay_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("drawing_overlay_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_view,
                    resolve_target: Some(&self.supersample_color_view),
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.view_opaque.draw(&mut overlay_pass, &self.draw_opaque_pipeline, &self.draw_view);
            self.view_blend.draw(&mut overlay_pass, &self.draw_blend_pipeline, &self.draw_view);
            self.screen_batch.draw(&mut overlay_pass, &self.draw_screen_pipeline, &self.draw_screen);
        }

        {
            // スーパーサンプリングのダウンサンプルパス: 内部解像度で描いた
            // supersample_color_viewを、実際のcanvas解像度のスワップチェーンへ線形フィルタで
            // 縮小して描く(画面いっぱいの三角形1枚、頂点バッファなし)。
            let mut downsample_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("downsample_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            downsample_pass.set_pipeline(&self.downsample_pipeline);
            downsample_pass.set_bind_group(0, &self.downsample_bind_group, &[]);
            downsample_pass.draw(0..3, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(frame);
        Ok(())
    }
}

fn create_depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("terrain_depth_texture"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        // 同じレンダーパス内の他のアタッチメント(MSAAカラーテクスチャ)とサンプル数を
        // 揃える必要がある。
        sample_count: SAMPLE_COUNT,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// MSAA用の中間カラーテクスチャ(スーパーサンプリングの内部解像度、サンプル数
/// `SAMPLE_COUNT`)を作る。マルチサンプルテクスチャは直接シェーダーでサンプリングできない
/// ため、いったんこのテクスチャに描画してから`resolve_target`で
/// `supersample_color_view`(シングルサンプル、同じ内部解像度)へ解決する。
fn create_msaa_view(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("terrain_msaa_texture"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: SAMPLE_COUNT,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// MSAA解決先(スーパーサンプリングの内部解像度、シングルサンプル)。`downsample_pipeline`が
/// テクスチャとしてサンプリングするため`TEXTURE_BINDING`も付与する。
fn create_supersample_color_view(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("terrain_supersample_color_texture"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// `downsample_pipeline`用のbind group(スーパーサンプリング済みテクスチャ+線形サンプラー)。
/// `supersample_color_view`を作り直すたび(初期化時・resize時)に作り直す必要がある。
fn create_downsample_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    supersample_color_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("downsample_bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(supersample_color_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}
