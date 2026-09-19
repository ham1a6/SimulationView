//! 地形メッシュ生成。DETAILED_DESIGN.md 3.2節(ENU変換)・6.5節(頂点構造)・6.7節(配色)。

use super::loader::{Ellipsoid, TerrainData};

/// 頂点構造(DETAILED_DESIGN.md 6.5節)。UV座標は使わず、標高由来の色を直接持たせる。
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TerrainVertex {
    pub position: [f32; 3], // x(East), y(North), z(Up) — ENU変換結果
    pub color: [f32; 3],
}

pub struct TerrainMesh {
    pub vertices: Vec<TerrainVertex>,
    pub indices: Vec<u32>,
}

/// 基準位置(原点)。DETAILED_DESIGN.md 3.1節。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Origin {
    pub lat_deg: f64,
    pub lon_deg: f64,
}

/// 緯度経度(+標高)からENU座標(東=X, 北=Y, 上=Z)へ変換する。DETAILED_DESIGN.md 3.2節の変換式そのもの。
/// C++側 sim_server の座標系定義と完全に一致させること(3.4節: サーバーとフロントで同一座標系)。
pub struct EnuTransform {
    origin_lat_rad: f64,
    origin_lon_rad: f64,
    origin_x: f64,
    origin_y: f64,
    origin_z: f64,
    a: f64,
    e2: f64,
}

impl EnuTransform {
    pub fn new(origin: &Origin, ellipsoid: &Ellipsoid) -> Self {
        let a = ellipsoid.a_m;
        let f = 1.0 / ellipsoid.inv_f;
        let e2 = f * (2.0 - f);
        let lat0 = origin.lat_deg.to_radians();
        let lon0 = origin.lon_deg.to_radians();
        let (x0, y0, z0) = geodetic_to_ecef(lat0, lon0, 0.0, a, e2);
        Self {
            origin_lat_rad: lat0,
            origin_lon_rad: lon0,
            origin_x: x0,
            origin_y: y0,
            origin_z: z0,
            a,
            e2,
        }
    }

    pub fn transform(&self, lat_deg: f64, lon_deg: f64, h: f64) -> [f32; 3] {
        let lat = lat_deg.to_radians();
        let lon = lon_deg.to_radians();
        let (x, y, z) = geodetic_to_ecef(lat, lon, h, self.a, self.e2);
        let dx = x - self.origin_x;
        let dy = y - self.origin_y;
        let dz = z - self.origin_z;

        let sin_lat0 = self.origin_lat_rad.sin();
        let cos_lat0 = self.origin_lat_rad.cos();
        let sin_lon0 = self.origin_lon_rad.sin();
        let cos_lon0 = self.origin_lon_rad.cos();

        let east = -sin_lon0 * dx + cos_lon0 * dy;
        let north = -sin_lat0 * cos_lon0 * dx - sin_lat0 * sin_lon0 * dy + cos_lat0 * dz;
        let up = cos_lat0 * cos_lon0 * dx + cos_lat0 * sin_lon0 * dy + sin_lat0 * dz;

        [east as f32, north as f32, up as f32]
    }

    /// `transform`の逆(近似): 原点からのENUオフセット(東, 北。メートル)から緯度経度を求める。
    /// ローカル接平面近似(原点緯度における子午線・卯酉線曲率半径を使う)。断面図
    /// (`terrain/profile.rs`)で、原点から方位角方向へ地表をサンプリングするために使う。
    pub fn inverse(&self, east: f64, north: f64) -> (f64, f64) {
        let sin_lat0 = self.origin_lat_rad.sin();
        let denom = (1.0 - self.e2 * sin_lat0 * sin_lat0).sqrt();
        let m = self.a * (1.0 - self.e2) / denom.powi(3); // 子午線曲率半径
        let n = self.a / denom; // 卯酉線曲率半径

        let lat = self.origin_lat_rad + north / m;
        let lon = self.origin_lon_rad + east / (n * self.origin_lat_rad.cos());
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

/// 標高を正規化し、低地(深緑)→高山(白に近い明色)の地形図的カラーランプへ写像する
/// (DETAILED_DESIGN.md 6.7節)。カラーストップは実装時に調整可能な固定テーブル。
fn elevation_to_color(elevation: f32, min: f32, max: f32) -> [f32; 3] {
    const STOPS: [(f32, [f32; 3]); 5] = [
        (0.0, [0.12, 0.4, 0.18]),   // 低地: 深緑
        (0.25, [0.15, 0.5, 0.2]),   // 緑
        (0.5, [0.55, 0.5, 0.25]),   // 黄土色
        (0.75, [0.45, 0.32, 0.22]), // 茶
        (1.0, [0.95, 0.95, 0.95]),  // 山頂付近: 白に近い明色
    ];

    let t = if max > min {
        ((elevation - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };

    for pair in STOPS.windows(2) {
        let (t0, c0) = pair[0];
        let (t1, c1) = pair[1];
        if t <= t1 {
            let local_t = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
            return [
                c0[0] + (c1[0] - c0[0]) * local_t,
                c0[1] + (c1[1] - c0[1]) * local_t,
                c0[2] + (c1[2] - c0[2]) * local_t,
            ];
        }
    }
    STOPS[STOPS.len() - 1].1
}

/// heightmapを双線形補間でサンプリングする。範囲外ならNone。海域(周辺4点のいずれかがNaN)は
/// 標高0mとして扱う(NaNをそのまま返すと呼び出し側の計算がNaN汚染されるため)。
/// `terrain/profile.rs`(断面図)・`components/terrain_view.rs`(カメラ注視点の高さ)から使う。
pub fn sample_heightmap(data: &TerrainData, lat_deg: f64, lon_deg: f64) -> Option<f32> {
    let b = &data.metadata.geodetic_bounds;
    if lat_deg < b.min_lat || lat_deg > b.max_lat || lon_deg < b.min_lon || lon_deg > b.max_lon {
        return None;
    }

    let width = data.metadata.width as usize;
    let height = data.metadata.height as usize;
    // 行順は南→北(DETAILED_DESIGN.md 2.6節の座標復元式と同じ向き。build_meshも同様)。
    let fx = (lon_deg - b.min_lon) / (b.max_lon - b.min_lon) * (width - 1) as f64;
    let fy = (lat_deg - b.min_lat) / (b.max_lat - b.min_lat) * (height - 1) as f64;

    let x0 = fx.floor().clamp(0.0, (width - 1) as f64) as usize;
    let y0 = fy.floor().clamp(0.0, (height - 1) as f64) as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let tx = (fx - x0 as f64) as f32;
    let ty = (fy - y0 as f64) as f32;

    let h00 = data.heightmap[y0 * width + x0];
    let h10 = data.heightmap[y0 * width + x1];
    let h01 = data.heightmap[y1 * width + x0];
    let h11 = data.heightmap[y1 * width + x1];
    let h0 = h00 + (h10 - h00) * tx;
    let h1 = h01 + (h11 - h01) * tx;
    let result = h0 + (h1 - h0) * ty;
    Some(if result.is_nan() { 0.0 } else { result })
}

/// heightmap全体からメッシュを構築する。原点変更時にも呼び直す(DETAILED_DESIGN.md 3.3節)。
pub fn build_mesh(data: &TerrainData, origin: &Origin) -> TerrainMesh {
    let width = data.metadata.width as usize;
    let height = data.metadata.height as usize;
    let bounds = &data.metadata.geodetic_bounds;
    let transform = EnuTransform::new(origin, &data.metadata.ellipsoid);

    let mut vertices = Vec::with_capacity(width * height);
    for j in 0..height {
        let lat = bounds.min_lat
            + (j as f64 / (height - 1) as f64) * (bounds.max_lat - bounds.min_lat);
        for i in 0..width {
            let lon = bounds.min_lon
                + (i as f64 / (width - 1) as f64) * (bounds.max_lon - bounds.min_lon);
            let elevation = data.heightmap[j * width + i];
            // NaN = データなし(海域、geotiff_preprocess参照)。頂点位置はNaNだと破綻するため
            // 標高0mとして配置するが、この頂点を含む三角形は下のindices生成で捨てるので描画されない。
            let is_ocean = elevation.is_nan();
            let position = transform.transform(lat, lon, if is_ocean { 0.0 } else { elevation as f64 });
            let color = if is_ocean {
                [0.0; 3]
            } else {
                elevation_to_color(elevation, data.metadata.elevation_min, data.metadata.elevation_max)
            };
            vertices.push(TerrainVertex { position, color });
        }
    }

    // 海域(NaN)の頂点を1つでも含む三角形は張らない(海は描画せず、背景色のまま見える)。
    // 三角形の有無はheightmapだけで決まり原点に依存しないため、原点変更時に再構築しても
    // インデックス数は変わらない(`TerrainRenderer::update_vertices`は頂点だけを書き換える)。
    let is_land = |k: u32| !data.heightmap[k as usize].is_nan();
    let mut indices = Vec::with_capacity((width - 1) * (height - 1) * 6);
    for j in 0..height - 1 {
        for i in 0..width - 1 {
            let i0 = (j * width + i) as u32;
            let i1 = (j * width + i + 1) as u32;
            let i2 = ((j + 1) * width + i) as u32;
            let i3 = ((j + 1) * width + i + 1) as u32;
            if is_land(i0) && is_land(i1) && is_land(i2) {
                indices.extend_from_slice(&[i0, i1, i2]);
            }
            if is_land(i1) && is_land(i3) && is_land(i2) {
                indices.extend_from_slice(&[i1, i3, i2]);
            }
        }
    }

    TerrainMesh { vertices, indices }
}
