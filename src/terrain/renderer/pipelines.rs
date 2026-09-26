//! 描画パイプラインの生成。パイプラインごとの違い(シェーダーのエントリ・頂点バッファ・ブレンド・
//! 深度)だけを`PipelineSpec`で指定し、共通の設定(三角形リスト・カリングなし・MSAA)はここに1つだけ持つ。

use super::targets::SAMPLE_COUNT;
use super::uniforms::{uniform_layout, DRAW_VERTEX_ATTRIBUTES, TERRAIN_VERTEX_ATTRIBUTES};
use crate::terrain::mesh::TerrainVertex;
use crate::terrain::vertex::DrawVertex;

/// 頂点バッファ1本のレイアウト(1要素が`T`)。
pub(super) fn vertex_layout<T>(
    attributes: &[wgpu::VertexAttribute],
    step_mode: wgpu::VertexStepMode,
) -> wgpu::VertexBufferLayout<'_> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<T>() as wgpu::BufferAddress,
        step_mode,
        attributes,
    }
}

/// bind groupのレイアウトを並べたパイプラインレイアウト(添字がgroup番号)。
pub(super) fn pipeline_layout(
    device: &wgpu::Device,
    label: &str,
    bind_group_layouts: &[&wgpu::BindGroupLayout],
) -> wgpu::PipelineLayout {
    let layouts: Vec<Option<&wgpu::BindGroupLayout>> =
        bind_group_layouts.iter().copied().map(Some).collect();
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &layouts,
        immediate_size: 0,
    })
}

/// WGSLのシェーダーモジュール。
pub(super) fn wgsl_module(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

/// パイプライン1本ぶんの、パイプラインごとに違う設定。
pub(super) struct PipelineSpec<'a> {
    /// デバッグ用のラベル。
    pub label: &'a str,
    /// bind groupの並び。
    pub layout: &'a wgpu::PipelineLayout,
    /// 頂点・フラグメントの両方をこのモジュールから取る。
    pub shader: &'a wgpu::ShaderModule,
    /// 頂点シェーダーの関数名。
    pub vs_entry: &'a str,
    /// フラグメントシェーダーの関数名。
    pub fs_entry: &'a str,
    /// 頂点バッファのレイアウト(添字がスロット番号)。空なら頂点バッファなし(画面いっぱいの三角形を頂点番号から作る)。
    pub buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    /// 出力先のカラー形式(surfaceと同じ)。
    pub format: wgpu::TextureFormat,
    /// 出力色の合成方法(上書き・アルファブレンドなど)。
    pub blend: wgpu::BlendState,
    /// (深度を書くか, 深度の比較)。`None`なら深度バッファなし。
    pub depth: Option<(bool, wgpu::CompareFunction)>,
    /// マルチサンプルのサンプル数(深度・MSAAカラーを使うパスと揃える)。
    pub samples: u32,
}

/// `spec`と共通の設定からパイプラインを作る(出力先はカラー1枚)。
pub(super) fn create_pipeline(device: &wgpu::Device, spec: &PipelineSpec) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(spec.label),
        layout: Some(spec.layout),
        vertex: wgpu::VertexState {
            module: spec.shader,
            entry_point: Some(spec.vs_entry),
            buffers: spec.buffers,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: spec.shader,
            entry_point: Some(spec.fs_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: spec.format,
                blend: Some(spec.blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            // 裏面カリングは使っていない。地形は高さ場で、上から見る限り裏面はほとんど映らないので
            // 効果は小さい。有効化するなら、スカート(縁の壁)の巻き順が4辺で揃っているかを先に
            // 確認する必要がある(揃っていないとクラックが出る。AGENTS.mdの既知の技術的負債)。
            // 作図の面は表裏とも見える(2D図形は裏から見ることがあり、線の帯は向きが一定でない)。
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: spec.depth.map(|(write, compare)| wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(write),
            // 反転Z(camera.rs参照): 深度値が大きいほどカメラに近い。
            depth_compare: Some(compare),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: spec.samples,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}

/// 地形用のシェーダーモジュール(地形・水域・ドーム・縮小の4本が共有する)。
pub(super) fn terrain_shader(device: &wgpu::Device) -> wgpu::ShaderModule {
    wgsl_module(device, "terrain_shader", include_str!("../terrain.wgsl"))
}

/// メインパスで使うパイプライン一式(縮小パスのパイプラインは`targets::Downsample`)。
pub(super) struct Pipelines {
    /// 地形メッシュ(不透明、深度を書く)。
    pub terrain: wgpu::RenderPipeline,
    /// 解像度レベルの切り替え中のメッシュ(クロスフェード。`fade`)。地形と同じだが、メッシュごとの割合の表
    /// (`fade_bind_group_layout`)を`instance_index`で引き、ディザで`discard`する。
    pub terrain_fade: wgpu::RenderPipeline,
    /// クロスフェードの割合の表(`terrain.wgsl`の`FadeTable`)のbind groupレイアウト。
    pub fade_bind_group_layout: wgpu::BindGroupLayout,
    /// 水域レイヤー(画面いっぱいの三角形で、楕円体との交点を画素ごとに求める)。
    pub water: wgpu::RenderPipeline,
    /// 覆域ドーム(半透明、深度は読むが書かない)。
    pub dome: wgpu::RenderPipeline,
    /// 作図: 不透明(深度を書く)・半透明(深度は書かずにアルファブレンド)・画面座標(深度テストなし)。
    pub draw_opaque: wgpu::RenderPipeline,
    /// 作図・マーカー・航跡の半透明(深度テストあり・書き込みなし)。
    pub draw_blend: wgpu::RenderPipeline,
    /// 作図の画面座標と2Dの覆域(深度テストなし。描く順で重ねる)。
    pub draw_screen: wgpu::RenderPipeline,
    /// 作図のuniform(`DrawUniform`)のbind groupレイアウト。
    pub draw_bind_group_layout: wgpu::BindGroupLayout,
}

impl Pipelines {
    /// メインパスのパイプラインをすべて作る。地形系は`terrain_shader`と`camera_bind_group_layout`(group 0)を、
    /// 作図系は`draw.wgsl`と作図のuniformのレイアウトを使う。
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        terrain_shader: &wgpu::ShaderModule,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let terrain_layout = pipeline_layout(
            device,
            "terrain_pipeline_layout",
            &[camera_bind_group_layout],
        );
        let terrain_vertices = [Some(vertex_layout::<TerrainVertex>(
            &TERRAIN_VERTEX_ATTRIBUTES,
            wgpu::VertexStepMode::Vertex,
        ))];
        let terrain_spec = |label, vs_entry, fs_entry, buffers, blend, depth| PipelineSpec {
            label,
            layout: &terrain_layout,
            shader: terrain_shader,
            vs_entry,
            fs_entry,
            buffers,
            format,
            blend,
            depth: Some(depth),
            samples: SAMPLE_COUNT,
        };

        // 地形メッシュ。深度は書き、「より近ければ上書き」(反転Zなのでcompareはgreater)。
        let terrain = create_pipeline(
            device,
            &terrain_spec(
                "terrain_pipeline",
                "vs_main",
                "fs_main",
                &terrain_vertices,
                wgpu::BlendState::REPLACE,
                (true, wgpu::CompareFunction::Greater),
            ),
        );
        // クロスフェード中のメッシュ。地形と同じ設定で、フラグメントがディザで`discard`する
        // (`discard`を持つシェーダーは早期深度テストが効きにくいので、切り替え中のメッシュだけに使う)。
        let fade_bind_group_layout =
            uniform_layout(device, "fade_bind_group_layout", wgpu::ShaderStages::VERTEX);
        let fade_layout = pipeline_layout(
            device,
            "terrain_fade_pipeline_layout",
            &[camera_bind_group_layout, &fade_bind_group_layout],
        );
        let terrain_fade = create_pipeline(
            device,
            &PipelineSpec {
                layout: &fade_layout,
                ..terrain_spec(
                    "terrain_fade_pipeline",
                    "vs_fade",
                    "fs_fade",
                    &terrain_vertices,
                    wgpu::BlendState::REPLACE,
                    (true, wgpu::CompareFunction::Greater),
                )
            },
        );
        // 水域レイヤー。画面いっぱいの三角形(頂点バッファなし)を、地形メッシュより先に描く。深度テストは
        // しない(常に描く)が、フラグメントシェーダーが楕円体との交点の深度(`WATER_DEPTH_MARGIN_M`だけ
        // 奥へずらしたもの)を書く。続く地形メッシュは通常の深度テストでこれと比較され、楕円体の向こう側の
        // 地形は隠れ、手前(と余裕の範囲)の地形は水域の上に描かれる。視線が楕円体に当たらない画素(空)は
        // `discard`してclearの黒(深度0=最遠)のままにする。
        let water = create_pipeline(
            device,
            &terrain_spec(
                "water_pipeline",
                "vs_fullscreen",
                "fs_water",
                &[],
                wgpu::BlendState::REPLACE,
                (true, wgpu::CompareFunction::Always),
            ),
        );
        // 覆域ドーム(半球状の面)。地形・マーカーの奥に透けて見えるよう、アルファブレンドを有効にし、
        // 深度は「テストはする(地形より奥にあれば隠れる)が書き込みはしない(ドーム自身の三角形同士が
        // 奥行き順で互いを隠して欠けて見えることがないようにする、半透明物体の簡易的な描き方)」にする。
        // 書き込まないが、地形より奥(=depth値が小さい)なら隠れてほしいので比較はGreaterのまま。
        let dome = create_pipeline(
            device,
            &terrain_spec(
                "dome_surface_pipeline",
                "vs_main",
                "fs_dome",
                &terrain_vertices,
                wgpu::BlendState::ALPHA_BLENDING,
                (false, wgpu::CompareFunction::Greater),
            ),
        );

        // 作図用(`draw.wgsl`)。地形とは別のシェーダー・bind group(uniformの中身が違う)。
        let draw_shader = wgsl_module(device, "draw_shader", include_str!("../draw.wgsl"));
        let draw_bind_group_layout =
            uniform_layout(device, "draw_bind_group_layout", wgpu::ShaderStages::VERTEX);
        let draw_layout =
            pipeline_layout(device, "draw_pipeline_layout", &[&draw_bind_group_layout]);
        let draw_vertices = [Some(vertex_layout::<DrawVertex>(
            &DRAW_VERTEX_ATTRIBUTES,
            wgpu::VertexStepMode::Vertex,
        ))];
        // 作図のパイプラインは深度の設定だけが違う。どれもアルファブレンドにしておく(不透明の列の色は
        // アルファがほぼ1(`drawing_geometry::OPAQUE_ALPHA`以上)なので、上書きとほぼ同じ結果になる)。
        let draw = |label, depth| {
            create_pipeline(
                device,
                &PipelineSpec {
                    label,
                    layout: &draw_layout,
                    shader: &draw_shader,
                    vs_entry: "vs_main",
                    fs_entry: "fs_main",
                    buffers: &draw_vertices,
                    format,
                    blend: wgpu::BlendState::ALPHA_BLENDING,
                    depth: Some(depth),
                    samples: SAMPLE_COUNT,
                },
            )
        };
        let draw_opaque = draw(
            "draw_opaque_pipeline",
            (true, wgpu::CompareFunction::Greater),
        );
        let draw_blend = draw(
            "draw_blend_pipeline",
            (false, wgpu::CompareFunction::Greater),
        );
        let draw_screen = draw(
            "draw_screen_pipeline",
            (false, wgpu::CompareFunction::Always),
        );

        Self {
            terrain,
            terrain_fade,
            fade_bind_group_layout,
            water,
            dome,
            draw_opaque,
            draw_blend,
            draw_screen,
            draw_bind_group_layout,
        }
    }
}
