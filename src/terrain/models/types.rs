//! 3Dモデルの描画用データ(GPUへ渡す頂点・インスタンス)。`model.wgsl`の頂点入力と対で、並び・大きさが
//! 一致することは`renderer`の単体テスト(naga)が確認する。

use bytemuck::{Pod, Zeroable};

/// 3Dモデルの頂点。座標・法線は**機体座標**(x=右、y=前、z=上。単位はメートル、原点はモデルの基準点)。
/// glTFの座標(+Y上・+Z前)からの変換は`gltf_import`が読み込み時に行う。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct ModelVertex {
    /// 位置(機体座標、メートル)。
    pub position: [f32; 3],
    /// 単位法線(機体座標)。陰影に使う。
    pub normal: [f32; 3],
    /// 頂点色×マテリアルの基本色(リニア。アルファは常に1)。
    pub color: [f32; 4],
}

/// 3Dモデルのインスタンス(同じモデルを何機も描くときの、1機ごとの違い)。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct ModelInstance {
    /// 機体座標→ENU座標(地形メッシュの原点基準)の変換行列(列優先。位置・向き・大きさを含む)。
    pub model: [[f32; 4]; 4],
    /// rgb: 所属の色。a: モデルの色へ混ぜる割合(0で混ぜない)。
    pub tint: [f32; 4],
}

/// 読み込み済みのモデル1つ(CPU側。GPUへの転送は`TerrainRenderer::set_model`)。
#[derive(Debug, Clone, PartialEq)]
pub struct ModelMesh {
    /// 頂点(ノードの変換・マテリアルの色を焼き込み済み)。
    pub vertices: Vec<ModelVertex>,
    /// 三角形リスト(3つで1枚)。
    pub indices: Vec<u32>,
    /// 基準点から最も遠い頂点までの距離(メートル)。「最小画面サイズ」の大きさの見積もりに使う。
    pub radius_m: f32,
}
