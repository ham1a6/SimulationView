//! 地表の測地線を一定の楕円体高へ持ち上げた飛行経路。設計書9.3.2節。
//!
//! 考え方: 地表の測地線上の各点を、その点の楕円体の法線方向へ高さhだけ持ち上げた曲線の長さを求める。
//! 高さhの曲面上では、北向き・東向きの長さの伸び率がそれぞれ(1+h/M)・(1+h/N)になる
//! (M: 子午線曲率半径、N: 卯酉線曲率半径)。地表の線素の方位αから持ち上げた線素の長さが決まるので、
//! それを測地線に沿って数値積分する(`lifted_length`)。

use geographiclib_rs::{DirectGeodesic, Geodesic};

use super::{measure, InvalidRoutePoint, RouteMeasurement};

/// 一定高度での経路測定に失敗した理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlightMeasurementError {
    /// 高度が非有限値、または対応範囲0〜1,000,000mの外。
    InvalidHeight,
    /// 不正な緯度経度。点番号は0始まり。
    InvalidPoint(InvalidRoutePoint),
}

impl std::fmt::Display for FlightMeasurementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHeight => write!(f, "楕円体高は0〜1,000,000mの有限値で指定してください"),
            Self::InvalidPoint(point) => point.fmt(f),
        }
    }
}

impl std::error::Error for FlightMeasurementError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidPoint(point) => Some(point),
            Self::InvalidHeight => None,
        }
    }
}

/// 地表の最短測地線の真上を、一定のWGS84楕円体高(m)で飛ぶ経路を測定する。
///
/// `height_m`は0〜1,000,000mの有限値。海抜高度・対地高度・気圧高度ではない。
/// カーニー法で得た地表経路を楕円体の法線方向へ持ち上げ、弧長を数値積分する。
/// 高度面上で最短経路を再探索するものでも、2点間の空間直線距離でもない。
/// 地形との衝突や障害物は判定しない。海抜高度を使う場合は呼び出し側でジオイド高を考慮する。
///
/// 緯度経度の検証・空経路・不定方位の扱いは[`measure_route`](super::measure_route)と同じ。
/// 距離・累積距離・初期方位は飛行経路の値。高度0では`measure_route`と完全に一致する。
/// 最短測地線が複数ある場合はGeographicLibが選んだ地表経路に沿う。
pub fn measure_route_at_height(
    points: &[(f64, f64)],
    height_m: f64,
) -> Result<RouteMeasurement, FlightMeasurementError> {
    // 範囲の判定はNaN・無限大も弾く。
    if !(0.0..=1_000_000.0).contains(&height_m) {
        return Err(FlightMeasurementError::InvalidHeight);
    }
    // まず地表の経路を測る(入力の検証も兼ねる)。高さ0ならそのまま返す(積分の丸め誤差で
    // 地表の値とずれないように)。
    let (mut route, bearings) = measure(points).map_err(FlightMeasurementError::InvalidPoint)?;
    if height_m == 0.0 {
        return Ok(route);
    }
    let geodesic = Geodesic::wgs84();
    let mut total = 0.0;
    for ((segment, &(lat, lon)), bearing) in route.segments.iter_mut().zip(points).zip(bearings) {
        // Noneの方位でも距離は定義できるため、GeographicLibの規約に従う逆解の方位を使う。
        segment.distance_m =
            lifted_length(&geodesic, lat, lon, bearing, segment.distance_m, height_m);
        // 持ち上げると北・東の伸び率が違うので、飛行経路の方位は地表の方位からわずかにずれる。
        segment.initial_bearing_deg = segment.initial_bearing_deg.map(|azimuth| {
            let (north, east) = tangent_components(&geodesic, lat, azimuth, height_m);
            east.atan2(north).to_degrees().rem_euclid(360.0)
        });
        total += segment.distance_m;
        segment.cumulative_distance_m = total;
    }
    route.total_distance_m = total;
    Ok(route)
}

/// 地表の単位接線を高度hへ持ち上げたときの北・東成分。
/// ds_h² = ((1+h/M) cosα)² ds² + ((1+h/N) sinα)² ds²。
fn tangent_components(geodesic: &Geodesic, lat: f64, azimuth: f64, h: f64) -> (f64, f64) {
    // W² = 1-e²sin²φ、卯酉線曲率半径N = a/W、子午線曲率半径M = a(1-e²)/W³ = N(1-e²)/W²。
    let f = geodesic.flattening();
    let e2 = f * (2.0 - f);
    let w2 = 1.0 - e2 * lat.to_radians().sin().powi(2);
    let n = geodesic.equatorial_radius() / w2.sqrt();
    let m = n * (1.0 - e2) / w2;
    let (sin_azimuth, cos_azimuth) = azimuth.to_radians().sin_cos();
    ((1.0 + h / m) * cos_azimuth, (1.0 + h / n) * sin_azimuth)
}

/// (`lat`, `lon`)から初期方位`bearing`(度)で長さ`distance`(m)の地表の測地線を、高さ`h`へ持ち上げた
/// 曲線の長さ。測地線上の各点の方位と緯度から線素の伸び率(`tangent_components`の長さ)を求めて積分する。
fn lifted_length(
    geodesic: &Geodesic,
    lat: f64,
    lon: f64,
    bearing: f64,
    distance: f64,
    h: f64,
) -> f64 {
    if distance == 0.0 {
        return 0.0;
    }
    // 最大100kmの各区間を4点Gauss–Legendre積分。端点・極での差分を使わない。
    const QUADRATURE: [(f64, f64); 4] = [
        (-0.8611363115940526, 0.3478548451374538),
        (-0.3399810435848563, 0.6521451548625461),
        (0.3399810435848563, 0.6521451548625461),
        (0.8611363115940526, 0.3478548451374538),
    ];
    let count = (distance / 100_000.0).ceil() as usize;
    let step = distance / count as f64;
    let mut length = 0.0;
    for i in 0..count {
        // 区間[i*step, (i+1)*step]の中点を中心に、[-1, 1]のガウス点を区間の半幅で伸ばして置く。
        let midpoint = (i as f64 + 0.5) * step;
        for (node, weight) in QUADRATURE {
            let s = midpoint + node * step * 0.5;
            // 順問題で、始点から測地線に沿って距離sの点の(緯度, 経度, その点での方位)を得る。
            let (sample_lat, _, azimuth): (f64, f64, f64) = geodesic.direct(lat, lon, bearing, s);
            let (north, east) = tangent_components(geodesic, sample_lat, azimuth, h);
            length += weight * north.hypot(east) * step * 0.5;
        }
    }
    length
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::measurement::measure_route;

    #[test]
    fn zero_height_and_degenerate_routes() {
        for points in [
            vec![],
            vec![(35.0, 139.0)],
            vec![(90.0, 0.0), (90.0, 180.0)],
            vec![(35.0, 139.0), (40.0, -170.0), (-20.0, -70.0)],
        ] {
            assert_eq!(
                measure_route_at_height(&points, 0.0).unwrap(),
                measure_route(&points).unwrap()
            );
        }
        for points in [
            vec![],
            vec![(35.0, 139.0)],
            vec![(90.0, 0.0), (90.0, 180.0)],
        ] {
            assert_eq!(
                measure_route_at_height(&points, 10_000.0)
                    .unwrap()
                    .total_distance_m,
                0.0
            );
        }
    }

    #[test]
    fn equatorial_and_meridional_lengths_match_analytic_values() {
        let h = 10_000.0;
        let equator = measure_route_at_height(&[(0.0, 179.0), (0.0, -179.0)], h).unwrap();
        let expected = (6_378_137.0 + h) * 2.0_f64.to_radians();
        assert!((equator.total_distance_m - expected).abs() < 1e-6);
        assert_eq!(equator.segments[0].initial_bearing_deg, Some(90.0));
        // 子午線の法線回転角は緯度差に等しく、追加の弧長はh*Δφ。
        for points in [[(-70.0, 139.0), (80.0, 139.0)], [(0.0, 0.0), (0.0, 180.0)]] {
            let surface = measure_route(&points).unwrap().total_distance_m;
            let angle = if points[1].1 == 180.0 {
                std::f64::consts::PI
            } else {
                150.0_f64.to_radians()
            };
            let flight = measure_route_at_height(&points, h).unwrap();
            assert!((flight.total_distance_m - surface - h * angle).abs() < 1e-6);
        }
    }

    #[test]
    fn validates_height_and_coordinates() {
        for h in [-1.0, 1_000_001.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                measure_route_at_height(&[], h),
                Err(FlightMeasurementError::InvalidHeight)
            );
        }
        assert_eq!(
            measure_route_at_height(&[(0.0, 0.0), (91.0, 0.0)], 100.0),
            Err(FlightMeasurementError::InvalidPoint(InvalidRoutePoint {
                index: 1
            }))
        );
    }

    #[test]
    fn flight_lengths_match_independent_ecef_chords() {
        // 積分式とは独立にECEF折れ線を作り、分割数の異なる弦長を外挿する。
        fn chord_sum(start: (f64, f64), bearing: f64, distance: f64, h: f64, count: usize) -> f64 {
            let g = Geodesic::wgs84();
            let mut previous = None;
            let mut sum = 0.0;
            for i in 0..=count {
                let (lat, lon): (f64, f64) = g.direct(
                    start.0,
                    start.1,
                    bearing,
                    distance * i as f64 / count as f64,
                );
                let (slat, clat) = lat.to_radians().sin_cos();
                let (slon, clon) = lon.to_radians().sin_cos();
                let f = 1.0 / 298.257223563;
                let e2 = f * (2.0 - f);
                let n = 6_378_137.0 / (1.0 - e2 * slat * slat).sqrt();
                let point = glam::DVec3::new(
                    (n + h) * clat * clon,
                    (n + h) * clat * slon,
                    (n * (1.0 - e2) + h) * slat,
                );
                if let Some(prev) = previous {
                    sum += point.distance(prev);
                }
                previous = Some(point);
            }
            sum
        }
        for (start, end) in [
            ((35.0, 139.0), (48.0, -123.0)),
            ((-41.32, 174.81), (40.96, -5.5)),
            ((80.0, 0.0), (80.0, 180.0)),
        ] {
            let (d, b) = super::super::distance_and_bearing(start.0, start.1, end.0, end.1);
            for h in [10_000.0, 1_000_000.0] {
                let coarse = chord_sum(start, b, d, h, 2000);
                let fine = chord_sum(start, b, d, h, 4000);
                let reference = (4.0 * fine - coarse) / 3.0;
                let flight = measure_route_at_height(&[start, end], h).unwrap();
                assert!((flight.total_distance_m - reference).abs() < 0.001);
                let reverse = measure_route_at_height(&[end, start], h).unwrap();
                assert!((flight.total_distance_m - reverse.total_distance_m).abs() < 1e-6);
                assert!(flight.total_distance_m > d);
            }
        }
        let route =
            measure_route_at_height(&[(35.0, 139.0), (48.0, -123.0), (48.0, -123.0)], 10_000.0)
                .unwrap();
        assert_eq!(route.segments[1].distance_m, 0.0);
        assert_eq!(
            route.segments[0].cumulative_distance_m,
            route.total_distance_m
        );
        assert_eq!(
            route.segments[1].cumulative_distance_m,
            route.total_distance_m
        );
    }
}
