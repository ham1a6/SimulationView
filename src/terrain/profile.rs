//! 断面図(cross-section)生成。断面図パネル(`ui::cross_section_view`)で、中心の点(選択中のシンボルの位置、
//! 何も選択されていなければ原点)を通る、指定した方位角の直線に沿って地表をサンプリングし、
//! (距離, 標高)の点列を作る(2D表示用)。距離は中心が0で、方位角の向きが正、その反対が負。
//! `max_valid_distance`は`terrain::los`(見通し範囲)とも共有している。

use super::geodesy::EnuTransform;
use super::heightmap::sample_heightmap;
use super::loader::TerrainData;
use super::origin::Origin;
pub struct ProfilePoint {
    pub distance_m: f64,
    pub elevation_m: f32,
    pub lat_deg: f64,
    pub lon_deg: f64,
}

/// 方位角方向1本につき何点サンプリングするか。
const NUM_SAMPLES: usize = 300;
/// 断面図・見通し範囲が扱う最大距離(メートル)。二分探索の初期上限でもあり、この距離でもまだ
/// 地形データの範囲内なら、範囲の端まで届かなくてもここで打ち切る(地形データ全体は30度四方で
/// 約3,300kmあるが、原点からこれより遠くは扱わない)。
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

/// 原点から方位角(度、北=0・東=90・時計回り)方向へ、地形データの範囲内(最大で
/// `SEARCH_UPPER_BOUND_M`)まで地表をサンプリングする(片側だけ)。原点変更・方位角変更のたびに呼び直す想定。
/// (片側だけの断面は、単体テストで`build_profile_span`との一致を確かめるために残してある。)
#[cfg(test)]
pub fn build_profile(data: &TerrainData, origin: &Origin, azimuth_deg: f64) -> Vec<ProfilePoint> {
    build_profile_span(data, origin, azimuth_deg, 0.0, SEARCH_UPPER_BOUND_M)
}

/// `center`を通る、方位角(度、北=0・東=90・時計回り)の直線に沿って、地形データの範囲内で、
/// 方位角の向きに`forward_m`、その反対に`back_m`まで(それぞれ地形データの端で打ち切る)地表をサンプリングする。
/// 点の`distance_m`は中心が0で、方位角の向きが正・反対が負(先頭が`-back`、末尾が`+forward`)。
/// 断面図で、選択中のシンボルの位置を中心にするために使う。
pub fn build_profile_span(
    data: &TerrainData,
    center: &Origin,
    azimuth_deg: f64,
    back_m: f64,
    forward_m: f64,
) -> Vec<ProfilePoint> {
    let transform = EnuTransform::new(center, &data.metadata.ellipsoid);
    let az_rad = azimuth_deg.to_radians();
    let dir_east = az_rad.sin();
    let dir_north = az_rad.cos();

    let forward = max_valid_distance(data, &transform, dir_east, dir_north).min(forward_m.max(0.0));
    let back = if back_m > 0.0 {
        max_valid_distance(data, &transform, -dir_east, -dir_north).min(back_m)
    } else {
        0.0
    };
    let total = forward + back;
    if total <= 0.0 {
        return Vec::new();
    }

    (0..=NUM_SAMPLES)
        .map(|i| {
            let distance = -back + total * (i as f64) / (NUM_SAMPLES as f64);
            let (lat, lon) = transform.inverse(dir_east * distance, dir_north * distance);
            let elevation = sample_heightmap(data, lat, lon).unwrap_or(0.0);
            ProfilePoint {
                distance_m: distance,
                elevation_m: elevation,
                lat_deg: lat,
                lon_deg: lon,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::geodesy::Ellipsoid;
    fn transform(lat: f64, lon: f64) -> EnuTransform {
        EnuTransform::new(
            &Origin {
                lat_deg: lat,
                lon_deg: lon,
            },
            &Ellipsoid::WGS84,
        )
    }

    /// 東へ1度で600m上がる斜面(タイル(30,120)用。ノード間隔1/6度で整数になる)。
    fn east_slope(_lat: f64, lon: f64) -> i16 {
        (600.0 * (lon - 120.0)).round() as i16
    }

    #[test]
    fn max_valid_distance_stops_at_the_data_edge() {
        let data = TerrainData::synthetic(30, 120, 1, 1, east_slope);
        let t = transform(30.5, 120.5);
        // 東へ0.5度(緯度30.5での経度1度は約96km)、北へ0.5度(約55km)。
        let east = max_valid_distance(&data, &t, 1.0, 0.0);
        let north = max_valid_distance(&data, &t, 0.0, 1.0);
        assert!((east - 48_000.0).abs() < 1_500.0, "east={east}");
        assert!((north - 55_400.0).abs() < 1_000.0, "north={north}");
    }

    #[test]
    fn max_valid_distance_is_capped_at_the_search_bound() {
        // 東へ15度(約1,600km)先まで地形がある場合でも、1,000kmで打ち切る。
        let data = TerrainData::synthetic(0, 100, 30, 30, |_, _| 0);
        let t = transform(15.0, 115.0);
        assert_eq!(
            max_valid_distance(&data, &t, 1.0, 0.0),
            SEARCH_UPPER_BOUND_M
        );
    }

    /// 中心を通る断面: 中心の位置が距離0になり、方位角の向きが正・反対が負。片側が地形データの端に近ければ、その側だけ短くなる。
    #[test]
    fn centered_profile_passes_through_the_center_and_stops_at_the_data_edge() {
        let data = TerrainData::synthetic(30, 120, 1, 1, east_slope);
        // 東端(経度121度)まで約24km、西端まで約72kmの点。東西に±100km取ろうとしても、データの端で切れる。
        let center = Origin {
            lat_deg: 30.5,
            lon_deg: 120.75,
        };
        let p = build_profile_span(&data, &center, 90.0, 100_000.0, 100_000.0);
        assert_eq!(p.len(), NUM_SAMPLES + 1);
        let (first, last) = (p[0].distance_m, p[NUM_SAMPLES].distance_m);
        assert!((first + 72_000.0).abs() < 2_000.0, "西端 {first}");
        assert!((last - 24_000.0).abs() < 1_000.0, "東端 {last}");
        // 距離0の点は、中心の標高(東へ1度で600m上がる斜面の、経度120.75度=450m)。
        let at_center = p
            .iter()
            .min_by(|a, b| a.distance_m.abs().total_cmp(&b.distance_m.abs()))
            .unwrap();
        assert!(at_center.distance_m.abs() < (last - first) / NUM_SAMPLES as f64);
        assert!(
            (at_center.elevation_m - 450.0).abs() < 25.0,
            "{}",
            at_center.elevation_m
        );
        // 距離は単調増加で等間隔。
        let step = p[1].distance_m - p[0].distance_m;
        assert!(p
            .windows(2)
            .all(|w| ((w[1].distance_m - w[0].distance_m) - step).abs() < 1e-6));
        // 東へ進むほど高い。
        assert!(p.windows(2).all(|w| w[1].elevation_m >= w[0].elevation_m));
        // 範囲を狭くすると、その範囲で切れる。
        let narrow = build_profile_span(&data, &center, 90.0, 10_000.0, 5_000.0);
        assert!((narrow[0].distance_m + 10_000.0).abs() < 1e-6);
        assert!((narrow[NUM_SAMPLES].distance_m - 5_000.0).abs() < 1e-6);
        // 片側だけ(back=0)は、従来の`build_profile`と同じ。
        let one_way = build_profile_span(&data, &center, 90.0, 0.0, SEARCH_UPPER_BOUND_M);
        let old = build_profile(&data, &center, 90.0);
        assert_eq!(one_way.len(), old.len());
        assert!(one_way
            .iter()
            .zip(&old)
            .all(|(a, b)| a.distance_m == b.distance_m));
    }

    #[test]
    fn profile_samples_evenly_from_the_origin_to_the_edge() {
        let data = TerrainData::synthetic(30, 120, 1, 1, east_slope);
        let origin = Origin {
            lat_deg: 30.5,
            lon_deg: 120.5,
        };
        let east = build_profile(&data, &origin, 90.0);
        assert_eq!(east.len(), NUM_SAMPLES + 1);
        assert_eq!(east[0].distance_m, 0.0);
        assert!((east[0].elevation_m - 300.0).abs() < 1.0); // 原点の標高
                                                            // 東へ進むほど高くなり、最後は東端(経度121度=600m)付近。
        assert!(east
            .windows(2)
            .all(|w| w[1].elevation_m >= w[0].elevation_m));
        assert!(
            (east[NUM_SAMPLES].elevation_m - 600.0).abs() < 25.0,
            "{}",
            east[NUM_SAMPLES].elevation_m
        );
        // 等間隔。
        let step = east[1].distance_m;
        assert!((east[NUM_SAMPLES].distance_m - step * NUM_SAMPLES as f64).abs() < 1e-6);
        // 北向きは東西方向に傾斜のない斜面の上なので標高が変わらない。
        let north = build_profile(&data, &origin, 0.0);
        assert!(north.iter().all(|p| (p.elevation_m - 300.0).abs() < 25.0));
    }
}
