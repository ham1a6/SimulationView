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

#[cfg(test)]
mod tests {
    use super::*;

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
