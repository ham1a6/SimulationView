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

/// 計算する方位角の刻み数。1周=6400mil(NATO式)で、1mil刻み(=360/6400度≒0.05625度)。
/// 50km先で約49m幅、地形の最細解像度(30m)と同程度の細かさ。
const NUM_AZIMUTHS: usize = 6400;

/// 方位角インデックス(0〜`NUM_AZIMUTHS`-1、1mil刻み)を度(北=0・東=90・時計回り)へ変換する。
fn azimuth_deg_of(az_i: usize) -> f64 {
    az_i as f64 * 360.0 / NUM_AZIMUTHS as f64
}

/// 1方位角あたりのサンプル点数(`compute_los`・`compute_coverage_area`・`compute_los_dome`)。
/// 最大観測範囲50kmで50m間隔、200kmで200m間隔(地形の最細解像度30mに近い細かさ)。
/// 増やすほど計算時間はほぼ比例して増える(6400方位 x この点数だけ標高を引く)。
const SAMPLES_PER_RAY: usize = 1000;
/// 対象地点1点までの見通し判定(`is_visible`・`min_visible_altitude`。断面図で断面上の
/// 各点ごとに呼ぶので軽くしておく必要がある)のサンプル点数の上限。500m間隔で、10〜この値の範囲。
const MAX_TARGET_SAMPLES: usize = 200;

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
            let azimuth_deg = azimuth_deg_of(az_i);
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
/// 適用したもの。断面図の覆域表示(`ui::cross_section_view`)で、断面上の各点がいずれかの
/// レーダーから見えるかを判定するために使う。
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

    let samples = ((target_distance / 500.0).ceil() as usize).clamp(10, MAX_TARGET_SAMPLES);
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
/// (`ui::cross_section_view`)で、地表だけでなく上空を含めた覆域を図示するために使う。
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
    let samples = ((target_distance / 500.0).ceil() as usize).clamp(10, MAX_TARGET_SAMPLES);
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

/// 指定した1つの海抜高度(絶対標高、メートル)を飛ぶ対象について、全方位角の
/// 探知可能距離(水平距離)を計算する。2D地図モード(`components/terrain_view.rs`)での
/// 覆域表示に使う。`compute_los_dome`が「仰角一定の直線」を仰角ごとに走査するのに対し、
/// ここでは「高度一定の直線」を対象の高度について走査する(observer-target間の仰角の必要値は
/// 地球曲率の効果で距離とともにほぼ単調に下がるため、`compute_los_dome`と同じ
/// 「最初に遮蔽されたら以降も遮蔽され続ける」扱いにできる。地形自身の遮蔽判定は
/// `compute_los`と同じマスク角アルゴリズム)。
pub fn compute_coverage_area(
    data: &TerrainData,
    origin: &Origin,
    params: &LosParams,
    target_altitude_m: f64,
) -> Vec<LosPoint> {
    let transform = EnuTransform::new(origin, &data.metadata.ellipsoid);
    let observer_ground_elevation =
        sample_heightmap(data, origin.lat_deg, origin.lon_deg).unwrap_or(0.0) as f64;
    let observer_height = observer_ground_elevation + params.observer_height_m;
    let r_eff = EARTH_RADIUS_M * K_FACTOR;

    (0..NUM_AZIMUTHS)
        .map(|az_i| {
            let azimuth_deg = azimuth_deg_of(az_i);
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
                if angle > max_angle {
                    max_angle = angle;
                }
                let target_angle =
                    (target_altitude_m - curvature_drop_m(d, r_eff) - observer_height) / d;
                if target_angle >= max_angle {
                    visible_range = d;
                } else {
                    break;
                }
            }
            LosPoint { azimuth_deg, range_m: visible_range }
        })
        .collect()
}

/// 半球状ドーム表示(`terrain::markers::push_coverage_dome`)1リングぶんの、全方位角の
/// 見通し限界スラントレンジ。
pub struct DomeRing {
    pub elevation_deg: f64,
    /// 各点の`range_m`は、この仰角における観測点からのスラントレンジ(直線距離)。
    pub points: Vec<LosPoint>,
}

/// 指定した複数の仰角それぞれについて、全方位角の見通し限界距離(スラントレンジ)を計算する。
/// `compute_los`(仰角0°=地表を這う見え方のみ)を仰角方向に拡張したもの。
///
/// `compute_los`との違い: 観測点からの**直線(仰角一定のレイ)**が地形に遮蔽されずに
/// どこまで届くかを求める。直線は一度地形にぶつかったら、その先で地形が下がっても
/// (直線である以上)二度と地形の陰から出てこないため、「最初に遮蔽された時点で打ち切り」が
/// 物理的に正しい(`compute_los`の「手前の尾根の陰でも先で地形が高くなれば再び見える」扱いは、
/// 地表を這うように見ていく別の設定であり、ここでは採用しない)。**地形に遮蔽されない方角
/// では、最大観測範囲(スラントレンジ)までそのまま届く**ため、遮蔽がなければ滑らかな球面
/// (=どの仰角でも同じ半径)になる。地表付近だけ、地形の遮蔽によって半径が内側に凹む。
pub fn compute_los_dome(
    data: &TerrainData,
    origin: &Origin,
    params: &LosParams,
    elevation_degs: &[f64],
) -> Vec<DomeRing> {
    let transform = EnuTransform::new(origin, &data.metadata.ellipsoid);
    let observer_ground_elevation =
        sample_heightmap(data, origin.lat_deg, origin.lon_deg).unwrap_or(0.0) as f64;
    let observer_height = observer_ground_elevation + params.observer_height_m;
    let r_eff = EARTH_RADIUS_M * K_FACTOR;

    let ring_tans: Vec<f64> = elevation_degs.iter().map(|d| d.to_radians().tan()).collect();
    let ring_cos: Vec<f64> = elevation_degs.iter().map(|d| d.to_radians().cos()).collect();
    let num_rings = elevation_degs.len();
    debug_assert!(
        elevation_degs.windows(2).all(|w| w[0] < w[1]) && elevation_degs.iter().all(|&d| (0.0..90.0).contains(&d)),
        "elevation_degsは0以上90未満の昇順で渡すこと"
    );
    let mut ring_slant_ranges = vec![vec![0.0_f64; NUM_AZIMUTHS]; num_rings];

    for az_i in 0..NUM_AZIMUTHS {
        let azimuth_deg = azimuth_deg_of(az_i);
        let az_rad = azimuth_deg.to_radians();
        let dir_east = az_rad.sin();
        let dir_north = az_rad.cos();
        let data_max = max_valid_distance(data, &transform, dir_east, dir_north);

        // 仰角が大きいほど、同じスラントレンジ上限に対応する水平距離の上限は小さくなる
        // (horizontal = スラントレンジ * cos(仰角))。
        let horizontal_cap = |k: usize| data_max.min(params.max_range_m * ring_cos[k]);
        let ray_max = (0..num_rings).map(horizontal_cap).fold(0.0_f64, f64::max);
        if ray_max <= 0.0 {
            continue;
        }
        let sample_distance = |i: usize| ray_max * i as f64 / SAMPLES_PER_RAY as f64;

        // リングkについて、遮蔽されずに届いた最後のサンプル番号`reach`(0なら手前で遮蔽)から、
        // そのリングのスラントレンジを確定する(水平距離の上限`horizontal_cap(k)`を超える
        // サンプルは対象外)。
        let mut finalize = |k: usize, reach: usize| {
            let cap = horizontal_cap(k);
            let mut j = reach.min(((cap / ray_max) * SAMPLES_PER_RAY as f64).floor() as usize);
            while j > 0 && sample_distance(j) > cap {
                j -= 1;
            }
            while j < reach && sample_distance(j + 1) <= cap {
                j += 1;
            }
            if j > 0 {
                let d = sample_distance(j);
                ring_slant_ranges[k][az_i] = if ring_cos[k] > 1e-6 { d / ring_cos[k] } else { d };
            }
        };

        // 仰角が低いリングほど`ring_tans`が小さく、先に遮蔽される(`max_angle`は単調非減少)。
        // 遮蔽が確定したリングは先頭から順に`first_alive`まで進むので、リング数に関わらず
        // 1サンプルあたりの比較は(新たに遮蔽されたリングを除いて)1回で済む。
        let mut max_angle = f64::NEG_INFINITY;
        let mut first_alive = 0;
        for i in 1..=SAMPLES_PER_RAY {
            let d = sample_distance(i);
            let (lat, lon) = transform.inverse(dir_east * d, dir_north * d);
            let elevation = sample_heightmap(data, lat, lon).unwrap_or(0.0) as f64;
            let apparent_height = elevation - curvature_drop_m(d, r_eff);
            let angle = (apparent_height - observer_height) / d;
            if angle > max_angle {
                max_angle = angle;
                while first_alive < num_rings && max_angle > ring_tans[first_alive] {
                    finalize(first_alive, i - 1);
                    first_alive += 1;
                }
                if first_alive == num_rings {
                    break;
                }
            }
        }
        for k in first_alive..num_rings {
            finalize(k, SAMPLES_PER_RAY);
        }
    }

    elevation_degs
        .iter()
        .zip(ring_slant_ranges)
        .map(|(&elevation_deg, ranges)| {
            let points = ranges
                .into_iter()
                .enumerate()
                .map(|(az_i, range_m)| LosPoint {
                    azimuth_deg: azimuth_deg_of(az_i),
                    range_m,
                })
                .collect();
            DomeRing { elevation_deg, points }
        })
        .collect()
}
