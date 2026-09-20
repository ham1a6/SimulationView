// 作図機能(`terrain::drawing`)の描画シェーダー。面(塗り・立体)と太い線を同じ頂点形式で描く。
// 頂点データは`drawing_geometry::DrawVertex`。座標の種類(絶対座標・視点空間・画面)ごとに
// `view_proj`を差し替えて、同じシェーダーで描く。

struct DrawUniform {
    view_proj: mat4x4<f32>,
    // x,y: 描画先(canvas)の大きさ(px)。線の太さ(px)をクリップ空間へ換算するのに使う。
    viewport: vec4<f32>,
    // xyz: 光源の向き(面から光源へ向かう単位ベクトル。頂点の座標系で)。
    light: vec4<f32>,
};
@group(0) @binding(0)
var<uniform> u: DrawUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    // 面: 法線(陰影を付けるとき)。線: 反対側の端点。
    @location(2) aux: vec3<f32>,
    // x: 線の太さ(px。0以下なら面)、y: 線の側(-1/+1)、z: 1ならビルボード(画面サイズ固定のマーカー)、
    // w: 陰影を付けるなら1(面のみ)。
    @location(3) params: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

// 光が当たらない面の明るさの下限(0〜1)。
const AMBIENT = 0.4;
// 線の端点がカメラの後ろ側(クリップ空間のwが小さい)にあるとき、線分をこのwで打ち切る。
// (透視投影では、視点の後ろの点を画面へ射影すると向きが反転して、線の帯が壊れるため)
const LINE_MIN_W = 0.5;
// 線を、同じ位置の面(塗りや立体の表面)よりわずかに手前として深度テストするための、
// クリップ空間のzの相対的な加算量(反転Zなので大きいほど手前)。線が面と同じ深度になって
// 縞模様(Zファイティング)になるのを避ける。
const LINE_DEPTH_BIAS = 2.0e-5;
// ビルボード(マーカー)を手前へ寄せる、クリップ空間のzの相対的な加算量。マーカーは地表の1点に立てるので、
// 粗いLODの地形メッシュとの高さのずれ(遠いほど大きい)で地面に埋まって消えないよう、距離に比例して
// 大きめに寄せる(距離の0.2%: 400km先で約800m、5km先で10m)。これより手前の山には隠れる。
const BILLBOARD_DEPTH_BIAS = 2.0e-3;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    if (in.params.z > 0.5) {
        // ビルボード: position(アンカーの位置)を射影し、aux.xy(画面のpx、右・上が正)だけずらす。
        // 大きさが拡大・縮小・回転で変わらず、常に画面の正面を向く。
        out.color = in.color;
        let c = u.view_proj * vec4<f32>(in.position, 1.0);
        let ndc = c.xy / c.w + in.aux.xy / (u.viewport.xy * 0.5);
        out.clip_position = vec4<f32>(ndc * c.w, min(c.z * (1.0 + BILLBOARD_DEPTH_BIAS), c.w), c.w);
        return out;
    }
    let width_px = in.params.x;
    if (width_px <= 0.0) {
        out.clip_position = u.view_proj * vec4<f32>(in.position, 1.0);
        var shade = 1.0;
        if (in.params.w > 0.5) {
            shade = AMBIENT + (1.0 - AMBIENT) * max(dot(normalize(in.aux), u.light.xyz), 0.0);
        }
        out.color = vec4<f32>(in.color.rgb * shade, in.color.a);
        return out;
    }

    // 線: この頂点は線分の端点(position)にあり、反対側の端点(aux)との向きで帯を広げる。
    out.color = in.color;
    let this_clip = u.view_proj * vec4<f32>(in.position, 1.0);
    let other_clip = u.view_proj * vec4<f32>(in.aux, 1.0);
    var a = this_clip;
    var b = other_clip;
    if (a.w < LINE_MIN_W) {
        if (b.w < LINE_MIN_W) {
            // 両端ともカメラの後ろ: 何も描かない(クリップ空間の外に出す)。
            out.clip_position = vec4<f32>(0.0, 0.0, -1.0, 1.0);
            return out;
        }
        a = mix(this_clip, other_clip, (LINE_MIN_W - this_clip.w) / (other_clip.w - this_clip.w));
    } else if (b.w < LINE_MIN_W) {
        b = mix(other_clip, this_clip, (LINE_MIN_W - other_clip.w) / (this_clip.w - other_clip.w));
    }

    // 画面のピクセル単位で、線の向き(反対側→この端点)と、その左手の法線を求める。
    let half = u.viewport.xy * 0.5;
    let pa = a.xy / a.w * half;
    let pb = b.xy / b.w * half;
    var dir = pa - pb;
    let len = length(dir);
    if (len > 1.0e-4) {
        dir = dir / len;
    } else {
        dir = vec2<f32>(1.0, 0.0);
    }
    let normal = vec2<f32>(-dir.y, dir.x);
    let half_width = 0.5 * width_px;
    // 幅方向に±半幅、端点は線の向きへ半幅だけ延ばす(折れ線のつなぎ目の隙間を埋める)。
    let offset_px = normal * (in.params.y * half_width) + dir * half_width;
    let ndc = a.xy / a.w + offset_px / half;
    let z = min(a.z * (1.0 + LINE_DEPTH_BIAS), a.w);
    out.clip_position = vec4<f32>(ndc * a.w, z, a.w);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
