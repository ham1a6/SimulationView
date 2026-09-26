//! 見通し範囲(レーダー探知可能範囲)の計算。指定したポイント(原点)から全方位角へ
//! 地表をサンプリングし、地形による遮蔽と地球曲率(等価地球半径)を考慮した上で、
//! 各方位角ごとの見通し限界距離を求める。`components/los_view.rs`から使う。

use super::geodesy::EnuTransform;
use super::heightmap::sample_heightmap;
use super::loader::TerrainData;
use super::origin::Origin;
use super::profile::max_valid_distance;

/// 平均地球半径(メートル)。
const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// 等価地球半径係数(k-factor)。標準大気中では電波が幾何学的な直線よりわずかに
/// 下向きに屈折するため、実際の見通し距離は真球上の幾何学的な地平線より遠くなる。
/// この効果を「実際の地球半径をk倍した仮想的に大きい球面上で電波が直進する」近似で
/// 表したものが等価地球半径で、標準大気(標準的な気温減率)ではk=4/3が広く使われる
/// (レーダー・無線工学の標準的な近似。実際の大気状態によって変動するが、v1では固定値とする)。
const K_FACTOR: f64 = 4.0 / 3.0;

/// 等価地球半径(メートル)。
const R_EFF_M: f64 = EARTH_RADIUS_M * K_FACTOR;

/// 計算する方位角の刻み数。1周=6400mil(NATO式)で、1mil刻み(=360/6400度≒0.05625度)。
/// 50km先で約49m幅、地形の最細解像度(30m)と同程度の細かさ。
const NUM_AZIMUTHS: usize = 6400;

/// 方位角インデックス(0〜`NUM_AZIMUTHS`-1、1mil刻み)を度(北=0・東=90・時計回り)へ変換する。
fn azimuth_deg_of(az_i: usize) -> f64 {
    az_i as f64 * 360.0 / NUM_AZIMUTHS as f64
}

/// 方位角インデックスの(東向き成分, 北向き成分)。
fn azimuth_direction(az_i: usize) -> (f64, f64) {
    let az_rad = azimuth_deg_of(az_i).to_radians();
    (az_rad.sin(), az_rad.cos())
}

/// 方位(出力の番号`j`)ごとの距離に方位角を付ける。
fn to_points(ranges: Vec<f64>, azimuth_step: usize) -> Vec<LosPoint> {
    ranges
        .into_iter()
        .enumerate()
        .map(|(j, range_m)| LosPoint {
            azimuth_deg: azimuth_deg_of(j * azimuth_step),
            range_m,
        })
        .collect()
}

/// 観測点から対象地点(緯度経度)までの水平距離と、東向き・北向きの単位ベクトル
/// (距離が1m未満なら向きは(0, 0)。呼び出し側は先に距離で場合分けする)。
fn transform_to_target(transform: &EnuTransform, lat_deg: f64, lon_deg: f64) -> (f64, f64, f64) {
    let pos = transform.transform(lat_deg, lon_deg, 0.0);
    let (east, north) = (pos[0] as f64, pos[1] as f64);
    let distance = east.hypot(north);
    if distance < 1.0 {
        (distance, 0.0, 0.0)
    } else {
        (distance, east / distance, north / distance)
    }
}

/// 観測点から出るレイの計算に共通の前提(5つの計算関数が同じ前処理を持っていた)。
struct RayContext {
    /// 観測点を原点とするENU変換。
    transform: EnuTransform,
    /// 観測点(アンテナ)の海抜高度(メートル)= 観測点の地表の標高 + アンテナ高。
    observer_altitude_msl: f64,
}

impl RayContext {
    fn new(data: &TerrainData, origin: &Origin, antenna_height_m: f64) -> Self {
        let ground = sample_heightmap(data, origin.lat_deg, origin.lon_deg) as f64;
        Self {
            transform: EnuTransform::new(origin, &data.metadata.ellipsoid),
            observer_altitude_msl: ground + antenna_height_m,
        }
    }

    /// 水平距離`d`にある海抜高度`height_m`の点の見かけの仰角(地球曲率による低下を差し引いた角度の正接の近似)。
    fn angle_of(&self, height_m: f64, d: f64) -> f64 {
        (height_m - curvature_drop_m(d, R_EFF_M) - self.observer_altitude_msl) / d
    }

    /// (東, 北)向きに水平距離`d`だけ進んだ地表点の見かけの仰角。
    fn angle_at(&self, data: &TerrainData, dir_east: f64, dir_north: f64, d: f64) -> f64 {
        let (lat, lon) = self.transform.inverse(dir_east * d, dir_north * d);
        self.angle_of(sample_heightmap(data, lat, lon) as f64, d)
    }
}

/// 観測点から対象地点(緯度経度)へ向かう1本のレイ(`is_visible`・`min_visible_altitude`)。
struct TargetRay {
    ctx: RayContext,
    /// 対象地点までの水平距離(メートル)。
    distance: f64,
    dir_east: f64,
    dir_north: f64,
}

impl TargetRay {
    fn new(
        data: &TerrainData,
        radar: &Origin,
        antenna_height_m: f64,
        target_lat_deg: f64,
        target_lon_deg: f64,
    ) -> Self {
        let ctx = RayContext::new(data, radar, antenna_height_m);
        let (distance, dir_east, dir_north) =
            transform_to_target(&ctx.transform, target_lat_deg, target_lon_deg);
        Self {
            ctx,
            distance,
            dir_east,
            dir_north,
        }
    }

    /// 観測点と対象地点の間(両端を除く)の地表サンプルそれぞれの見かけの仰角。
    /// 500m間隔で、10〜`MAX_TARGET_SAMPLES`区間に分ける。
    fn terrain_angles<'a>(&'a self, data: &'a TerrainData) -> impl Iterator<Item = f64> + 'a {
        let samples = ((self.distance / 500.0).ceil() as usize).clamp(10, MAX_TARGET_SAMPLES);
        (1..samples).map(move |i| {
            let d = self.distance * i as f64 / samples as f64;
            self.ctx.angle_at(data, self.dir_east, self.dir_north, d)
        })
    }
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
#[derive(Debug, Clone, Copy, PartialEq)]
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

/// 方位角の刻み(mil)から、出力する方位の数を求める。`NUM_AZIMUTHS`(6400)を割り切る値でなければならない。
fn azimuth_count(azimuth_step: usize) -> usize {
    assert!(
        azimuth_step >= 1 && NUM_AZIMUTHS.is_multiple_of(azimuth_step),
        "azimuth_stepは{NUM_AZIMUTHS}を割り切る値にすること: {azimuth_step}"
    );
    NUM_AZIMUTHS / azimuth_step
}

/// `RangeComputation`が方位角ごとに求める距離の種類。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RangeKind {
    /// 地表を這うように見たときの見通し限界距離(`compute_los`)。
    Visible,
    /// 指定した海抜高度(メートル)を飛ぶ対象の探知可能距離(`compute_coverage_area`)。
    AtAltitude(f64),
}

/// 方位角ごとに1つの距離を求める計算(`compute_los`・`compute_coverage_area`)を、少しずつ進められる形にしたもの。
/// 全方位を一度に計算すると(6400方位 x 1,000サンプル)ブラウザの画面が固まるので、呼び出し側が
/// `advance`を時間で区切って呼び、合間に画面へ処理を譲る(`ui::util::run_in_slices`)。
/// `azimuth_step`(mil)を大きくすると、方位を間引いて計算量がその分だけ減る(出力の点数は`6400 / azimuth_step`)。
pub struct RangeComputation {
    ctx: RayContext,
    max_range_m: f64,
    kind: RangeKind,
    azimuth_step: usize,
    /// 方位(0〜`azimuth_count`-1)ごとの距離。計算済みの分だけ埋まっている。
    ranges: Vec<f64>,
    done: usize,
}

impl RangeComputation {
    pub fn new(
        data: &TerrainData,
        origin: &Origin,
        params: &LosParams,
        kind: RangeKind,
        azimuth_step: usize,
    ) -> Self {
        Self {
            ctx: RayContext::new(data, origin, params.observer_height_m),
            max_range_m: params.max_range_m,
            kind,
            azimuth_step,
            ranges: vec![0.0; azimuth_count(azimuth_step)],
            done: 0,
        }
    }

    /// 次の方位から最大`count`個を計算する。終わったら`true`。
    /// `data`は毎回渡す(計算の途中で地形のレベルが切り替わっても、そのときの地形で続ける)。
    pub fn advance(&mut self, data: &TerrainData, count: usize) -> bool {
        let end = (self.done + count).min(self.ranges.len());
        for j in self.done..end {
            self.ranges[j] = self.trace(data, j * self.azimuth_step);
        }
        self.done = end;
        end == self.ranges.len()
    }

    /// 1方位(方位角インデックス`az_i`)の距離。
    fn trace(&self, data: &TerrainData, az_i: usize) -> f64 {
        let ctx = &self.ctx;
        let (dir_east, dir_north) = azimuth_direction(az_i);

        let data_max = max_valid_distance(data, &ctx.transform, dir_east, dir_north);
        let ray_max = data_max.min(self.max_range_m);
        if ray_max <= 0.0 {
            return 0.0;
        }

        let mut max_angle = f64::NEG_INFINITY;
        let mut visible_range = 0.0_f64;
        for i in 1..=SAMPLES_PER_RAY {
            let d = ray_max * i as f64 / SAMPLES_PER_RAY as f64;
            let angle = ctx.angle_at(data, dir_east, dir_north, d);
            match self.kind {
                RangeKind::Visible => {
                    if angle >= max_angle {
                        max_angle = angle;
                        visible_range = d;
                    }
                }
                RangeKind::AtAltitude(target_altitude_m) => {
                    if angle > max_angle {
                        max_angle = angle;
                    }
                    if ctx.angle_of(target_altitude_m, d) >= max_angle {
                        visible_range = d;
                    } else {
                        break;
                    }
                }
            }
        }
        visible_range
    }

    /// 結果(方位角つき)。未計算の方位は距離0。
    pub fn finish(self) -> Vec<LosPoint> {
        to_points(self.ranges, self.azimuth_step)
    }

    /// 全方位を一度に計算する(単体テスト用。画面を固めないよう、UIからは`advance`で小分けにすること)。
    #[cfg(test)]
    fn run(mut self, data: &TerrainData) -> Vec<LosPoint> {
        self.advance(data, usize::MAX);
        self.finish()
    }
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
///
/// 全方位を一度に計算して返す。画面を固めたくないときは`RangeComputation`を小分けに進めること。
#[cfg(test)]
pub fn compute_los(data: &TerrainData, origin: &Origin, params: &LosParams) -> Vec<LosPoint> {
    RangeComputation::new(data, origin, params, RangeKind::Visible, 1).run(data)
}

/// 単一の観測点(レーダー)から単一の対象地点への見通し(手前の地形に遮蔽されないか)を
/// 判定する。`compute_los`と同じマスク角アルゴリズムを、対象地点までの1本のレイに絞って
/// 適用したもの。断面図の覆域表示(`ui::cross_section_view`)で、断面上の各点がいずれかの
/// レーダーから見えるかを判定するために使う。
pub fn is_visible(
    data: &TerrainData,
    radar: &Origin,
    params: &LosParams,
    target_lat_deg: f64,
    target_lon_deg: f64,
) -> bool {
    let ray = TargetRay::new(
        data,
        radar,
        params.observer_height_m,
        target_lat_deg,
        target_lon_deg,
    );
    if ray.distance < 1.0 {
        return true;
    }
    if ray.distance > params.max_range_m {
        return false;
    }
    let target_elevation = sample_heightmap(data, target_lat_deg, target_lon_deg) as f64;
    let target_angle = ray.ctx.angle_of(target_elevation, ray.distance);
    let blocked = ray.terrain_angles(data).any(|angle| angle > target_angle);
    !blocked
}

/// 単一の観測点(レーダー)から見て、指定した地表座標の真上で「これ以上の高度(標高、m)なら
/// 見える」という下限高度を求める(等価地球半径による曲率・手前の地形によるマスク角を考慮)。
/// `is_visible`が対象の実標高との比較で真偽を返すのに対し、こちらは仰角の式が対象の高度に
/// ついて線形であることを利用し、可視となる最小高度を直接逆算する。断面図の覆域表示
/// (`ui::cross_section_view`)で、地表だけでなく上空を含めた覆域を図示するために使う。
/// 対象地点までの距離がレーダーの最大観測範囲を超える場合はNone(どんな高度でも覆域外)。
pub fn min_visible_altitude(
    data: &TerrainData,
    radar: &Origin,
    params: &LosParams,
    target_lat_deg: f64,
    target_lon_deg: f64,
) -> Option<f64> {
    let ray = TargetRay::new(
        data,
        radar,
        params.observer_height_m,
        target_lat_deg,
        target_lon_deg,
    );
    if ray.distance > params.max_range_m {
        return None;
    }
    if ray.distance < 1.0 {
        return Some(ray.ctx.observer_altitude_msl);
    }

    // 観測点から対象地点までの間の地形が作る最大の見かけ仰角(=対象が見えるために
    // 必要な最低仰角)を求める。target自身の標高には依存しない点がis_visibleと異なる。
    let required_angle =
        ray.terrain_angles(data).fold(
            f64::NEG_INFINITY,
            |max, angle| if angle > max { angle } else { max },
        );
    // angle(h) = (h - curvature_drop(d) - observer_altitude_msl) / d が対象高度hについて線形なので、
    // angle(h) == required_angle となるhを直接解く(それ以上の高度なら見える下限)。
    Some(
        required_angle * ray.distance
            + curvature_drop_m(ray.distance, R_EFF_M)
            + ray.ctx.observer_altitude_msl,
    )
}

/// 指定した1つの海抜高度(絶対標高、メートル)を飛ぶ対象について、全方位角の
/// 探知可能距離(水平距離)を計算する。2D地図モード(`components/terrain_view.rs`)での
/// 覆域表示に使う。`compute_los_dome`が「仰角一定の直線」を仰角ごとに走査するのに対し、
/// ここでは「高度一定の直線」を対象の高度について走査する(observer-target間の仰角の必要値は
/// 地球曲率の効果で距離とともにほぼ単調に下がるため、`compute_los_dome`と同じ
/// 「最初に遮蔽されたら以降も遮蔽され続ける」扱いにできる。地形自身の遮蔽判定は
/// `compute_los`と同じマスク角アルゴリズム)。
#[cfg(test)]
pub fn compute_coverage_area(
    data: &TerrainData,
    origin: &Origin,
    params: &LosParams,
    target_altitude_m: f64,
) -> Vec<LosPoint> {
    RangeComputation::new(
        data,
        origin,
        params,
        RangeKind::AtAltitude(target_altitude_m),
        1,
    )
    .run(data)
}

/// 半球状ドーム表示(`terrain::markers`)1リングぶんの、全方位角の
/// 見通し限界スラントレンジ。
#[derive(Debug, Clone)]
pub struct DomeRing {
    pub elevation_deg: f64,
    /// 各点の`range_m`は、この仰角における観測点からのスラントレンジ(直線距離)。
    pub points: Vec<LosPoint>,
}

/// 指定した複数の仰角それぞれについて、全方位角の見通し限界距離(スラントレンジ)を計算する
/// (`compute_los_dome`)を、少しずつ進められる形にしたもの。使い方・`azimuth_step`は`RangeComputation`と同じ。
pub struct DomeComputation {
    ctx: RayContext,
    max_range_m: f64,
    elevation_degs: Vec<f64>,
    ring_tans: Vec<f64>,
    ring_cos: Vec<f64>,
    azimuth_step: usize,
    /// リングごと・方位ごとのスラントレンジ。計算済みの方位の分だけ埋まっている。
    ring_slant_ranges: Vec<Vec<f64>>,
    done: usize,
}

impl DomeComputation {
    pub fn new(
        data: &TerrainData,
        origin: &Origin,
        params: &LosParams,
        elevation_degs: &[f64],
        azimuth_step: usize,
    ) -> Self {
        debug_assert!(
            elevation_degs.windows(2).all(|w| w[0] < w[1])
                && elevation_degs.iter().all(|&d| (0.0..90.0).contains(&d)),
            "elevation_degsは0以上90未満の昇順で渡すこと"
        );
        let count = azimuth_count(azimuth_step);
        Self {
            ctx: RayContext::new(data, origin, params.observer_height_m),
            max_range_m: params.max_range_m,
            elevation_degs: elevation_degs.to_vec(),
            ring_tans: elevation_degs
                .iter()
                .map(|d| d.to_radians().tan())
                .collect(),
            ring_cos: elevation_degs
                .iter()
                .map(|d| d.to_radians().cos())
                .collect(),
            azimuth_step,
            ring_slant_ranges: vec![vec![0.0; count]; elevation_degs.len()],
            done: 0,
        }
    }

    /// 計算する方位の総数。
    pub fn total(&self) -> usize {
        azimuth_count(self.azimuth_step)
    }

    /// 次の方位から最大`count`個を計算する。終わったら`true`。
    pub fn advance(&mut self, data: &TerrainData, count: usize) -> bool {
        let total = self.total();
        let end = (self.done + count).min(total);
        for j in self.done..end {
            self.trace(data, j);
        }
        self.done = end;
        end == total
    }

    /// 1方位(出力の番号`j`)について、全リングのスラントレンジを求めて`ring_slant_ranges`へ書く。
    fn trace(&mut self, data: &TerrainData, j: usize) {
        let Self {
            ctx,
            max_range_m,
            ring_tans,
            ring_cos,
            azimuth_step,
            ring_slant_ranges,
            ..
        } = self;
        let num_rings = ring_tans.len();
        let (dir_east, dir_north) = azimuth_direction(j * *azimuth_step);
        let data_max = max_valid_distance(data, &ctx.transform, dir_east, dir_north);

        // 仰角が大きいほど、同じスラントレンジ上限に対応する水平距離の上限は小さくなる
        // (horizontal = スラントレンジ * cos(仰角))。
        let horizontal_cap = |k: usize| data_max.min(*max_range_m * ring_cos[k]);
        let ray_max = (0..num_rings).map(horizontal_cap).fold(0.0_f64, f64::max);
        if ray_max <= 0.0 {
            return;
        }
        let sample_distance = |i: usize| ray_max * i as f64 / SAMPLES_PER_RAY as f64;

        // リングkについて、遮蔽されずに届いた最後のサンプル番号`reach`(0なら手前で遮蔽)から、
        // そのリングのスラントレンジを確定する(水平距離の上限`horizontal_cap(k)`を超える
        // サンプルは対象外)。
        let mut finalize = |k: usize, reach: usize| {
            let cap = horizontal_cap(k);
            // 上限(`cap`)まで遮蔽されなかったリングは、サンプル位置に丸めず上限ちょうどにする。
            // 丸めると、仰角ごとに水平距離の刻み(最大観測範囲/サンプル数)への丸め方が違うので、
            // 遮蔽のない方角でも高い仰角のリングほど半径が不揃いになり(cos仰角で割るので数百mの凸凹)、
            // ドームが滑らかな球面にならない。
            let cap_reached = reach >= SAMPLES_PER_RAY || sample_distance(reach + 1) > cap;
            let d = if cap_reached {
                cap
            } else {
                sample_distance(reach)
            };
            if d > 0.0 {
                ring_slant_ranges[k][j] = if ring_cos[k] > 1e-6 {
                    d / ring_cos[k]
                } else {
                    d
                };
            }
        };

        // 仰角が低いリングほど`ring_tans`が小さく、先に遮蔽される(`max_angle`は単調非減少)。
        // 遮蔽が確定したリングは先頭から順に`first_alive`まで進むので、リング数に関わらず
        // 1サンプルあたりの比較は(新たに遮蔽されたリングを除いて)1回で済む。
        let mut max_angle = f64::NEG_INFINITY;
        let mut first_alive = 0;
        for i in 1..=SAMPLES_PER_RAY {
            let angle = ctx.angle_at(data, dir_east, dir_north, sample_distance(i));
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

    /// 結果(リングごと・方位角つき)。未計算の方位は距離0。
    pub fn finish(self) -> Vec<DomeRing> {
        let step = self.azimuth_step;
        self.elevation_degs
            .into_iter()
            .zip(self.ring_slant_ranges)
            .map(|(elevation_deg, ranges)| DomeRing {
                elevation_deg,
                points: to_points(ranges, step),
            })
            .collect()
    }
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
///
/// 全方位を一度に計算して返す。画面を固めたくないときは`DomeComputation`を小分けに進めること。
#[cfg(test)]
pub fn compute_los_dome(
    data: &TerrainData,
    origin: &Origin,
    params: &LosParams,
    elevation_degs: &[f64],
) -> Vec<DomeRing> {
    let mut computation = DomeComputation::new(data, origin, params, elevation_degs, 1);
    computation.advance(data, usize::MAX);
    computation.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 標高0mの平らな地形(緯度30〜33度・経度120〜123度)と、その中央の原点。
    fn flat() -> (TerrainData, Origin) {
        (
            TerrainData::synthetic(30, 120, 3, 3, |_, _| 0),
            Origin {
                lat_deg: 31.5,
                lon_deg: 121.5,
            },
        )
    }

    /// 経度121.6667度(原点の東約16km)に1,500mの尾根がある地形(それ以外は0m)。
    fn ridge() -> TerrainData {
        TerrainData::synthetic(30, 120, 3, 3, |_, lon| {
            if (lon - 121.0 - 4.0 / 6.0).abs() < 1e-6 {
                1500
            } else {
                0
            }
        })
    }

    /// 観測点(`lat`, `lon`)・アンテナ高・最大観測範囲を個別に渡す`is_visible`。
    fn visible(
        data: &TerrainData,
        lat: f64,
        lon: f64,
        height_m: f64,
        max_range_m: f64,
        target_lat: f64,
        target_lon: f64,
    ) -> bool {
        let (radar, params) = radar(lat, lon, height_m, max_range_m);
        is_visible(data, &radar, &params, target_lat, target_lon)
    }

    /// 観測点(`lat`, `lon`)・アンテナ高・最大観測範囲を個別に渡す`min_visible_altitude`。
    fn min_altitude(
        data: &TerrainData,
        lat: f64,
        lon: f64,
        height_m: f64,
        max_range_m: f64,
        target_lat: f64,
        target_lon: f64,
    ) -> Option<f64> {
        let (radar, params) = radar(lat, lon, height_m, max_range_m);
        min_visible_altitude(data, &radar, &params, target_lat, target_lon)
    }

    fn radar(lat: f64, lon: f64, height_m: f64, max_range_m: f64) -> (Origin, LosParams) {
        (
            Origin {
                lat_deg: lat,
                lon_deg: lon,
            },
            LosParams {
                observer_height_m: height_m,
                max_range_m,
            },
        )
    }

    /// 平坦地で高さhのアンテナから見える地平線までの距離 √(2 r_eff h)(電波の地平線)。
    fn radio_horizon_m(height_m: f64) -> f64 {
        (2.0 * R_EFF_M * height_m).sqrt()
    }

    #[test]
    fn azimuth_index_is_in_mils() {
        assert_eq!(azimuth_deg_of(0), 0.0);
        assert_eq!(azimuth_deg_of(1600), 90.0);
        assert_eq!(azimuth_deg_of(3200), 180.0);
        assert!((azimuth_deg_of(NUM_AZIMUTHS - 1) - 359.94375).abs() < 1e-9);
    }

    #[test]
    fn curvature_drop_grows_with_the_square_of_distance() {
        let r = R_EFF_M;
        assert_eq!(curvature_drop_m(0.0, r), 0.0);
        assert!((curvature_drop_m(20_000.0, r) - 4.0 * curvature_drop_m(10_000.0, r)).abs() < 1e-9);
    }

    #[test]
    fn point_visibility_on_flat_ground_follows_the_radio_horizon() {
        let (data, origin) = flat();
        let (lat, lon) = (origin.lat_deg, origin.lon_deg);
        // 高さ10mのアンテナの地平線は約13km。その内側の地表は見え、外側は見えない。
        assert!((radio_horizon_m(10.0) - 13_000.0).abs() < 100.0);
        let at = |km: f64| lon + km / 94.9; // この緯度の経度1度は約94.9km
        assert!(visible(&data, lat, lon, 10.0, 200_000.0, lat, at(5.0)));
        assert!(!visible(&data, lat, lon, 10.0, 200_000.0, lat, at(30.0)));
        // アンテナが高ければ30km先も見える(100mなら地平線は約41km)。
        assert!(visible(&data, lat, lon, 100.0, 200_000.0, lat, at(30.0)));
        // 最大観測範囲の外は常に見えない。真上(距離0)は見える。
        assert!(!visible(&data, lat, lon, 100.0, 20_000.0, lat, at(30.0)));
        assert!(visible(&data, lat, lon, 10.0, 20_000.0, lat, lon));
    }

    #[test]
    fn a_ridge_hides_what_is_behind_it() {
        let data = ridge();
        let (lat, lon) = (31.5, 121.5);
        let at = |km: f64| lon + km / 94.9;
        // 尾根(東16km)の向こう25km地点は、41kmの地平線の内側でも尾根に遮られる。
        assert!(!visible(&data, lat, lon, 100.0, 200_000.0, lat, at(25.0)));
        // 反対側(西25km)は遮るものがなく見える。
        assert!(visible(&data, lat, lon, 100.0, 200_000.0, lat, at(-25.0)));
        // 尾根の手前は見える。
        assert!(visible(&data, lat, lon, 100.0, 200_000.0, lat, at(8.0)));
    }

    #[test]
    fn min_visible_altitude_is_consistent_with_visibility() {
        let (data, origin) = flat();
        let (lat, lon) = (origin.lat_deg, origin.lon_deg);
        let at = |km: f64| lon + km / 94.9;
        // 10mのアンテナで30km先を見るには、地表より少し(約17m)高くないと見えない。
        let alt = min_altitude(&data, lat, lon, 10.0, 200_000.0, lat, at(30.0)).unwrap();
        assert!(alt > 10.0 && alt < 25.0, "alt={alt}");
        // 100mのアンテナなら地表(0m)でほぼ見える。
        let low = min_altitude(&data, lat, lon, 100.0, 200_000.0, lat, at(30.0)).unwrap();
        assert!(low < 5.0 && low > -20.0, "low={low}");
        // 最大観測範囲の外はNone、真上は観測点の高さ。
        assert!(min_altitude(&data, lat, lon, 10.0, 20_000.0, lat, at(30.0)).is_none());
        assert_eq!(
            min_altitude(&data, lat, lon, 10.0, 20_000.0, lat, lon),
            Some(10.0)
        );
    }

    #[test]
    fn los_range_on_flat_ground_is_the_radio_horizon_in_every_direction() {
        let (data, origin) = flat();
        let params = LosParams {
            observer_height_m: 10.0,
            max_range_m: 50_000.0,
        };
        let result = compute_los(&data, &origin, &params);
        assert_eq!(result.len(), NUM_AZIMUTHS);
        assert_eq!(result[1600].azimuth_deg, 90.0);
        let horizon = radio_horizon_m(10.0);
        for p in [
            &result[0],
            &result[1600],
            &result[3200],
            &result[4800],
            &result[777],
        ] {
            assert!(
                (p.range_m - horizon).abs() < 300.0,
                "az={} range={}",
                p.azimuth_deg,
                p.range_m
            );
        }
    }

    #[test]
    fn coverage_area_reaches_the_max_range_for_a_high_target_and_stops_at_the_ridge() {
        let (data, origin) = flat();
        let params = LosParams {
            observer_height_m: 10.0,
            max_range_m: 30_000.0,
        };
        // 平坦地の上空1,000mを飛ぶ対象は、最大観測範囲(30km)までどの方位でも見える。
        for p in compute_coverage_area(&data, &origin, &params, 1_000.0)
            .iter()
            .step_by(800)
        {
            assert!(
                (p.range_m - 30_000.0).abs() < 1.0,
                "az={} range={}",
                p.azimuth_deg,
                p.range_m
            );
        }
        // 東の尾根(標高1,500m)より低い高度の対象は、東側では尾根の手前までしか届かない。
        let cov = compute_coverage_area(&ridge(), &origin, &params, 500.0);
        let east = cov[1600].range_m;
        let west = cov[4800].range_m;
        assert!(east < 17_000.0, "east={east}");
        assert!((west - 30_000.0).abs() < 1.0, "west={west}");
    }

    /// 全方位を小分けにして進めた結果は、一度に計算した結果と同じになる(小分けの境目で結果が変わらない)。
    #[test]
    fn sliced_computation_matches_the_batch_result() {
        let data = ridge();
        let origin = Origin {
            lat_deg: 31.5,
            lon_deg: 121.5,
        };
        let params = LosParams {
            observer_height_m: 100.0,
            max_range_m: 30_000.0,
        };
        let batch = compute_los(&data, &origin, &params);
        let mut sliced = RangeComputation::new(&data, &origin, &params, RangeKind::Visible, 1);
        while !sliced.advance(&data, 777) {}
        let sliced = sliced.finish();
        assert_eq!(batch.len(), sliced.len());
        assert!(batch.iter().zip(&sliced).all(|(a, b)| a == b));

        let elevations = [0.0, 3.0, 10.0];
        let batch = compute_los_dome(&data, &origin, &params, &elevations);
        let mut sliced = DomeComputation::new(&data, &origin, &params, &elevations, 1);
        while !sliced.advance(&data, 501) {}
        let sliced = sliced.finish();
        for (a, b) in batch.iter().zip(&sliced) {
            assert!(a.points.iter().zip(&b.points).all(|(p, q)| p == q));
        }
    }

    /// 方位を間引いた計算(`azimuth_step`)は、全方位の計算の同じ方位の値と一致する。点数は方位の刻みの分だけ減る。
    #[test]
    fn coarser_azimuth_step_matches_the_full_result_at_the_same_azimuths() {
        let data = ridge();
        let origin = Origin {
            lat_deg: 31.5,
            lon_deg: 121.5,
        };
        let params = LosParams {
            observer_height_m: 100.0,
            max_range_m: 30_000.0,
        };
        let full = compute_los(&data, &origin, &params);
        let coarse =
            RangeComputation::new(&data, &origin, &params, RangeKind::Visible, 4).run(&data);
        assert_eq!(coarse.len(), NUM_AZIMUTHS / 4);
        for (j, p) in coarse.iter().enumerate() {
            assert_eq!(*p, full[j * 4]);
        }

        let elevations = [0.0, 5.0];
        let full = compute_los_dome(&data, &origin, &params, &elevations);
        let mut coarse = DomeComputation::new(&data, &origin, &params, &elevations, 8);
        coarse.advance(&data, usize::MAX);
        let coarse = coarse.finish();
        for (ring_full, ring_coarse) in full.iter().zip(&coarse) {
            assert_eq!(ring_coarse.points.len(), NUM_AZIMUTHS / 8);
            for (j, p) in ring_coarse.points.iter().enumerate() {
                assert_eq!(*p, ring_full.points[j * 8]);
            }
        }
    }

    #[test]
    fn dome_rings_are_full_spheres_over_flat_ground() {
        let (data, origin) = flat();
        let params = LosParams {
            observer_height_m: 10.0,
            max_range_m: 30_000.0,
        };
        let rings = compute_los_dome(&data, &origin, &params, &[0.0, 5.0, 30.0]);
        assert_eq!(rings.len(), 3);
        for ring in &rings {
            assert_eq!(ring.points.len(), NUM_AZIMUTHS);
            // 遮蔽がなければ、どの仰角でも最大観測範囲(スラントレンジ)ちょうどの球面になる。
            for p in ring.points.iter().step_by(1000) {
                assert!(
                    (p.range_m - 30_000.0).abs() < 1.0,
                    "el={} az={} r={}",
                    ring.elevation_deg,
                    p.azimuth_deg,
                    p.range_m
                );
            }
        }
    }
}
