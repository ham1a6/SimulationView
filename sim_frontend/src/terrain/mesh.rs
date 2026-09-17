//! 地形メッシュ生成。DESIGN.md 3.2節(ENU変換)・5.1節(頂点構造・配色)。

use super::loader::{Ellipsoid, TerrainData};

/// 頂点構造(DESIGN.md 5.1節)。UV座標は使わず、標高由来の色を直接持たせる。
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

/// 基準位置(原点)。DESIGN.md 3.1節。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Origin {
    pub lat_deg: f64,
    pub lon_deg: f64,
}

/// 緯度経度(+標高)からENU座標(東=X, 北=Y, 上=Z)へ変換する。DESIGN.md 3.2節の変換式そのもの。
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
}

fn geodetic_to_ecef(lat: f64, lon: f64, h: f64, a: f64, e2: f64) -> (f64, f64, f64) {
    let n = a / (1.0 - e2 * lat.sin().powi(2)).sqrt();
    let x = (n + h) * lat.cos() * lon.cos();
    let y = (n + h) * lat.cos() * lon.sin();
    let z = (n * (1.0 - e2) + h) * lat.sin();
    (x, y, z)
}

/// 標高を正規化し、低地(緑〜青みの低彩度)→高山(白に近い明色)の地形図的カラーランプへ写像する
/// (DESIGN.md 5.1節)。カラーストップは実装時に調整可能な固定テーブル。
fn elevation_to_color(elevation: f32, min: f32, max: f32) -> [f32; 3] {
    const STOPS: [(f32, [f32; 3]); 5] = [
        (0.0, [0.05, 0.35, 0.35]),  // 低地: 深緑がかった青緑
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

/// heightmap全体からメッシュを構築する。原点変更時にも呼び直す(DESIGN.md 3.3節)。
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
            let position = transform.transform(lat, lon, elevation as f64);
            let color =
                elevation_to_color(elevation, data.metadata.elevation_min, data.metadata.elevation_max);
            vertices.push(TerrainVertex { position, color });
        }
    }

    let mut indices = Vec::with_capacity((width - 1) * (height - 1) * 6);
    for j in 0..height - 1 {
        for i in 0..width - 1 {
            let i0 = (j * width + i) as u32;
            let i1 = (j * width + i + 1) as u32;
            let i2 = ((j + 1) * width + i) as u32;
            let i3 = ((j + 1) * width + i + 1) as u32;
            indices.push(i0);
            indices.push(i1);
            indices.push(i2);
            indices.push(i1);
            indices.push(i3);
            indices.push(i2);
        }
    }

    TerrainMesh { vertices, indices }
}
