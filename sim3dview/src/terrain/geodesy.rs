//! 測地座標の変換。DETAILED_DESIGN.md 3.2節(ENU変換)。
//! 楕円体(`Ellipsoid`)と、緯度経度(+標高)からENU座標(東=X, 北=Y, 上=Z)への変換(`EnuTransform`)。

use glam::{DMat3, DVec3};
use serde::Deserialize;

use super::origin::Origin;

/// 地球の楕円体(長半径と逆扁平率)。`metadata.json`の`ellipsoid`と同じ形。
#[derive(Debug, Clone, Deserialize)]
pub struct Ellipsoid {
    pub a_m: f64,
    pub inv_f: f64,
}

impl Ellipsoid {
    /// WGS84。地形データ(ALOS DSM)の`metadata.json`もこの値。地形データを読む前に楕円体が必要な
    /// 場面(作図の距離・方位の計算など)と、単体テストで使う。
    pub const WGS84: Ellipsoid = Ellipsoid { a_m: 6_378_137.0, inv_f: 298.257_222_101 };
}

/// 緯度経度(+標高)からENU座標(東=X, 北=Y, 上=Z)へ変換する。DETAILED_DESIGN.md 3.2節の変換式そのもの。
/// C++側 sim_server の座標系定義と完全に一致させること(3.4節: サーバーとフロントで同一座標系)。
pub struct EnuTransform {
    origin_lat_rad: f64,
    origin_lon_rad: f64,
    /// 原点のECEF座標。
    origin_ecef: DVec3,
    /// ECEFの差分→ENUの回転(行が東・北・上の単位ベクトル)。原点の緯度経度の三角関数は、
    /// `transform_f64`・`enu_to_geodetic`が毎回求め直すと覆域ドームの頂点変換のように大量に呼ぶ
    /// 場面で重いので、`new`で1回だけ求めておく。
    enu_from_ecef: DMat3,
    a: f64,
    e2: f64,
    /// 原点緯度における子午線曲率半径(`inverse`用。呼び出しごとに求め直すと見通し計算で
    /// 数百万回呼ぶため重いので、`new`で1回だけ求めておく)。
    meridian_radius: f64,
    /// 原点緯度における「卯酉線曲率半径*cos(緯度)」(=原点緯度の緯線の半径、`inverse`用)。
    parallel_radius: f64,
}

impl EnuTransform {
    pub fn new(origin: &Origin, ellipsoid: &Ellipsoid) -> Self {
        let a = ellipsoid.a_m;
        let f = 1.0 / ellipsoid.inv_f;
        let e2 = f * (2.0 - f);
        let lat0 = origin.lat_deg.to_radians();
        let lon0 = origin.lon_deg.to_radians();
        let (x0, y0, z0) = geodetic_to_ecef(lat0, lon0, 0.0, a, e2);
        let (sin_lat0, cos_lat0) = lat0.sin_cos();
        let (sin_lon0, cos_lon0) = lon0.sin_cos();
        let denom = (1.0 - e2 * sin_lat0 * sin_lat0).sqrt();
        // 東 = (-sinλ, cosλ, 0)、北 = (-sinφcosλ, -sinφsinλ, cosφ)、上 = (cosφcosλ, cosφsinλ, sinφ)を行にした行列
        // (glamは列優先なので、列を並べて作る)。
        let enu_from_ecef = DMat3::from_cols(
            DVec3::new(-sin_lon0, -sin_lat0 * cos_lon0, cos_lat0 * cos_lon0),
            DVec3::new(cos_lon0, -sin_lat0 * sin_lon0, cos_lat0 * sin_lon0),
            DVec3::new(0.0, cos_lat0, sin_lat0),
        );
        Self {
            origin_lat_rad: lat0,
            origin_lon_rad: lon0,
            origin_ecef: DVec3::new(x0, y0, z0),
            enu_from_ecef,
            a,
            e2,
            meridian_radius: a * (1.0 - e2) / denom.powi(3),
            parallel_radius: a / denom * cos_lat0,
        }
    }

    /// 楕円体の長半径と第一離心率の二乗(`mesh`が、行ごとの卯酉線曲率半径を求めるのに使う)。
    pub(crate) fn ellipsoid_params(&self) -> (f64, f64) {
        (self.a, self.e2)
    }

    /// ECEF座標(メートル)をENU座標(東, 北, 上)へ変換する(原点のECEF座標を引いて回転する)。
    pub(crate) fn ecef_to_enu(&self, x: f64, y: f64, z: f64) -> [f64; 3] {
        (self.enu_from_ecef * (DVec3::new(x, y, z) - self.origin_ecef)).to_array()
    }

    pub fn transform(&self, lat_deg: f64, lon_deg: f64, h: f64) -> [f32; 3] {
        let [east, north, up] = self.transform_f64(lat_deg, lon_deg, h);
        [east as f32, north as f32, up as f32]
    }

    /// 水域レイヤー(WGS84楕円体の海抜0mの面、`terrain.wgsl`の`fs_water`)の視線との交点判定に使う係数
    /// (行列M(3行、各行はvec4の先頭3要素を使う), g・c0)。ENU座標の点pに対し、楕円体の陰関数は
    /// `f(p) = c0 + 2 g・(M p) + |M p|^2`(f=0が楕円体の面、f<0が内側)。原点は楕円体上(h=0)に
    /// あるので定数項が(丸め誤差の)c0だけになり、原点から遠い点でも桁落ちしない(ECEFの絶対座標
    /// 約6.4e6mのままf32で二乗すると数mの誤差になる)。
    /// M = diag(1/a,1/a,1/b) * R(ENU→ECEFの回転)、g = diag(1/a,1/a,1/b) * 原点のECEF。c0は、シェーダーが
    /// 使うf32に丸めたgについて`|g|^2-1`をf64で求めたもの(丸め誤差で原点が面からずれない)。
    pub fn ellipsoid_shader_params(&self) -> ([[f32; 4]; 3], [f32; 4]) {
        let b = self.a * (1.0 - self.e2).sqrt();
        let (da, db) = (1.0 / self.a, 1.0 / b);
        // ENU→ECEFの回転は`enu_from_ecef`の転置なので、その行は`enu_from_ecef`の列。
        let r = self.enu_from_ecef.transpose();
        let rows = [r.row(0) * da, r.row(1) * da, r.row(2) * db];
        let m = rows.map(|r| [r.x as f32, r.y as f32, r.z as f32, 0.0]);
        let g = [
            (self.origin_ecef.x * da) as f32,
            (self.origin_ecef.y * da) as f32,
            (self.origin_ecef.z * db) as f32,
        ];
        let c0 = g.iter().map(|&v| v as f64 * v as f64).sum::<f64>() - 1.0;
        (m, [g[0], g[1], g[2], c0 as f32])
    }

    /// `transform`のf64版(遠方の地表の上座標を丸めずに扱いたい呼び出し側用)。
    pub fn transform_f64(&self, lat_deg: f64, lon_deg: f64, h: f64) -> [f64; 3] {
        let (x, y, z) = geodetic_to_ecef(lat_deg.to_radians(), lon_deg.to_radians(), h, self.a, self.e2);
        self.ecef_to_enu(x, y, z)
    }

    /// `transform`の厳密な逆: ENU座標(東, 北, 上。メートル)から(緯度, 経度, 楕円体高)を求める。
    /// ENU→ECEF(原点の回転行列の転置)→測地座標(反復法)。原点から数千km離れた点でも
    /// 地球の丸み・楕円体を正しく扱う(下の`inverse`は原点近傍の接平面近似)。
    pub fn enu_to_geodetic(&self, east: f64, north: f64, up: f64) -> (f64, f64, f64) {
        let ecef = self.origin_ecef + self.enu_from_ecef.transpose() * DVec3::new(east, north, up);
        let (x, y, z) = (ecef.x, ecef.y, ecef.z);

        let p = x.hypot(y);
        let lon = y.atan2(x);
        let mut lat = z.atan2(p * (1.0 - self.e2));
        let mut h = 0.0;
        for _ in 0..6 {
            let n = self.a / (1.0 - self.e2 * lat.sin().powi(2)).sqrt();
            h = p / lat.cos() - n;
            lat = z.atan2(p * (1.0 - self.e2 * n / (n + h)));
        }
        (lat.to_degrees(), lon.to_degrees(), h)
    }

    /// `transform`の逆(近似): 原点からのENUオフセット(東, 北。メートル)から緯度経度を求める。
    /// ローカル接平面近似(原点緯度における子午線・卯酉線曲率半径を使う)。断面図
    /// (`terrain/profile.rs`)で、原点から方位角方向へ地表をサンプリングするために使う。
    pub fn inverse(&self, east: f64, north: f64) -> (f64, f64) {
        let lat = self.origin_lat_rad + north / self.meridian_radius;
        let lon = self.origin_lon_rad + east / self.parallel_radius;
        (lat.to_degrees(), lon.to_degrees())
    }
}

fn geodetic_to_ecef(lat: f64, lon: f64, h: f64, a: f64, e2: f64) -> (f64, f64, f64) {
    let n = a / (1.0 - e2 * lat.sin().powi(2)).sqrt();
    let x = (n + h) * lat.cos() * lon.cos();
    let y = (n + h) * lat.cos() * lon.sin();
    let z = (n * (1.0 - e2) + h) * lat.sin();
    (x, y, z)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transform_at(lat: f64, lon: f64) -> EnuTransform {
        EnuTransform::new(&Origin { lat_deg: lat, lon_deg: lon }, &Ellipsoid::WGS84)
    }

    // 原点から数千km離れた点でも、transform_f64とenu_to_geodeticが往復で一致すること。
    #[test]
    fn enu_round_trip_far_from_origin() {
        let t = transform_at(35.355556, 138.859722);
        for &(lat, lon, h) in &[(24.34, 124.16, 0.0), (33.0, 130.0, 500.0), (37.5, 127.0, 100.0), (49.0, 121.0, 3000.0)] {
            let [e, n, u] = t.transform_f64(lat, lon, h);
            let (lat2, lon2, h2) = t.enu_to_geodetic(e, n, u);
            assert!((lat - lat2).abs() < 1e-9, "lat {lat} vs {lat2}");
            assert!((lon - lon2).abs() < 1e-9, "lon {lon} vs {lon2}");
            assert!((h - h2).abs() < 1e-3, "h {h} vs {h2}");
        }
    }

    // 丸みで遠方の地表が下がる量が概算(d^2/2R)と同程度であること(石垣島は原点から約2,000km)。
    #[test]
    fn far_ground_curves_down() {
        let t = transform_at(35.355556, 138.859722);
        let [e, n, u] = t.transform_f64(24.34, 124.16, 0.0);
        let d = e.hypot(n);
        assert!(d > 1_800_000.0 && d < 2_000_000.0, "d={d}");
        assert!(u < -250_000.0 && u > -350_000.0, "u={u}");
    }

    // 原点そのものは(0, 0, 標高)になり、東・北・上の向きが正しいこと。
    #[test]
    fn axes_point_east_north_up() {
        let t = transform_at(35.0, 135.0);
        let [e, n, u] = t.transform_f64(35.0, 135.0, 250.0);
        assert!(e.abs() < 1e-6 && n.abs() < 1e-6 && (u - 250.0).abs() < 1e-6, "{e} {n} {u}");
        let [e, n, _] = t.transform_f64(35.0, 135.001, 0.0);
        assert!(e > 80.0 && e < 100.0 && n.abs() < 0.1, "east: {e} {n}"); // 経度0.001度 ≒ 91m
        let [e, n, _] = t.transform_f64(35.001, 135.0, 0.0);
        assert!(n > 100.0 && n < 120.0 && e.abs() < 0.1, "north: {e} {n}"); // 緯度0.001度 ≒ 110m
    }

    // 水域シェーダーの係数: 原点(楕円体上)では陰関数がほぼ0、原点の真上・真下で符号が変わること。
    #[test]
    fn shader_params_describe_the_ellipsoid_surface() {
        let t = transform_at(35.0, 135.0);
        let (m, g) = t.ellipsoid_shader_params();
        let f = |p: [f64; 3]| -> f64 {
            let mp: Vec<f64> = m
                .iter()
                .map(|row| row[0] as f64 * p[0] + row[1] as f64 * p[1] + row[2] as f64 * p[2])
                .collect();
            let dot = g[0] as f64 * mp[0] + g[1] as f64 * mp[1] + g[2] as f64 * mp[2];
            g[3] as f64 + 2.0 * dot + mp.iter().map(|v| v * v).sum::<f64>()
        };
        assert!(f([0.0, 0.0, 0.0]).abs() < 1e-6);
        assert!(f([0.0, 0.0, 1000.0]) > 0.0); // 面の外(上空)
        assert!(f([0.0, 0.0, -1000.0]) < 0.0); // 面の内側(地下)
        // 遠方でも、その地点の楕円体上の点(標高0m)で0に近い。
        let far = t.transform_f64(30.0, 130.0, 0.0);
        assert!(f(far).abs() < 1e-6, "f={}", f(far));
    }

    // 接平面近似の`inverse`は、原点の近く(20km程度)なら厳密な逆と数十m以内で一致する
    // (経度の差は、緯線の半径が原点の緯度のままという近似のぶん)。
    #[test]
    fn inverse_agrees_with_the_exact_inverse_near_the_origin() {
        let t = transform_at(35.0, 135.0);
        let (east, north) = (10_000.0, -20_000.0);
        let up = -(east * east + north * north) / (2.0 * 6_378_137.0); // 地表(標高0m)の上座標の概算
        let (lat, lon, _) = t.enu_to_geodetic(east, north, up);
        let (lat2, lon2) = t.inverse(east, north);
        assert!((lat - lat2).abs() < 1e-4, "lat {lat} vs {lat2}");
        assert!((lon - lon2).abs() < 5e-4, "lon {lon} vs {lon2}");
    }
}
