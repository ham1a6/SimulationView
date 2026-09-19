struct CameraUniform {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: CameraUniform;

// 表示オプション(表示メニュー「海を表示」チェックボックス、terrain/display.rs参照)。
// show_water<0.5なら海(WATER_COLORとほぼ同じ色の頂点)のフラグメントを破棄する。
// 頂点に「海かどうか」の専用属性を持たせる代わりに、既存のcolor属性がWATER_COLORと
// 一致するかどうかで判定している(mesh.rsのNaNセル・背景スカートはどちらも
// 厳密に同じWATER_COLORを使っているため、追加の頂点属性なしで判別できる)。
struct DisplayUniform {
    show_water: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};
@group(0) @binding(1)
var<uniform> display: DisplayUniform;

const WATER_COLOR: vec3<f32> = vec3<f32>(0.55, 0.78, 0.92);

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    if (display.show_water < 0.5 && distance(in.color, WATER_COLOR) < 0.01) {
        discard;
    }
    return vec4<f32>(in.color, 1.0);
}

// 見通し範囲の覆域ドーム(半球状の面)用。地形やマーカーを透けて見せたいため、
// 固定の半透明アルファで出力する(頂点データ自体は他のパイプラインと共用のposition+colorのまま)。
@fragment
fn fs_dome(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 0.22);
}

// スーパーサンプリングのダウンサンプル用(renderer.rsのdownsample_pipeline)。地形メッシュを
// 内部解像度(画面の`SUPERSAMPLE_FACTOR`倍、4倍MSAA込み)で描いた後、このシェーダーで画面
// いっぱいの三角形を1枚描いて線形フィルタでサンプリングし、実際のcanvas解像度へ縮小する。
// 4倍MSAAだけでは、2048×2048化後の遠景・浅い角度で細かい陸地/海(NaN)の三角形による
// エイリアシング(斑点・ちらつき)を抑えきれなかったため導入した
// (「背景と同じ色の点が多数表示される/ズーム操作やカメラ操作時に画面がちかちかする」との
// 報告を受けて対処)。頂点バッファを使わない「画面いっぱいの三角形」の定石
// (vertex_indexだけから3頂点を計算し、クリップ領域外にはみ出す部分はラスタライザが
// 自動的に切り捨てる)を使っている。
struct DownsampleOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> DownsampleOutput {
    var out: DownsampleOutput;
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    out.clip_position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

@group(0) @binding(0)
var supersample_texture: texture_2d<f32>;
@group(0) @binding(1)
var supersample_sampler: sampler;

@fragment
fn fs_downsample(in: DownsampleOutput) -> @location(0) vec4<f32> {
    return textureSample(supersample_texture, supersample_sampler, in.uv);
}
