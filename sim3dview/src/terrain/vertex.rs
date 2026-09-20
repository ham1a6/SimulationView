//! 作図・航跡・マーカーが共通で使う描画用の頂点(`draw.wgsl`の`VertexInput`と対)。

/// `DrawVertex::params.z`の値(頂点の種類)。`draw.wgsl`の`vs_main`が`> 0.5`・`> 1.5`で見分けるので、
/// 値を変えるときはシェーダーも合わせること(型は単体テストのWGSL検証が見ている)。
/// 面・線(ワールド/視点空間/画面の座標そのまま)。
pub(crate) const KIND_FLAT: f32 = 0.0;
/// 画面サイズ固定のビルボード(マーカー)。
pub(crate) const KIND_BILLBOARD: f32 = 1.0;
/// 向きつきビルボード(シンボル。進行方向が画面のどちらを向くかに合わせて回す)。
pub(crate) const KIND_ORIENTED_BILLBOARD: f32 = 2.0;

/// 描画用の頂点。面と線(太さ付き)を同じ頂点形式・同じパイプラインで描く(`draw.wgsl`)。
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DrawVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
    /// 面: 単位法線(陰影を付けるとき)。線: 反対側の端点。
    pub aux: [f32; 3],
    /// x: 線の太さ(px。0なら面)。y: 線の側(-1/+1)。z: ビルボードの種類(0=面・線、1=画面サイズ固定のマーカー、
    /// 2=向きつきシンボル。`draw.wgsl`の`vs_main`が見る)。w: 陰影を付けるなら1(面のみ)。
    pub params: [f32; 4],
}

impl DrawVertex {
    pub(crate) fn surface(position: [f32; 3], color: [f32; 4], normal: Option<[f32; 3]>) -> Self {
        match normal {
            Some(aux) => Self { position, color, aux, params: [0.0, 0.0, KIND_FLAT, 1.0] },
            None => Self { position, color, aux: [0.0; 3], params: [0.0, 0.0, KIND_FLAT, 0.0] },
        }
    }

    pub(crate) fn line(position: [f32; 3], other: [f32; 3], color: [f32; 4], width_px: f32, side: f32) -> Self {
        Self { position, color, aux: other, params: [width_px, side, KIND_FLAT, 0.0] }
    }

    /// ビルボード(画面サイズ固定のマーカー)の頂点。`anchor`は3D空間の位置、`offset_px`はそこからの
    /// 画面上のずれ(px、右・上が正)。拡大・縮小しても大きさが変わらず、常に画面の正面を向く。
    pub(crate) fn billboard(anchor: [f32; 3], offset_px: [f32; 2], color: [f32; 4]) -> Self {
        Self { position: anchor, color, aux: [offset_px[0], offset_px[1], 0.0], params: [0.0, 0.0, KIND_BILLBOARD, 0.0] }
    }

    /// 向きつきビルボード(`billboard`と同じだが、`offset_px`を「進行方向が画面のどちらを向くか」に合わせて回す)。
    /// `offset_px`は進行方向が上(+y)・その右が+xの座標で、`heading_rad`は北から時計回りの進行方向(ENU座標の水平)。
    /// 画面上の向きは、シェーダーがアンカーとアンカーから進行方向へ少し進んだ点を射影して求める
    /// (3Dでカメラを回しても、2Dの地図でも、シンボルの向きが実際の進行方向を指す)。
    pub(crate) fn oriented_billboard(
        anchor: [f32; 3],
        offset_px: [f32; 2],
        heading_rad: f32,
        color: [f32; 4],
    ) -> Self {
        Self {
            position: anchor,
            color,
            aux: [offset_px[0], offset_px[1], 0.0],
            params: [heading_rad, 0.0, KIND_ORIENTED_BILLBOARD, 0.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // シェーダーは`params.z`を`> 0.5`(ビルボード)・`> 1.5`(向きつき)で見分けるので、定数がその閾値の
    // 正しい側に入っていること。
    #[test]
    fn kinds_are_told_apart_by_the_shader_thresholds() {
        let shader = include_str!("draw.wgsl");
        assert!(shader.contains("in.params.z > 1.5") && shader.contains("in.params.z > 0.5"));
        assert!(KIND_FLAT <= 0.5);
        assert!(KIND_BILLBOARD > 0.5 && KIND_BILLBOARD <= 1.5);
        assert!(KIND_ORIENTED_BILLBOARD > 1.5);
    }

    #[test]
    fn constructors_set_the_kind() {
        assert_eq!(DrawVertex::billboard([0.0; 3], [0.0; 2], [0.0; 4]).params[2], KIND_BILLBOARD);
        assert_eq!(DrawVertex::oriented_billboard([0.0; 3], [0.0; 2], 1.0, [0.0; 4]).params[2], KIND_ORIENTED_BILLBOARD);
        assert_eq!(DrawVertex::surface([0.0; 3], [0.0; 4], None).params[2], KIND_FLAT);
        assert_eq!(DrawVertex::line([0.0; 3], [1.0; 3], [0.0; 4], 2.0, 1.0).params[2], KIND_FLAT);
    }
}
