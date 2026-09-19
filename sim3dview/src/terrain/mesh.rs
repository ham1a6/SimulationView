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

/// 海域(heightmapがNaN=データなし)の塗り色。DETAILED_DESIGN.md 2.3節: 元データが複数の
/// GeoTIFFタイルに分かれており、タイルが存在しない領域・各タイル内のNODATA画素は
/// geotiff_preprocessがNaNとして出力する(海の目印)。地形の標高グラデーションとは
/// 明確に区別できる水色にする。
const WATER_COLOR: [f32; 3] = [0.55, 0.78, 0.92];

/// 標高を正規化し、低地(緑〜青みの低彩度)→高山(白に近い明色)の地形図的カラーランプへ写像する
/// (DETAILED_DESIGN.md 6.7節)。カラーストップは実装時に調整可能な固定テーブル。
///
/// 低地(標高0付近)の色は元々[0.05, 0.35, 0.35](深緑がかった青緑)だったが、WATER_COLOR
/// ([0.55, 0.78, 0.92])と同じ青緑系統の色相だったため、低地の陸地(NaN=海のセルと
/// 隣接して細かく入り組む沿岸部)が遠景でMSAA/スーパーサンプリングによって周囲の海色と
/// 混ざり、陸地の存在自体が視認できないほど海色に埋もれてしまう問題があった(「原点から
/// 遠いところでは陸地があるはずなのに海になっている」と報告された不具合の実体。位置・
/// 標高データ・NaN判定はすべて正しく、色のコントラスト不足が原因だったことをheightmap.bin
/// の直接検証で確認済み)。水色(WATER_COLOR)から明確に離れた緑系の色相に変更し、
/// どのズーム倍率・距離からでも低地の陸地が海と混同されないようにした。
fn elevation_to_color(elevation: f32, min: f32, max: f32) -> [f32; 3] {
    const STOPS: [(f32, [f32; 3]); 5] = [
        (0.0, [0.12, 0.4, 0.18]),   // 低地: 深緑(WATER_COLORの青緑系統から意図的に離した)
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
            // 標高0mとして配置し、色だけ地形グラデーションと区別できる水色にする。
            let is_ocean = elevation.is_nan();
            let position = transform.transform(lat, lon, if is_ocean { 0.0 } else { elevation as f64 });
            let color = if is_ocean {
                WATER_COLOR
            } else {
                elevation_to_color(elevation, data.metadata.elevation_min, data.metadata.elevation_max)
            };
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

    let mut mesh = TerrainMesh { vertices, indices };
    append_background_skirt(&mut mesh, data.metadata.elevation_min, bounds, &transform);
    mesh
}

/// メインパネルの背景(実データの外接矩形の外側)を「地平線から下は水色」に見せるための、
/// 実データよりずっと広い水平な板(スカート)。標高は実データの最低標高より確実に低い位置に
/// 置き、実際の地形(海面=標高0m付近を含む)が常に手前に描画されるようにする(同じ高さだと
/// Zファイティングで点滅しうるため)。
///
/// 外周の半径は`terrain::camera`のZ_FAR・MAX_DISTANCEと整合させる必要がある: カメラは
/// 原点から最大MAX_DISTANCE離れられるため、スカートの外周(隅ではコーナーウェッジ用に
/// さらに2倍まで伸ばす、下記参照)がZ_FARを超えると、カメラの遠方クリップ面によって
/// スカートの外周を切り取ってしまい、その先に何も描画されない黒い背景が見えてしまう
/// (実機で確認した不具合。以前はZ_FARが無限遠だったため問題にならなかったが、有限の
/// Z_FARに変更した際にこの制約を見落としていた。またこの半径を小さくしすぎても、
/// 俯瞰プリセットの広いFOV+浅いピッチ角では画面端で地平線方向の視線がこの半径より
/// 遠くまで届いてしまい、同様に黒い背景が見える不具合も実機で確認した。以前の
/// 「一律の全面板」方式で使っていた半径と同じ4,000,000mまで戻すことで解消した)。
/// 実地形が実際に必要とする範囲(MAX_DISTANCE+データ外接矩形の対角線長)はこれより
/// 大幅に小さいままなので、Z_FARを大きくしても実地形側の深度精度には影響しない。
const BACKGROUND_OUTER_RADIUS_M: f32 = 4_000_000.0;
const BACKGROUND_MARGIN_BELOW_MIN_M: f32 = 500.0;

/// スカートを1枚の全面板として実データの直下にも敷いていた際、実データ最低標高との差
/// (`BACKGROUND_MARGIN_BELOW_MIN_M`)が数百m程度しかない低地の沿岸部で、遠景では
/// 実地形側が負けてスカートに隠れる(=陸地が海に沈んで見える)報告を受けた対応。
/// 「一律の平面ではなく、データの外接矩形の外側にだけ敷く」よう、実データの4隅をENU座標に
/// 変換した四角形を「穴」として持つ、額縁状の構成に変更した。これにより実データの直下に
/// スカートが存在すること自体がなくなり、上記の沈み込みが原理的に起こり得なくなる。
///
/// 「隅から辺ごとに個別の図形をつなぎ合わせる」方式(軸並行外接矩形での穴あけ、隅からの
/// 放射状引き伸ばし、辺の法線オフセット+コーナーウェッジ、の3通りを試した)は、いずれも
/// 特定の隅で図形同士の継ぎ目に隙間ができ、そこから何も描画されない黒い背景が見えてしまう
/// 不具合が実機で繰り返し見つかった(実機のデバッグ配色で断片ごとに色分けして確認)。
/// このような「複数図形を個別に計算してつなぎ合わせる」方式は継ぎ目の考慮漏れに弱いため、
/// 代わりに中心から全方位角を一定刻みで走査し、各方位角について「実データの境界との交点」
/// (内周)と「BACKGROUND_OUTER_RADIUS_M上の点」(外周)を求め、隣り合う方位角同士を
/// 1枚の四角形でつなぐリング状のテッセレーションに変更した。
///
/// 内周は当初、実データの4隅だけを直線で結んだ四角形との交差で求めていたが、これでも
/// 同じ隙間が再現し続けた。原因は緯度経度の矩形の辺(例えば緯度一定・経度が変化する辺)を
/// ENU座標へ変換すると、地球が球面(正確には回転楕円体面)であるために直線にはならず
/// わずかに湾曲すること。実際に検証したところ、原点から約465km離れた辺(5°×5°の矩形の
/// 遠い側の辺)では、両端の隅を結んだ直線と実際の辺の中点との間に約2.7kmもの差があった
/// (`EnuTransform`で数値検証済み)。これは無視できない大きさで、4隅だけの直線近似では
/// スカートの内周と実際の地形メッシュの外周(2048点の格子)の間に隙間ができてしまう。
/// 4辺それぞれを`BOUNDARY_SAMPLES_PER_EDGE`点で細かくサンプリングした多角形を使うことで
/// この湾曲に追従させ、隙間を解消した。
const SKIRT_RING_SEGMENTS: usize = 64;
const BOUNDARY_SAMPLES_PER_EDGE: usize = 32;

fn append_background_skirt(
    mesh: &mut TerrainMesh,
    elevation_min: f32,
    bounds: &super::loader::GeodeticBounds,
    transform: &EnuTransform,
) {
    let up = elevation_min - BACKGROUND_MARGIN_BELOW_MIN_M;

    let to_xy = |lat: f64, lon: f64| -> [f32; 2] {
        let p = transform.transform(lat, lon, 0.0);
        [p[0], p[1]]
    };

    // 実データの外周を、4隅だけでなく各辺をBOUNDARY_SAMPLES_PER_EDGE点でサンプリングした
    // 多角形として求める(地球の湾曲により、緯度経度の辺はENU座標上では直線にならないため)。
    // 反時計回り(or 時計回り、原点次第)に、南辺→東辺→北辺→西辺の順で1周する。
    let mut boundary: Vec<[f32; 2]> = Vec::with_capacity(BOUNDARY_SAMPLES_PER_EDGE * 4);
    let n = BOUNDARY_SAMPLES_PER_EDGE;
    for k in 0..n {
        let t = k as f64 / n as f64;
        let lon = bounds.min_lon + t * (bounds.max_lon - bounds.min_lon);
        boundary.push(to_xy(bounds.min_lat, lon)); // 南辺: 西→東
    }
    for k in 0..n {
        let t = k as f64 / n as f64;
        let lat = bounds.min_lat + t * (bounds.max_lat - bounds.min_lat);
        boundary.push(to_xy(lat, bounds.max_lon)); // 東辺: 南→北
    }
    for k in 0..n {
        let t = k as f64 / n as f64;
        let lon = bounds.max_lon - t * (bounds.max_lon - bounds.min_lon);
        boundary.push(to_xy(bounds.max_lat, lon)); // 北辺: 東→西
    }
    for k in 0..n {
        let t = k as f64 / n as f64;
        let lat = bounds.max_lat - t * (bounds.max_lat - bounds.min_lat);
        boundary.push(to_xy(lat, bounds.min_lon)); // 西辺: 北→南
    }
    let boundary_len = boundary.len();

    let center_x = boundary.iter().map(|c| c[0]).sum::<f32>() / boundary_len as f32;
    let center_y = boundary.iter().map(|c| c[1]).sum::<f32>() / boundary_len as f32;

    // 方位角ごとに、中心からその方向へのレイが実データの境界多角形と交わる点(内周)を求める。
    // 全辺を評価して条件を満たす候補を集め、その中から最小のt(中心に最も近い交点)を
    // 採用する(凸多角形の内部の点から出るレイは幾何学的には辺をちょうど1つだけ通過する
    // はずだが、浮動小数点誤差で複数の辺が条件を満たしてしまう場合に備え、最も近い交点を
    // 選ぶことで頑健にしてある)。
    let inner_point = |dx: f32, dy: f32| -> [f32; 2] {
        let mut best_t: Option<f32> = None;
        for k in 0..boundary_len {
            let a = boundary[k];
            let b = boundary[(k + 1) % boundary_len];
            let ex = b[0] - a[0];
            let ey = b[1] - a[1];
            let denom = ex * dy - ey * dx;
            if denom.abs() < 1e-3 {
                continue; // レイと辺がほぼ平行
            }
            let ax = a[0] - center_x;
            let ay = a[1] - center_y;
            let t = (ex * ay - ey * ax) / denom;
            let s = (dx * ay - dy * ax) / denom;
            if t > 0.0 && s.is_finite() && (-1e-3..=1.0 + 1e-3).contains(&s) {
                if best_t.is_none_or(|bt| t < bt) {
                    best_t = Some(t);
                }
            }
        }
        match best_t {
            Some(t) => [center_x + dx * t, center_y + dy * t],
            // 理論上はここに来ないはずだが、万一交点が見つからなければ中心そのものを返す
            // (退化した三角形になるだけで、隙間や破綻にはならない)。
            None => [center_x, center_y],
        }
    };

    let mut ring_inner = Vec::with_capacity(SKIRT_RING_SEGMENTS);
    let mut ring_outer = Vec::with_capacity(SKIRT_RING_SEGMENTS);
    for i in 0..SKIRT_RING_SEGMENTS {
        let theta = (i as f32) / (SKIRT_RING_SEGMENTS as f32) * std::f32::consts::TAU;
        let (dy, dx) = theta.sin_cos();
        ring_inner.push(inner_point(dx, dy));
        ring_outer.push([center_x + dx * BACKGROUND_OUTER_RADIUS_M, center_y + dy * BACKGROUND_OUTER_RADIUS_M]);
    }

    for i in 0..SKIRT_RING_SEGMENTS {
        let j = (i + 1) % SKIRT_RING_SEGMENTS;
        let quad = [ring_inner[i], ring_inner[j], ring_outer[j], ring_outer[i]];
        add_skirt_quad(mesh, up, &quad);
    }
}

fn add_skirt_quad(mesh: &mut TerrainMesh, up: f32, quad: &[[f32; 2]; 4]) {
    let base = mesh.vertices.len() as u32;
    for p in quad {
        mesh.vertices.push(TerrainVertex { position: [p[0], p[1], up], color: WATER_COLOR });
    }
    mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}
