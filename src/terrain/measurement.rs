//! WGS84楕円体上の位置関係の計算(カーニー法)。設計書9.3.1節・9.3.2節。
//!
//! 地形データを読まずに使える公開API: 2点間の距離と方位(`distance_and_bearing`)、経路の区間・累積距離
//! (`measure_route`)、一定の楕円体高を飛ぶ経路の距離(`measure_route_at_height`)。
//! 計算は`geographiclib-rs`の測地線(厳密な楕円体の解)に任せ、ここでは入力の検証・方位の正規化・
//! 累積の集計だけを行う(作図の形を作る`geodesy`の球面近似とは別の、精度を優先した計算)。

use geographiclib_rs::{Geodesic, InverseGeodesic};

mod flight;
pub use flight::{measure_route_at_height, FlightMeasurementError};

/// 緯度・経度(度)からWGS84楕円体上の最短測地距離(m)と初期方位(北から時計回り、0以上360未満の度)を返す。
/// カーニー法を使う。入力は有限値、緯度は-90〜90度を前提とする。
/// 同一点・複数の最短測地線が存在する場合の方位はGeographicLibの規約値で、一意ではない。
pub fn distance_and_bearing(lat0: f64, lon0: f64, lat1: f64, lon1: f64) -> (f64, f64) {
    // 逆問題の戻り値は(距離s12, 始点の方位azi1, 終点の方位azi2, 補助球上の弧長a12)。
    // 方位は-180〜180度で返るので、0以上360未満へ寄せる。
    let (distance, bearing, _, _): (f64, f64, f64, f64) =
        Geodesic::wgs84().inverse(lat0, lon0, lat1, lon1);
    (distance, bearing.rem_euclid(360.0))
}

/// 経路の1区間の測定結果。距離・方位の基準は呼び出した測定関数による。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteSegment {
    /// この区間の長さ(m)。
    pub distance_m: f64,
    /// 経路の始点からこの区間の終点までの累積距離(m)。
    pub cumulative_distance_m: f64,
    /// 北から時計回りの初期方位(度)。補助球の弧長が0・π付近では保守的にNone。
    pub initial_bearing_deg: Option<f64>,
}

/// 入力順に隣接する地点を結ぶ経路の測定結果。閉路にする場合は始点を末尾にも渡す。
#[derive(Debug, Clone, PartialEq)]
pub struct RouteMeasurement {
    /// 区間(点i→点i+1)ごとの結果。点の数-1個(0〜1点なら空)。
    pub segments: Vec<RouteSegment>,
    /// 全区間の長さの合計(m)。最後の区間の`cumulative_distance_m`と同じ値。
    pub total_distance_m: f64,
}

/// 経路内の不正な座標。点の番号は0始まり。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidRoutePoint {
    /// 最初に見つかった不正な点の番号。
    pub index: usize,
}

impl std::fmt::Display for InvalidRoutePoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "経路の点{}の緯度経度が範囲外または非有限値です",
            self.index
        )
    }
}

impl std::error::Error for InvalidRoutePoint {}

/// (緯度, 経度)の列から区間距離・累積距離・初期方位を計算する。
///
/// 度単位で緯度は[-90, 90]、経度は[-180, 180]の有限値を受け付ける。
/// 不正な点があれば最初の点の番号を返す。0〜1点では距離0・区間なし。
/// カーニー法によるWGS84楕円体上の最短測地距離で、標高や地形に沿った距離ではない。
/// 日付変更線や対蹠点付近をまたぐ区間にも対応する。
/// 方位の不定・不安定な領域を避けるため、補助球上の弧長が0・πから1e-7 rad以内では
/// 保守的に方位をNoneにする(距離は常に計算する)。
pub fn measure_route(points: &[(f64, f64)]) -> Result<RouteMeasurement, InvalidRoutePoint> {
    measure(points).map(|(route, _)| route)
}

/// `measure_route`の本体。区間ごとのGeographicLibの初期方位(度、正規化も不定の判定もしない生の値)も返す。
fn measure(points: &[(f64, f64)]) -> Result<(RouteMeasurement, Vec<f64>), InvalidRoutePoint> {
    // 範囲の判定はNaN・無限大も弾く。
    if let Some(index) = points
        .iter()
        .position(|&(lat, lon)| !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon))
    {
        return Err(InvalidRoutePoint { index });
    }
    let mut total_distance_m = 0.0;
    let geodesic = Geodesic::wgs84();
    let (segments, bearings) = points
        .windows(2)
        .map(|pair| {
            let (lat0, lon0) = pair[0];
            let (lat1, lon1) = pair[1];
            // (距離, 始点の方位, 終点の方位, 補助球上の弧長(度))。
            let (distance_m, bearing, _, arc_deg): (f64, f64, f64, f64) =
                geodesic.inverse(lat0, lon0, lat1, lon1);
            // 弧長が0(同一点)やπ(対蹠点)に近いと、方位は定まらないか入力のわずかな差で大きく変わる。
            let angle = arc_deg.to_radians();
            let initial_bearing_deg = (angle > 1e-7 && std::f64::consts::PI - angle > 1e-7)
                .then_some(bearing.rem_euclid(360.0));
            total_distance_m += distance_m;
            let segment = RouteSegment {
                distance_m,
                cumulative_distance_m: total_distance_m,
                initial_bearing_deg,
            };
            (segment, bearing)
        })
        .unzip();
    let route = RouteMeasurement {
        segments,
        total_distance_m,
    };
    Ok((route, bearings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_antipodal_route_matches_geographiclib_reference() {
        // GeographicLib公式例のWellington→Salamanca。球面近似では一致しない。
        // https://geographiclib.sourceforge.io/html/python/examples.html
        let (distance, bearing) = distance_and_bearing(-41.32, 174.81, 40.96, -5.50);
        assert!((distance - 19_959_679.267).abs() < 0.001);
        assert!(bearing.is_finite() && (0.0..360.0).contains(&bearing));
        let route = measure_route(&[(-41.32, 174.81), (40.96, -5.50)]).unwrap();
        assert_eq!(route.total_distance_m, distance);
        assert_eq!(route.segments[0].initial_bearing_deg, Some(bearing));
        let reverse = distance_and_bearing(40.96, -5.50, -41.32, 174.81);
        assert!((reverse.0 - distance).abs() < 1e-6);
    }

    #[test]
    fn route_crosses_date_line_and_accumulates_distances() {
        let result = measure_route(&[(0.0, 179.0), (0.0, -180.0), (1.0, -180.0)]).unwrap();
        assert_eq!(result.segments.len(), 2);
        assert!((result.total_distance_m - 221_893.87935107236).abs() < 1e-6);
        assert!((result.segments[0].cumulative_distance_m - 111_319.49079327357).abs() < 1e-6);
        assert_eq!(
            result.segments[1].cumulative_distance_m,
            result.total_distance_m
        );
        assert_eq!(result.segments[0].initial_bearing_deg, Some(90.0));
        assert_eq!(result.segments[1].initial_bearing_deg, Some(0.0));
    }

    #[test]
    fn route_handles_empty_single_repeated_and_antipodal_points() {
        for points in [vec![], vec![(35.0, 139.0)]] {
            let result = measure_route(&points).unwrap();
            assert!(result.segments.is_empty());
            assert_eq!(result.total_distance_m, 0.0);
        }
        let result = measure_route(&[(0.0, 0.0), (0.0, 0.0), (0.0, 180.0)]).unwrap();
        assert_eq!(result.segments[0].distance_m, 0.0);
        assert!(result
            .segments
            .iter()
            .all(|s| s.initial_bearing_deg.is_none()));
        assert!((result.total_distance_m - 20_003_931.458625447).abs() < 1e-6);
        let poles = measure_route(&[(90.0, -180.0), (90.0, 180.0)]).unwrap();
        assert!(poles.segments[0].initial_bearing_deg.is_none());
    }

    #[test]
    fn route_reports_first_invalid_point_even_without_segments() {
        for point in [
            (91.0, 0.0),
            (-91.0, 0.0),
            (0.0, 181.0),
            (0.0, -181.0),
            (f64::NAN, 0.0),
            (0.0, f64::INFINITY),
        ] {
            assert_eq!(measure_route(&[point]), Err(InvalidRoutePoint { index: 0 }));
            assert_eq!(
                measure_route(&[(0.0, 0.0), point, point]),
                Err(InvalidRoutePoint { index: 1 })
            );
        }
    }

    #[test]
    fn cardinal_directions_and_date_line() {
        for (lon0, lon1, bearing) in [(0.0, 1.0, 90.0), (1.0, 0.0, 270.0), (179.5, -179.5, 90.0)] {
            let (d, b) = distance_and_bearing(0.0, lon0, 0.0, lon1);
            assert!((d - 111_319.49079327357).abs() < 1e-6);
            assert!((b - bearing).abs() < 1e-8);
        }
        let (d, b) = distance_and_bearing(0.0, 139.0, 1.0, 139.0);
        assert!((d - 110_574.38855779878).abs() < 1e-6);
        assert!(b.abs() < 1e-8);
    }

    #[test]
    fn coincident_and_antipodal_points_are_finite() {
        assert_eq!(distance_and_bearing(35.0, 139.0, 35.0, 139.0).0, 0.0);
        let (d, _) = distance_and_bearing(0.0, 0.0, 0.0, 180.0);
        assert!((d - 20_003_931.458625447).abs() < 1e-6);
    }
}
