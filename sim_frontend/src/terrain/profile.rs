//! 断面図(cross-section)生成。側面図パネルで、原点を起点に指定した方位角方向へ
//! 地表をサンプリングし、(距離, 標高)の点列を作る(2D表示用、`components/cross_section_view.rs`)。

use super::loader::TerrainData;
use super::mesh::{sample_heightmap, EnuTransform, Origin};

pub struct ProfilePoint {
    pub distance_m: f64,
    pub elevation_m: f32,
}

/// 方位角方向1本につき何点サンプリングするか。
const NUM_SAMPLES: usize = 300;
/// 二分探索で「地形データの範囲内かどうか」を調べる際の初期上限距離。
/// 地形データの外接矩形(5度四方、対角線で約780km)より確実に大きい値にしておく。
const SEARCH_UPPER_BOUND_M: f64 = 1_000_000.0;

/// 方位角方向(東=dir_east, 北=dir_north の単位ベクトル)に、地形データの範囲内でいられる
/// 最大距離を二分探索で求める。原点自体は必ず範囲内にある前提(サーバー側の`set_origin`
/// バリデーション済み)。
fn max_valid_distance(
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

/// 原点から方位角(度、北=0・東=90・時計回り)方向へ、地形データの範囲内いっぱいまで
/// 地表をサンプリングする。原点変更・方位角変更のたびに呼び直す想定。
pub fn build_profile(data: &TerrainData, origin: &Origin, azimuth_deg: f64) -> Vec<ProfilePoint> {
    let transform = EnuTransform::new(origin, &data.metadata.ellipsoid);
    let az_rad = azimuth_deg.to_radians();
    let dir_east = az_rad.sin();
    let dir_north = az_rad.cos();

    let max_distance = max_valid_distance(data, &transform, dir_east, dir_north);
    if max_distance <= 0.0 {
        return Vec::new();
    }

    (0..=NUM_SAMPLES)
        .map(|i| {
            let distance = max_distance * (i as f64) / (NUM_SAMPLES as f64);
            let (lat, lon) = transform.inverse(dir_east * distance, dir_north * distance);
            let elevation = sample_heightmap(data, lat, lon).unwrap_or(0.0);
            ProfilePoint { distance_m: distance, elevation_m: elevation }
        })
        .collect()
}
