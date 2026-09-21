struct CameraUniform {
    view_proj: mat4x4<f32>,
    // x: 陰影(ヒルシェード)を付けるなら1、付けないなら0。y,z,wは未使用。
    shading: vec4<f32>,
    // 水域レイヤー(fs_water)が各画素の視線を求めるための値(`Camera::water_ray_basis`):
    // eye.xyz=視点、eye.w=1なら透視投影・0なら正射影。forward=視線方向。right/up=画面端までの長さ倍。
    eye: vec4<f32>,
    forward: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    // WGS84楕円体(海抜0m)の陰関数の係数(`EnuTransform::ellipsoid_shader_params`)。
    // f(p) = ellipsoid_g.w + 2 g・(M p) + |M p|^2。M=ellipsoid_m(3行)、g=ellipsoid_g.xyz。
    ellipsoid_m: array<vec4<f32>, 3>,
    ellipsoid_g: vec4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    // 単位法線のx(East)・y(North)成分(snorm16x2)。z(Up)は復元する。長さが1を超える値は
    // 「陰影を付けない」印(`TerrainVertex::UNLIT_NORMAL`。マーカー・覆域ドームなど)。
    @location(2) normal_xy: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

// 陰影(ヒルシェード)の光源。ENU座標(東,北,上)で、北西から仰角45度の固定(地図の陰影の
// 一般的な向き)。カメラの向きに依らず、地形の見え方が変わらない。単位ベクトル。
const LIGHT_DIR = vec3<f32>(-0.5, 0.5, 0.70710678);
// 光が当たらない面(光源の反対側の斜面)の明るさの下限(0〜1)。
const AMBIENT = 0.35;

// 頂点の色に掛ける明るさ。平地(法線が真上)は1(色が変わらない)、光源側に向いた斜面は1より
// 明るく、反対側の斜面は暗くなる。
fn hillshade(normal_xy: vec2<f32>) -> f32 {
    let xy2 = dot(normal_xy, normal_xy);
    if (xy2 > 1.0) {
        return 1.0;
    }
    let normal = vec3<f32>(normal_xy, sqrt(1.0 - xy2));
    let lambert = max(dot(normal, LIGHT_DIR), 0.0);
    let flat_level = AMBIENT + (1.0 - AMBIENT) * LIGHT_DIR.z;
    return (AMBIENT + (1.0 - AMBIENT) * lambert) / flat_level;
}

fn shade_vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    // 陰影は頂点ごとに求めて色に掛け、面の内側は補間する(明るさは法線について線形なので、
    // 法線を補間してからフラグメントごとに求めるのとほぼ同じ結果になる)。
    out.color = in.color * mix(1.0, hillshade(in.normal_xy), camera.shading.x);
    return out;
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    return shade_vertex(in);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}

// 解像度レベルの切り替え中のメッシュ(クロスフェード)用。新旧のメッシュを、画面の画素ごとの
// 疎密(ディザ)で互いに補う割合で重ねて描き、いきなり切り替わらず、混ざりながら入れ替わって見せる。
// 半透明ではなく`discard`なので、深度は通常どおり書け、描く順にも依存しない(新旧は画素ごとに
// どちらか一方だけが描かれる)。
// メッシュごとの値は、`instance_index`で引く表(`fades`)に入れる(頂点バッファ・bind groupを
// メッシュごとに増やさない。描画側が`first_instance`に表の番号を渡す)。
// x: 表示する画素の割合(0〜1)、y: 1なら、表示する画素を反転する(x=新しい側の割合に対する、古い側)。
struct FadeTable {
    entries: array<vec4<f32>, 2048>,
};
@group(1) @binding(0)
var<uniform> fades: FadeTable;

struct FadeVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) @interpolate(flat) fade: vec2<f32>,
};

@vertex
fn vs_fade(in: VertexInput, @builtin(instance_index) instance: u32) -> FadeVertexOutput {
    let base = shade_vertex(in);
    var out: FadeVertexOutput;
    out.clip_position = base.clip_position;
    out.color = base.color;
    out.fade = fades.entries[instance].xy;
    return out;
}

// 画素の位置から0〜1の疎密のパターンを作る(interleaved gradient noise。周期が短く、隣の画素と
// 値が散るので、割合が同じなら画面全体に均一な粒になる)。
fn dither_noise(pixel: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fs_fade(in: FadeVertexOutput) -> @location(0) vec4<f32> {
    let noise = dither_noise(in.clip_position.xy);
    // 新しい側は noise < 割合 の画素、古い側は残りの画素。
    // (比較を`select`の引数に直接書くと、`<`と`>`がテンプレートの括弧と読まれる)
    let is_new = noise < in.fade.x;
    let inverted = in.fade.y > 0.5;
    if (is_new == inverted) {
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

// 水域レイヤー。「地形データが無いところ(マスクファイルの水域・欠損タイル・地形データの範囲外)」は
// 地形メッシュが張られないので、そこから見えるのは地形の後ろの背景になる。そこにWGS84楕円体の
// 海抜0mの面を水色で描く。メッシュではなく画面いっぱいの三角形1枚で、各画素の視線と楕円体の
// 交点があるかをシェーダーで直接判定する(交点なし=空は描かずclearの黒のまま)。
// - 地球の丸み・楕円体を厳密に扱える(メッシュ近似の弦の誤差・継ぎ目・範囲の限りがない)
// - 地形メッシュより先に描く。**交点の深度を書く**ので、楕円体(地球本体)の向こう側にある地形は
//   隠れる(地球の丸みの向こう側・海面の下・海岸の張り出した縁の下から見える地形の裏側が、
//   水面越しに透けて見えない)。ただし深度は交点より視線方向に`WATER_DEPTH_MARGIN_M`奥へずらして
//   書く: 地形は交点から`WATER_DEPTH_MARGIN_M`以内の奥までは水域より手前として描かれ、
//   標高が0m以下(DSMの負の値・海面以下の陸地)の地形が水域に隠れることはない
const WATER_COLOR = vec3<f32>(0.25, 0.55, 0.85);
// 水域の深度を、視線の交点から奥へずらす距離(メートル、視線に沿って)。地形が0m以下でも隠れない
// ための余裕。標高-Hの地形が視線と角度thetaで交わるとき、交点との視線方向の距離はH/sin(theta)。
// 標高-10mでもtheta>=約0.6度(ほぼ水平の見え方)まで隠れない。一方、地球の丸みの向こう側の地形は
// 水平線からの距離が数km以上なので、これより大きく奥になり隠れる。
const WATER_DEPTH_MARGIN_M = 1000.0;

struct WaterOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_water(in: DownsampleOutput) -> WaterOutput {
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let offset = camera.right.xyz * ndc.x + camera.up.xyz * ndc.y;
    let perspective = camera.eye.w;
    let origin = camera.eye.xyz + offset * (1.0 - perspective);
    let dir = camera.forward.xyz + offset * perspective;

    let m0 = camera.ellipsoid_m[0].xyz;
    let m1 = camera.ellipsoid_m[1].xyz;
    let m2 = camera.ellipsoid_m[2].xyz;
    let g = camera.ellipsoid_g.xyz;
    let q0 = vec3<f32>(dot(m0, origin), dot(m1, origin), dot(m2, origin));
    let w = vec3<f32>(dot(m0, dir), dot(m1, dir), dot(m2, dir));

    // f(origin + t*dir) = a t^2 + 2 half_b t + c = 0 の解(視線と楕円体の交点)。
    let a = dot(w, w);
    let half_b = dot(g, w) + dot(q0, w);
    let c = camera.ellipsoid_g.w + 2.0 * dot(g, q0) + dot(q0, q0);
    let disc = half_b * half_b - a * c;
    if (disc < 0.0) {
        discard;
    }
    // 桁落ちしにくい解の公式。視点は楕円体の外にあるので、2つの解は同符号で、
    // 正なら視線の前方で交わる(負なら後ろ向き=空)。手前の交点(小さい方の解)が水面。
    let s = sqrt(disc);
    let q = -(half_b + select(-s, s, half_b >= 0.0));
    let t1 = q / a;
    let t2 = c / q;
    let t_near = min(t1, t2);
    if (!(max(t1, t2) > 0.0) || !(t_near > 0.0)) {
        discard;
    }

    // 交点から視線に沿って奥へ`WATER_DEPTH_MARGIN_M`ずらした点の深度(反転Z: 0=遠い、1=近い)。
    let hit = origin + dir * (t_near + WATER_DEPTH_MARGIN_M / length(dir));
    let clip = camera.view_proj * vec4<f32>(hit, 1.0);
    var out: WaterOutput;
    out.color = vec4<f32>(WATER_COLOR, 1.0);
    out.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    return out;
}

// スーパーサンプリングのダウンサンプル用(renderer.rsのdownsample_pipeline)。地形メッシュを
// 内部解像度(画面の`SUPERSAMPLE_FACTOR`倍、4倍MSAA込み)で描いた後、このシェーダーで画面
// いっぱいの三角形を1枚描いて線形フィルタでサンプリングし、実際のcanvas解像度へ縮小する。
// 4倍MSAAだけでは、2048×2048化後の遠景・浅い角度で細かい陸地の三角形による
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
