//! 描画先のテクスチャ(深度・MSAA・スーパーサンプリングの解決先)と、canvasの解像度への縮小(downsample)。
//!
//! 描画はcanvasの表示上の大きさ(CSSピクセル)の`SUPERSAMPLE_FACTOR`倍の内部解像度で、4倍MSAAをかけて
//! 行い、最後に線形フィルタでcanvasの内部解像度(CSSピクセル×devicePixelRatio)へ縮小する。
//! devicePixelRatioが2の高DPIの画面では縮小はほぼ等倍になり、描画の負荷は通常の画面と同じまま、
//! ブラウザによる引き伸ばしが無くなってくっきり表示される。

use super::pipelines::{create_pipeline, pipeline_layout, PipelineSpec};

/// マルチサンプルアンチエイリアシング(MSAA)のサンプル数。地形メッシュの解像度を
/// 高解像度化した後、遠景で多数の細かい三角形が1画素に収まりきらず
/// エイリアシング(市松状のちらつき/斑点)を起こすようになったため導入した。
/// 4はWebGPU実装で広くサポートされる標準的な値(8はハードウェアによって非対応。実機で
/// `createTexture({sampleCount: 8, ...})`がエラーになることを確認済み)。
pub(super) const SAMPLE_COUNT: u32 = 4;

/// スーパーサンプリングの倍率。4倍MSAAだけでは、やや引いた視点・浅い角度で地形の
/// エイリアシング(「背景と同じ色の点が多数表示される/ズーム操作やカメラ操作時に画面が
/// ちかちかする」)を抑えきれなかったため追加した。canvasの表示上の大きさ(CSSピクセル)の`SUPERSAMPLE_FACTOR`倍の
/// 内部解像度で描画(+4倍MSAA)した後、線形フィルタで実際のcanvasの内部解像度へ縮小する
/// (`Downsample`)。
const SUPERSAMPLE_FACTOR: u32 = 2;
/// スーパーサンプリング後の内部テクスチャの一辺の上限(ピクセル)。WebGPUが保証する
/// 最小の`maxTextureDimension2D`は8192だが、非常に大きなcanvas(4K超のウインドウ等)で
/// 2倍すると際どくなるため、余裕を持って安全側に制限する。
pub(super) const SUPERSAMPLE_MAX_DIMENSION: u32 = 4096;

/// canvasの表示上の大きさ(CSSピクセル)からスーパーサンプリング用の内部解像度を求める。
pub(super) fn supersample_size(width: u32, height: u32) -> (u32, u32) {
    let w = (width.max(1) * SUPERSAMPLE_FACTOR).min(SUPERSAMPLE_MAX_DIMENSION);
    let h = (height.max(1) * SUPERSAMPLE_FACTOR).min(SUPERSAMPLE_MAX_DIMENSION);
    (w, h)
}

/// 描画先のテクスチャ1枚(2D、ミップなし)を作ってビューを返す。
fn create_target(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
    sample_count: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// スーパーサンプリングの内部解像度の描画先3枚。canvasの大きさが変わるたびに作り直す。
pub(super) struct RenderTargets {
    /// 深度(Depth32Float。MSAAカラーとサンプル数を揃える)。
    pub depth_view: wgpu::TextureView,
    /// MSAA用の中間カラー(`SAMPLE_COUNT`倍のマルチサンプル)。マルチサンプルテクスチャは直接
    /// シェーダーでサンプリングできないため、いったんここへ描画してから`supersample_color_view`へ解決する。
    pub msaa_view: wgpu::TextureView,
    /// MSAAの解決先(シングルサンプル)。`Downsample`がテクスチャとしてサンプリングするので
    /// `TEXTURE_BINDING`も付ける。
    pub supersample_color_view: wgpu::TextureView,
}

impl RenderTargets {
    /// canvasの表示上の大きさ(CSSピクセル)から内部解像度(`supersample_size`)を決めて、3枚を作る。
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        css_width: u32,
        css_height: u32,
    ) -> Self {
        let size = supersample_size(css_width, css_height);
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
        Self {
            depth_view: create_target(
                device,
                "terrain_depth_texture",
                wgpu::TextureFormat::Depth32Float,
                size,
                SAMPLE_COUNT,
                attachment,
            ),
            msaa_view: create_target(
                device,
                "terrain_msaa_texture",
                format,
                size,
                SAMPLE_COUNT,
                attachment,
            ),
            supersample_color_view: create_target(
                device,
                "terrain_supersample_color_texture",
                format,
                size,
                1,
                attachment | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
        }
    }
}

/// スーパーサンプリングした画像を、canvasの解像度のスワップチェーンへ縮小するパス(頂点バッファなし、
/// 画面いっぱいの三角形1枚。`terrain.wgsl`の`vs_fullscreen`/`fs_downsample`)。
///
/// 縮小のフィルタは線形(`min_filter`もLinear)で、2倍の縮小では出力の1画素が元の2x2を平均する。
/// これがスーパーサンプリングの要なので、wgpu標準の`TextureBlitter`(縮小側がNearest固定)には置き換えない。
pub(super) struct Downsample {
    /// 縮小のパイプライン(深度・MSAAなし)。
    pipeline: wgpu::RenderPipeline,
    /// binding 0 = 縮小元のテクスチャ、binding 1 = サンプラー。`rebind`で使い回す。
    bind_group_layout: wgpu::BindGroupLayout,
    /// 線形フィルタのサンプラー(端はClampToEdge)。
    sampler: wgpu::Sampler,
    /// いまの縮小元を束縛したbind group。
    bind_group: wgpu::BindGroup,
}

impl Downsample {
    /// 縮小のパイプラインと、`supersample_color_view`を縮小元にしたbind groupを作る。
    pub(super) fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        format: wgpu::TextureFormat,
        supersample_color_view: &wgpu::TextureView,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
        let layout = pipeline_layout(device, "downsample_pipeline_layout", &[&bind_group_layout]);
        let pipeline = create_pipeline(
            device,
            &PipelineSpec {
                label: "downsample_pipeline",
                layout: &layout,
                shader,
                vs_entry: "vs_fullscreen",
                fs_entry: "fs_downsample",
                buffers: &[],
                format,
                blend: wgpu::BlendState::REPLACE,
                // スワップチェーンへ直接(シングルサンプルで)描くパスなので深度・MSAAとも不要。
                depth: None,
                samples: 1,
            },
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("downsample_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let bind_group =
            Self::create_bind_group(device, &bind_group_layout, &sampler, supersample_color_view);
        Self {
            pipeline,
            bind_group_layout,
            sampler,
            bind_group,
        }
    }

    /// 縮小元(`supersample_color_view`)を作り直したときに、それを参照するbind groupも作り直す。
    pub(super) fn rebind(
        &mut self,
        device: &wgpu::Device,
        supersample_color_view: &wgpu::TextureView,
    ) {
        self.bind_group = Self::create_bind_group(
            device,
            &self.bind_group_layout,
            &self.sampler,
            supersample_color_view,
        );
    }

    /// 縮小パスの描画(パスは呼び出し側が開く)。
    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// 縮小元のテクスチャとサンプラーを束縛したbind group。
    fn create_bind_group(
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
}
