//! 球面近似による位置関係の計算。楕円体上の厳密な測地距離ではない。

/// 緯度・経度(度)から大円距離(m)と初期方位(北から時計回り、0以上360未満の度)を返す。
/// 平均半径6371kmの球面近似。入力は有限値、緯度は-90〜90度を前提とする。
/// 同一点・対蹠点では方位は不定のため、返された方位を使用しないこと。
pub fn distance_and_bearing(lat0: f64, lon0: f64, lat1: f64, lon1: f64) -> (f64, f64) {
    const EARTH_RADIUS_M: f64 = 6_371_000.0;
    let (p0, p1) = (lat0.to_radians(), lat1.to_radians());
    let dlon = (lon1 - lon0).to_radians();
    let a = ((p1 - p0) * 0.5).sin().powi(2) + p0.cos() * p1.cos() * (dlon * 0.5).sin().powi(2);
    let distance = 2.0 * EARTH_RADIUS_M * a.clamp(0.0, 1.0).sqrt().asin();
    let bearing =
        (dlon.sin() * p1.cos()).atan2(p0.cos() * p1.sin() - p0.sin() * p1.cos() * dlon.cos());
    (distance, bearing.to_degrees().rem_euclid(360.0))
}

/// 経路の1区間の測定結果。距離は高度・地形の起伏を含まない球面上の距離。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteSegment {
    /// この区間の長さ(m)。
    pub distance_m: f64,
    /// 経路の始点からこの区間の終点までの累積距離(m)。
    pub cumulative_distance_m: f64,
    /// 北から時計回りの初期方位(度)。同一点・対蹠点付近では不定のためNone。
    pub initial_bearing_deg: Option<f64>,
}

/// 入力順に隣接する地点を結ぶ経路の測定結果。閉路にする場合は始点を末尾にも渡す。
#[derive(Debug, Clone, PartialEq)]
pub struct RouteMeasurement {
    pub segments: Vec<RouteSegment>,
    pub total_distance_m: f64,
}

/// 経路内の不正な座標。点の番号は0始まり。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidRoutePoint {
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
/// 平均半径6371kmの球面近似で、標高や地形に沿った距離ではない。
/// 日付変更線をまたぐ区間も短い方の大円弧を測る。
/// 角距離が同一点・対蹠点から1e-7 rad以内では方位をNoneにする。
pub fn measure_route(points: &[(f64, f64)]) -> Result<RouteMeasurement, InvalidRoutePoint> {
    for (index, &(lat, lon)) in points.iter().enumerate() {
        if !lat.is_finite()
            || !lon.is_finite()
            || !(-90.0..=90.0).contains(&lat)
            || !(-180.0..=180.0).contains(&lon)
        {
            return Err(InvalidRoutePoint { index });
        }
    }
    let mut total_distance_m = 0.0;
    let segments = points
        .windows(2)
        .map(|pair| {
            let (lat0, lon0) = pair[0];
            let (lat1, lon1) = pair[1];
            let (distance_m, bearing) = distance_and_bearing(lat0, lon0, lat1, lon1);
            let angle = distance_m / 6_371_000.0;
            let initial_bearing_deg =
                (angle > 1e-7 && std::f64::consts::PI - angle > 1e-7).then_some(bearing);
            total_distance_m += distance_m;
            RouteSegment {
                distance_m,
                cumulative_distance_m: total_distance_m,
                initial_bearing_deg,
            }
        })
        .collect();
    Ok(RouteMeasurement {
        segments,
        total_distance_m,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_crosses_date_line_and_accumulates_distances() {
        let result = measure_route(&[(0.0, 179.0), (0.0, -180.0), (1.0, -180.0)]).unwrap();
        assert_eq!(result.segments.len(), 2);
        assert!((result.total_distance_m - 222_389.8533).abs() < 0.01);
        assert!((result.segments[0].cumulative_distance_m - 111_194.9266).abs() < 0.01);
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
        assert!((result.total_distance_m - 20_015_086.796).abs() < 0.01);
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
            assert!((d - 111_194.9266).abs() < 0.01);
            assert!((b - bearing).abs() < 1e-8);
        }
        let (d, b) = distance_and_bearing(35.0, 139.0, 35.5, 139.0);
        assert!((d - 55_597.4633).abs() < 0.01);
        assert!(b.abs() < 1e-8);
    }

    #[test]
    fn coincident_and_antipodal_points_are_finite() {
        assert_eq!(distance_and_bearing(35.0, 139.0, 35.0, 139.0).0, 0.0);
        let (d, _) = distance_and_bearing(0.0, 0.0, 0.0, 180.0);
        assert!((d - std::f64::consts::PI * 6_371_000.0).abs() < 0.01);
    }
}
