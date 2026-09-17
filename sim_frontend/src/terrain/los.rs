//! 見通し範囲(レーダー探知可能範囲)の計算。指定したポイント(原点)から全方位角へ
//! 地表をサンプリングし、地形による遮蔽と地球曲率(等価地球半径)を考慮した上で、
//! 各方位角ごとの見通し限界距離を求める。`components/los_view.rs`から使う。

use super::loader::TerrainData;
use super::mesh::{sample_heightmap, EnuTransform, Origin};
use super::profile::max_valid_distance;

/// 平均地球半径(メートル)。
const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// 等価地球半径係数(k-factor)。標準大気中では電波が幾何学的な直線よりわずかに
/// 下向きに屈折するため、実際の見通し距離は真球上の幾何学的な地平線より遠くなる。
/// この効果を「実際の地球半径をk倍した仮想的に大きい球面上で電波が直進する」近似で
/// 表したものが等価地球半径で、標準大気(標準的な気温減率)ではk=4/3が広く使われる
/// (レーダー・無線工学の標準的な近似。実際の大気状態によって変動するが、v1では固定値とする)。
const K_FACTOR: f64 = 4.0 / 3.0;

/// 計算する方位角の刻み数(1度刻み=360方向)。
const NUM_AZIMUTHS: usize = 360;
/// 1方位角あたりのサンプル点数。
const SAMPLES_PER_RAY: usize = 200;

/// 見通し範囲計算のパラメータ。
pub struct LosParams {
    /// 観測点の地表からのアンテナ高(メートル)。
    pub observer_height_m: f64,
    /// 最大観測範囲(メートル)。地形データの範囲とこの値のうち小さい方まで計算する。
    pub max_range_m: f64,
}

/// 1方位角ぶんの見通し範囲計算結果。
pub struct LosPoint {
    /// 方位角(度、北=0・東=90・時計回り)。
    pub azimuth_deg: f64,
    /// その方位角での見通し限界距離(メートル)。
    pub range_m: f64,
}

/// 距離`d`(メートル)における、等価地球半径`r_eff`による地球曲率分の見かけの高度低下量。
/// 標準的な近似式(距離に対して地球半径が十分大きいことを前提とした2次近似)。
fn curvature_drop_m(distance_m: f64, r_eff_m: f64) -> f64 {
    (distance_m * distance_m) / (2.0 * r_eff_m)
}

/// 指定したポイント(原点)を中心に、全方位角の見通し範囲を計算する。
/// 原点変更・パラメータ変更のたびに呼び直す想定(`cross_section_view`と同じ反応性)。
///
/// アルゴリズム(方位角ごと): 観測点から外側へサンプリングしながら、各サンプル点の
/// 「見かけの仰角」(地球曲率による低下を差し引いた角度)を計算する。手前の地形で
/// それまでの最大仰角より低い角度の点は、手前の地形に遮蔽されて見えない。見える点
/// (それまでの最大仰角以上の点)のうち最も遠いものの距離を、その方位角の見通し
/// 限界距離とする(手前の尾根の陰でも、その先で地形が十分高くなれば再び見える
/// ケースを許容する。単純な「最初の遮蔽物で打ち切り」より実際のレーダー覆域に近い)。
pub fn compute_los(data: &TerrainData, origin: &Origin, params: &LosParams) -> Vec<LosPoint> {
    let transform = EnuTransform::new(origin, &data.metadata.ellipsoid);
    let observer_ground_elevation =
        sample_heightmap(data, origin.lat_deg, origin.lon_deg).unwrap_or(0.0) as f64;
    let observer_height = observer_ground_elevation + params.observer_height_m;
    let r_eff = EARTH_RADIUS_M * K_FACTOR;

    (0..NUM_AZIMUTHS)
        .map(|az_i| {
            let azimuth_deg = az_i as f64 * 360.0 / NUM_AZIMUTHS as f64;
            let az_rad = azimuth_deg.to_radians();
            let dir_east = az_rad.sin();
            let dir_north = az_rad.cos();

            let data_max = max_valid_distance(data, &transform, dir_east, dir_north);
            let ray_max = data_max.min(params.max_range_m);
            if ray_max <= 0.0 {
                return LosPoint { azimuth_deg, range_m: 0.0 };
            }

            let mut max_angle = f64::NEG_INFINITY;
            let mut visible_range = 0.0_f64;
            for i in 1..=SAMPLES_PER_RAY {
                let d = ray_max * i as f64 / SAMPLES_PER_RAY as f64;
                let (lat, lon) = transform.inverse(dir_east * d, dir_north * d);
                let elevation = sample_heightmap(data, lat, lon).unwrap_or(0.0) as f64;
                let apparent_height = elevation - curvature_drop_m(d, r_eff);
                let angle = (apparent_height - observer_height) / d;
                if angle >= max_angle {
                    max_angle = angle;
                    visible_range = d;
                }
            }
            LosPoint { azimuth_deg, range_m: visible_range }
        })
        .collect()
}

/// 単一の観測点(レーダー)から単一の対象地点への見通し(手前の地形に遮蔽されないか)を
/// 判定する。`compute_los`と同じマスク角アルゴリズムを、対象地点までの1本のレイに絞って
/// 適用したもの。断面図の覆域表示(`components/cross_section_view.rs`)で、断面上の各点が
/// いずれかのレーダーから見えるかを判定するために使う。
pub fn is_visible(
    data: &TerrainData,
    radar_lat_deg: f64,
    radar_lon_deg: f64,
    radar_height_m: f64,
    max_range_m: f64,
    target_lat_deg: f64,
    target_lon_deg: f64,
) -> bool {
    let radar_origin = Origin { lat_deg: radar_lat_deg, lon_deg: radar_lon_deg };
    let transform = EnuTransform::new(&radar_origin, &data.metadata.ellipsoid);
    let observer_ground_elevation =
        sample_heightmap(data, radar_lat_deg, radar_lon_deg).unwrap_or(0.0) as f64;
    let observer_height = observer_ground_elevation + radar_height_m;
    let r_eff = EARTH_RADIUS_M * K_FACTOR;

    let target_pos = transform.transform(target_lat_deg, target_lon_deg, 0.0);
    let target_distance = ((target_pos[0] as f64).powi(2) + (target_pos[1] as f64).powi(2)).sqrt();
    if target_distance < 1.0 {
        return true;
    }
    if target_distance > max_range_m {
        return false;
    }
    let dir_east = target_pos[0] as f64 / target_distance;
    let dir_north = target_pos[1] as f64 / target_distance;

    let target_elevation = sample_heightmap(data, target_lat_deg, target_lon_deg).unwrap_or(0.0) as f64;
    let target_angle =
        (target_elevation - curvature_drop_m(target_distance, r_eff) - observer_height) / target_distance;

    let samples = ((target_distance / 500.0).ceil() as usize).clamp(10, SAMPLES_PER_RAY);
    for i in 1..samples {
        let d = target_distance * i as f64 / samples as f64;
        let (lat, lon) = transform.inverse(dir_east * d, dir_north * d);
        let elevation = sample_heightmap(data, lat, lon).unwrap_or(0.0) as f64;
        let angle = (elevation - curvature_drop_m(d, r_eff) - observer_height) / d;
        if angle > target_angle {
            return false;
        }
    }
    true
}

/// 単一の観測点(レーダー)から見て、指定した地表座標の真上で「これ以上の高度(標高、m)なら
/// 見える」という下限高度を求める(等価地球半径による曲率・手前の地形によるマスク角を考慮)。
/// `is_visible`が対象の実標高との比較で真偽を返すのに対し、こちらは仰角の式が対象の高度に
/// ついて線形であることを利用し、可視となる最小高度を直接逆算する。断面図の覆域表示
/// (`components/cross_section_view.rs`)で、地表だけでなく上空を含めた覆域を図示するために使う。
/// 対象地点までの距離がレーダーの最大観測範囲を超える場合はNone(どんな高度でも覆域外)。
pub fn min_visible_altitude(
    data: &TerrainData,
    radar_lat_deg: f64,
    radar_lon_deg: f64,
    radar_height_m: f64,
    max_range_m: f64,
    target_lat_deg: f64,
    target_lon_deg: f64,
) -> Option<f64> {
    let radar_origin = Origin { lat_deg: radar_lat_deg, lon_deg: radar_lon_deg };
    let transform = EnuTransform::new(&radar_origin, &data.metadata.ellipsoid);
    let observer_ground_elevation =
        sample_heightmap(data, radar_lat_deg, radar_lon_deg).unwrap_or(0.0) as f64;
    let observer_height = observer_ground_elevation + radar_height_m;
    let r_eff = EARTH_RADIUS_M * K_FACTOR;

    let target_pos = transform.transform(target_lat_deg, target_lon_deg, 0.0);
    let target_distance = ((target_pos[0] as f64).powi(2) + (target_pos[1] as f64).powi(2)).sqrt();
    if target_distance > max_range_m {
        return None;
    }
    if target_distance < 1.0 {
        return Some(observer_height);
    }
    let dir_east = target_pos[0] as f64 / target_distance;
    let dir_north = target_pos[1] as f64 / target_distance;

    // 観測点から対象地点までの間の地形が作る最大の見かけ仰角(=対象が見えるために
    // 必要な最低仰角)を求める。target自身の標高には依存しない点がis_visibleと異なる。
    let samples = ((target_distance / 500.0).ceil() as usize).clamp(10, SAMPLES_PER_RAY);
    let mut required_angle = f64::NEG_INFINITY;
    for i in 1..samples {
        let d = target_distance * i as f64 / samples as f64;
        let (lat, lon) = transform.inverse(dir_east * d, dir_north * d);
        let elevation = sample_heightmap(data, lat, lon).unwrap_or(0.0) as f64;
        let angle = (elevation - curvature_drop_m(d, r_eff) - observer_height) / d;
        if angle > required_angle {
            required_angle = angle;
        }
    }
    // angle(h) = (h - curvature_drop(d) - observer_height) / d が対象高度hについて線形なので、
    // angle(h) == required_angle となるhを直接解く(それ以上の高度なら見える下限)。
    Some(required_angle * target_distance + curvature_drop_m(target_distance, r_eff) + observer_height)
}
