//! 地形データ範囲の判定(`max_valid_distance`)。元々は断面図(cross-section)パネル
//! (`components/cross_section_view.rs`、方位角方向への地表サンプリングに使用)向けの
//! モジュールだったが、断面図パネル自体を削除した際にこの関数だけ`terrain::los`が
//! 共有していたため残した(「ボトムステータスパネルの側面図もいらない」との要望、
//! git履歴参照)。

use super::loader::TerrainData;
use super::mesh::EnuTransform;

/// 二分探索で「地形データの範囲内かどうか」を調べる際の初期上限距離。
/// 地形データの外接矩形(5度四方、対角線で約780km)より確実に大きい値にしておく。
const SEARCH_UPPER_BOUND_M: f64 = 1_000_000.0;

/// 方位角方向(東=dir_east, 北=dir_north の単位ベクトル)に、地形データの範囲内でいられる
/// 最大距離を二分探索で求める。原点自体は必ず範囲内にある前提(サーバー側の`set_origin`
/// バリデーション済み)。`terrain::los`(見通し範囲)でも使うため`pub(super)`にしてある。
pub(super) fn max_valid_distance(
    data: &TerrainData,
    transform: &EnuTransform,
    dir_east: f64,
    dir_north: f64,
) -> f64 {
    let in_bounds = |distance: f64| -> bool {
        let (lat, lon) = transform.inverse(dir_east * distance, dir_north * distance);
        let b = &data.metadata.geodetic_bounds;
        lat >= b.min_lat && lat <= b.max_lat && lon >= b.min_lon && lon <= b.max_lon
    };

    let mut lo = 0.0_f64;
    let mut hi = SEARCH_UPPER_BOUND_M;
    if in_bounds(hi) {
        return hi;
    }
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if in_bounds(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}
