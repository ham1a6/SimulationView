//! 視錐台カリング。地形メッシュ1個ごとの外接直方体が、視錐台の外にあれば描かない。

use crate::terrain::mesh::TerrainVertex;

/// 頂点位置を囲む直方体(最小の角, 最大の角)。
pub(super) fn position_bounds(vertices: &[TerrainVertex]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for v in vertices {
        for axis in 0..3 {
            min[axis] = min[axis].min(v.position[axis]);
            max[axis] = max[axis].max(v.position[axis]);
        }
    }
    (min, max)
}

/// 直方体が視錐台(クリップ空間の-w<=x<=w, -w<=y<=w, 0<=z<=w)の完全に外にあるか。8つの角が
/// すべて同じ面の外側にあれば、直方体全体がその面の外にある(GPUのクリッピングでも何も描かれない
/// ので、この判定で描画を省いても見た目は変わらない)。判定は保守的で、外にあるのに「外でない」と
/// 判定することはあっても、見えているものを「外」とすることはない。
pub(super) fn is_outside_frustum(view_proj: &glam::Mat4, bounds: ([f32; 3], [f32; 3])) -> bool {
    let (min, max) = bounds;
    // 6面それぞれについて「全部の角が外側」を表すビットを、角ごとのビットとの論理積で求める。
    let mut all_outside = 0b11_1111u8;
    for corner in 0..8 {
        let p = glam::Vec4::new(
            if corner & 1 == 0 { min[0] } else { max[0] },
            if corner & 2 == 0 { min[1] } else { max[1] },
            if corner & 4 == 0 { min[2] } else { max[2] },
            1.0,
        );
        let c = *view_proj * p;
        let mut outside = 0u8;
        outside |= (c.x < -c.w) as u8;
        outside |= ((c.x > c.w) as u8) << 1;
        outside |= ((c.y < -c.w) as u8) << 2;
        outside |= ((c.y > c.w) as u8) << 3;
        outside |= ((c.z < 0.0) as u8) << 4;
        outside |= ((c.z > c.w) as u8) << 5;
        all_outside &= outside;
        if all_outside == 0 {
            return false;
        }
    }
    all_outside != 0
}
