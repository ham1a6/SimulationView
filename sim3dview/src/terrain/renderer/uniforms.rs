//! シェーダーへ渡すuniform・頂点属性の定義(`terrain.wgsl`・`draw.wgsl`と対。並び・大きさが一致することは
//! `renderer`の単体テスト(naga)が確認する)。

use bytemuck::{Pod, Zeroable};

/// 作図(`terrain::drawing`)用のuniform(`draw.wgsl`の`DrawUniform`)。座標の種類ごとに1つ持つ。
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct DrawUniform {
    pub view_proj: [[f32; 4]; 4],
    /// x,y: 描画先(canvas)の大きさ(px)。
    pub viewport: [f32; 4],
    /// xyz: 光源の向き(面から光源へ向かう単位ベクトル)。
    pub light: [f32; 4],
}

/// 絶対座標の作図の光源。地形の陰影(`terrain.wgsl`の`LIGHT_DIR`)と同じ、北西・仰角45度(ENU座標)。
pub(super) const WORLD_DRAW_LIGHT: [f32; 4] = [-0.5, 0.5, 0.707_106_8, 0.0];
/// カメラ固定(視点空間)の作図の光源。カメラから見て左上手前から当てる(x右・y上・z手前)。
pub(super) const VIEW_DRAW_LIGHT: [f32; 4] = [-0.348, 0.497, 0.795, 0.0];

/// 画面のピクセル座標(左上原点、x右・y下)をクリップ空間へ写す行列(作図のカメラ固定・画面座標用)。
/// 深度は一定(0.5)。深度テストは使わず、描く順で重ねる。
pub(super) fn screen_matrix(width: f32, height: f32) -> glam::Mat4 {
    glam::Mat4::from_cols(
        glam::Vec4::new(2.0 / width, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, -2.0 / height, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, 0.0, 0.0),
        glam::Vec4::new(-1.0, 1.0, 0.5, 1.0),
    )
}

/// 地形・水域・ドームのシェーダー(`terrain.wgsl`)が読むカメラのuniform。
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    /// x: 陰影(ヒルシェード)を付けるなら1、付けないなら0(`terrain.wgsl`の`camera.shading`)。
    pub shading: [f32; 4],
    /// 水域レイヤーの視線の計算用(`Camera::water_ray_basis`): 視点(w=透視なら1)・視線方向・右・上。
    pub eye: [f32; 4],
    pub forward: [f32; 4],
    pub right: [f32; 4],
    pub up: [f32; 4],
    /// WGS84楕円体(海抜0m)の陰関数の係数(`EnuTransform::ellipsoid_shader_params`)。
    pub ellipsoid_m: [[f32; 4]; 3],
    pub ellipsoid_g: [f32; 4],
}

impl CameraUniform {
    /// 描画前の初期値(バッファを作るためだけの値。毎フレーム`render`が書き直す)。
    pub(super) fn initial() -> Self {
        Self {
            view_proj: glam::Mat4::IDENTITY.to_cols_array_2d(),
            shading: [0.0; 4],
            eye: [0.0; 4],
            forward: [0.0, 0.0, -1.0, 0.0],
            right: [1.0, 0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0, 0.0],
            ellipsoid_m: [[0.0; 4]; 3],
            ellipsoid_g: [0.0; 4],
        }
    }
}

/// 地形の頂点(`TerrainVertex`)のシェーダー入力(`terrain.wgsl`の`VertexInput`)。位置・色・法線xy(snorm16x2)。
/// オフセットは`vertex_attr_array!`が並びから求める(構造体のレイアウトと一致することは単体テストで確認)。
pub(super) const TERRAIN_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Snorm16x2];

/// 作図の頂点(`DrawVertex`)のシェーダー入力(`draw.wgsl`の`VertexInput`)。位置・色・aux・params。
pub(super) const DRAW_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 4] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4, 2 => Float32x3, 3 => Float32x4];

/// uniformバッファ1つだけのbind groupのレイアウト(`visibility`で、どのシェーダーステージが読むかを指定する)。
pub(super) fn uniform_layout(
    device: &wgpu::Device,
    label: &str,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// `uniform_layout`のレイアウトで、バッファ全体を束縛するbind group。
pub(super) fn uniform_bind_group(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
    })
}
