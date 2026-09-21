// 3Dモデル(`terrain::models`)の描画シェーダー。モデルの頂点(機体座標)にインスタンスごとの変換行列を掛けてENU座標へ置き、
// カメラのview_projで射影する。陰影は頂点ごとのランバート(環境光+光源1つ)。
// uniformは作図(`draw.wgsl`)の絶対座標用と同じもの(`DrawUniform`。view_proj・光源だけ使う)を共有する。
// 頂点データは`terrain::models::types::{ModelVertex, ModelInstance}`。

struct DrawUniform {
    view_proj: mat4x4<f32>,
    viewport: vec4<f32>,
    // xyz: 光源の向き(面から光源へ向かう単位ベクトル。ENU座標)。
    light: vec4<f32>,
};
@group(0) @binding(0)
var<uniform> u: DrawUniform;

struct VertexInput {
    // 頂点ごと(機体座標)。
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    // インスタンスごと: 機体座標→ENU座標の行列(4列)と、所属の色(rgb)・混ぜる割合(a)。
    @location(3) model_0: vec4<f32>,
    @location(4) model_1: vec4<f32>,
    @location(5) model_2: vec4<f32>,
    @location(6) model_3: vec4<f32>,
    @location(7) tint: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

// 光が当たらない面の明るさの下限(0〜1)。
const AMBIENT = 0.4;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    out.clip_position = u.view_proj * (model * vec4<f32>(in.position, 1.0));
    // 拡大縮小は一様なので、法線は行列の回転成分を掛けて正規化するだけでよい。
    let normal = normalize((model * vec4<f32>(in.normal, 0.0)).xyz);
    let shade = AMBIENT + (1.0 - AMBIENT) * max(dot(normal, u.light.xyz), 0.0);
    let base = mix(in.color.rgb, in.tint.rgb, in.tint.a);
    out.color = vec4<f32>(base * shade, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
