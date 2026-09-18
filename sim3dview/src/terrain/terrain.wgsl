struct CameraUniform {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: CameraUniform;

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
    return vec4<f32>(in.color, 1.0);
}

// 見通し範囲の覆域ドーム(半球状の面)用。地形やマーカーを透けて見せたいため、
// 固定の半透明アルファで出力する(頂点データ自体は他のパイプラインと共用のposition+colorのまま)。
@fragment
fn fs_dome(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 0.22);
}
