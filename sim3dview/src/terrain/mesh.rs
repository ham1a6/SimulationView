//! 地形メッシュ生成。DETAILED_DESIGN.md 3.2節(ENU変換)・6.5節(頂点構造)・6.7節(配色)。

use super::loader::{Ellipsoid, TerrainData, TileEntry, NO_DATA};

/// 頂点構造(DETAILED_DESIGN.md 6.5節)。UV座標は使わず、標高由来の色を直接持たせる。
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TerrainVertex {
    pub position: [f32; 3], // x(East), y(North), z(Up) — ENU変換結果
    pub color: [f32; 3],
    /// 陰影(ヒルシェード)用の単位法線のx(East)・y(North)成分(snorm16。-32767〜32767が-1〜1)。
    /// z(Up)成分は`sqrt(1-x^2-y^2)`でシェーダーが復元する(地表の法線は常に上向きなので符号は
    /// 決まっている。xyだけにして頂点を4バイト増やすだけで済ませ、snorm8より精度が高い)。
    /// 陰影を付けない頂点(マーカー・覆域・海に隣接して法線が求まらない点)は`UNLIT_NORMAL`。
    pub normal_xy: [i16; 2],
}

impl TerrainVertex {
    /// 「陰影を付けない」を表す法線。xy成分の長さが1を超える(=単位法線ではありえない)値にしてあり、
    /// シェーダーがこれを見て陰影を掛けない。x=y=0(真上向き=平地)とは区別される。
    pub const UNLIT_NORMAL: [i16; 2] = [i16::MIN, i16::MIN];

    /// 陰影を付けない頂点(マーカー・覆域ドームなど、色をそのまま出したいもの)。
    pub fn unlit(position: [f32; 3], color: [f32; 3]) -> Self {
        Self { position, color, normal_xy: Self::UNLIT_NORMAL }
    }
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
    /// 原点の緯度経度の三角関数(`transform_f64`・`enu_to_geodetic`が毎回求め直すと、
    /// 覆域ドームの頂点変換のように大量に呼ぶ場面で重いので、`new`で1回だけ求めておく)。
    sin_lat0: f64,
    cos_lat0: f64,
    sin_lon0: f64,
    cos_lon0: f64,
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
        let sin_lat0 = lat0.sin();
        let denom = (1.0 - e2 * sin_lat0 * sin_lat0).sqrt();
        Self {
            origin_lat_rad: lat0,
            origin_lon_rad: lon0,
            origin_x: x0,
            origin_y: y0,
            origin_z: z0,
            sin_lat0,
            cos_lat0: lat0.cos(),
            sin_lon0: lon0.sin(),
            cos_lon0: lon0.cos(),
            a,
            e2,
            meridian_radius: a * (1.0 - e2) / denom.powi(3),
            parallel_radius: a / denom * lat0.cos(),
        }
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
        let (sin_lat, cos_lat, sin_lon, cos_lon) =
            (self.sin_lat0, self.cos_lat0, self.sin_lon0, self.cos_lon0);
        let b = self.a * (1.0 - self.e2).sqrt();
        let (da, db) = (1.0 / self.a, 1.0 / b);
        let rows = [
            [-sin_lon * da, -sin_lat * cos_lon * da, cos_lat * cos_lon * da],
            [cos_lon * da, -sin_lat * sin_lon * da, cos_lat * sin_lon * da],
            [0.0, cos_lat * db, sin_lat * db],
        ];
        let m = rows.map(|r| [r[0] as f32, r[1] as f32, r[2] as f32, 0.0]);
        let g = [
            (self.origin_x * da) as f32,
            (self.origin_y * da) as f32,
            (self.origin_z * db) as f32,
        ];
        let c0 = g.iter().map(|&v| v as f64 * v as f64).sum::<f64>() - 1.0;
        (m, [g[0], g[1], g[2], c0 as f32])
    }

    /// `transform`のf64版(遠方の地表の上座標を丸めずに扱いたい呼び出し側用)。
    pub fn transform_f64(&self, lat_deg: f64, lon_deg: f64, h: f64) -> [f64; 3] {
        let lat = lat_deg.to_radians();
        let lon = lon_deg.to_radians();
        let (x, y, z) = geodetic_to_ecef(lat, lon, h, self.a, self.e2);
        let dx = x - self.origin_x;
        let dy = y - self.origin_y;
        let dz = z - self.origin_z;

        let (sin_lat0, cos_lat0, sin_lon0, cos_lon0) =
            (self.sin_lat0, self.cos_lat0, self.sin_lon0, self.cos_lon0);

        let east = -sin_lon0 * dx + cos_lon0 * dy;
        let north = -sin_lat0 * cos_lon0 * dx - sin_lat0 * sin_lon0 * dy + cos_lat0 * dz;
        let up = cos_lat0 * cos_lon0 * dx + cos_lat0 * sin_lon0 * dy + sin_lat0 * dz;
        [east, north, up]
    }

    /// `transform`の厳密な逆: ENU座標(東, 北, 上。メートル)から(緯度, 経度, 楕円体高)を求める。
    /// ENU→ECEF(原点の回転行列の転置)→測地座標(反復法)。原点から数千km離れた点でも
    /// 地球の丸み・楕円体を正しく扱う(下の`inverse`は原点近傍の接平面近似)。
    pub fn enu_to_geodetic(&self, east: f64, north: f64, up: f64) -> (f64, f64, f64) {
        let (sin_lat0, cos_lat0, sin_lon0, cos_lon0) =
            (self.sin_lat0, self.cos_lat0, self.sin_lon0, self.cos_lon0);

        let x = self.origin_x - sin_lon0 * east - sin_lat0 * cos_lon0 * north + cos_lat0 * cos_lon0 * up;
        let y = self.origin_y + cos_lon0 * east - sin_lat0 * sin_lon0 * north + cos_lat0 * sin_lon0 * up;
        let z = self.origin_z + cos_lat0 * north + sin_lat0 * up;

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

/// 色の正規化に使う標高の下限(メートル)。元データ(DSM)には水面などのノイズによる大きな
/// 負の値が一部のタイルにあり、`metadata.elevation_min`をそのまま下限にすると低地全体の
/// 色がずれるため、0m以下はまとめて低地の色にする。
const COLOR_MIN_ELEVATION_M: f32 = 0.0;

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

/// 標高を双線形補間でサンプリングする。範囲外ならNone。タイルが無い・海域(周辺4ノードの
/// いずれかがデータなし)は標高0mとして扱う(欠損をそのまま返すと呼び出し側の計算が破綻するため)。
/// 各タイル(のチャンク)は、いま画面に出しているレベルのグリッド(`TerrainData::set_chunk_level`)
/// で引くので、描画されている地形と観測点・見通し計算・クリック判定の標高が一致する。
/// `terrain/los.rs`(見通し)・`terrain/markers.rs`(観測点)・`terrain/profile.rs`(断面図)・
/// `terrain/pick.rs`(クリック判定)・`ui/terrain_view.rs`(カメラ注視点の高さ)から使う。
pub fn sample_heightmap(data: &TerrainData, lat_deg: f64, lon_deg: f64) -> Option<f32> {
    let b = &data.metadata.geodetic_bounds;
    if lat_deg < b.min_lat || lat_deg > b.max_lat || lon_deg < b.min_lon || lon_deg > b.max_lon {
        return None;
    }

    // 東端・北端ちょうどは、その内側のタイルの端として扱う。
    let lat0 = (lat_deg.floor() as i32).min(b.max_lat as i32 - 1);
    let lon0 = (lon_deg.floor() as i32).min(b.max_lon as i32 - 1);
    let Some(tile) = data.tile((lat0, lon0)) else {
        return Some(0.0);
    };
    let u = (lon_deg - lon0 as f64).clamp(0.0, 1.0);
    let v = (lat_deg - lat0 as f64).clamp(0.0, 1.0);
    Some(data.sample_bilinear(tile, u, v))
}

/// ENUの水平位置(東, 北)の真上/真下にある地表点の(緯度, 経度, ENU上座標)。
/// 地形メッシュの頂点は楕円体上の地表をENUへ変換したもの(遠方ほど丸みで下がる)なので、
/// 視線との交差判定・注視点の高さ合わせには、標高そのものではなくこの上座標を使うこと。
/// 「(東, 北)を通る鉛直線」と地表の交点は、上座標を仮定→測地座標へ戻す→その地点の地表の
/// 上座標で更新、を数回繰り返して求める(地表の傾きが小さいので速やかに収束する)。
/// 地形データ範囲外・海域(NaN)は標高0mとして扱う。
pub fn ground_at_enu(
    data: &TerrainData,
    transform: &EnuTransform,
    east: f64,
    north: f64,
) -> (f64, f64, f32) {
    // 丸みによる低下量の第一近似を初期値にする(0から始めるより収束が速い)。
    let mut up = -(east * east + north * north) / (2.0 * transform.a);
    let mut lat_lon = (0.0, 0.0);
    for _ in 0..4 {
        let (lat, lon, _h) = transform.enu_to_geodetic(east, north, up);
        let elevation = sample_heightmap(data, lat, lon).unwrap_or(0.0);
        up = transform.transform_f64(lat, lon, elevation as f64)[2];
        lat_lon = (lat, lon);
    }
    (lat_lon.0, lat_lon.1, up as f32)
}

/// 地表上の(緯度, 経度)のENU座標(東, 北, 上)。`ground_at_enu`の逆向き。
/// 地形データ範囲外・海域(NaN)は標高0mとして扱う。
pub fn ground_at_geodetic(
    data: &TerrainData,
    transform: &EnuTransform,
    lat_deg: f64,
    lon_deg: f64,
) -> (f32, f32, f32) {
    let elevation = sample_heightmap(data, lat_deg, lon_deg).unwrap_or(0.0);
    let [east, north, up] = transform.transform(lat_deg, lon_deg, elevation as f64);
    (east, north, up)
}

/// メッシュの縁に沿って下へ垂らす「スカート」の深さ(メートル)。解像度の違う隣のメッシュ同士は、
/// 縁のノードの高さがわずかに食い違い、縁に隙間ができて背景の黒が見える。縁から下向きの壁
/// (スカート)を付けて隙間を隠す。粗いレベルほど食い違いが大きいので深くしてある。
fn skirt_depth_m(level: usize) -> f32 {
    const DEPTHS: [f32; 5] = [800.0, 400.0, 250.0, 150.0, 100.0];
    DEPTHS[level.min(DEPTHS.len() - 1)]
}

/// グリッド1枚(一辺`cells`セル)のメッシュの頂点数(ノード(N+1)^2 + 縁4辺のスカート4(N+1))。
/// タイル全体(レベル0)でもチャンクでも同じ式。
pub fn tile_vertex_count(cells: usize) -> usize {
    let n = cells + 1;
    n * n + 4 * n
}

/// スカートの辺e(0=南,1=東,2=北,3=西)のk番目のノードの、グリッド内の番号。
fn edge_node(edge: usize, k: usize, cells: usize) -> usize {
    let n = cells + 1;
    match edge {
        0 => k,
        1 => k * n + cells,
        2 => cells * n + k,
        _ => k * n,
    }
}

/// メッシュにするグリッド1枚の位置決め: ノード(列i, 行j)は緯度`lat_start + j*step_deg`、
/// 経度`lon_start + i*step_deg`にある。
struct GridPlacement {
    lat_start: f64,
    lon_start: f64,
    step_deg: f64,
    skirt_depth: f32,
}

/// グリッドの各ノードの法線のxy成分(`TerrainVertex::normal_xy`)。ノードの東西・南北の隣の
/// ノードの位置の差(中心差分。縁のノード・海に隣接するノードは、陸の側だけを使う片側差分)の
/// 外積から求める。位置はENU座標(地球の丸み込み)なので、法線もENUの向きで得られる。
/// 海のノード自身、隣が両側とも海(差分が取れない)のノードは陰影なし(`UNLIT_NORMAL`)。
/// チャンクの縁のノードは隣のチャンクの標高を見ないので片側差分になり、隣のチャンクの縁と
/// わずかに食い違うが、目立たない程度(下記の実機確認参照)。
fn node_normals(grid: &[i16], n: usize, positions: &[[f64; 3]]) -> Vec<[i16; 2]> {
    let land = |i: usize, j: usize| grid[j * n + i] != NO_DATA;
    // 隣(-1/+1)のうち陸のものを選ぶ。なければ自分自身。
    let neighbor = |k: usize, lo_land: bool, hi_land: bool| -> (usize, usize) {
        (if lo_land { k - 1 } else { k }, if hi_land { k + 1 } else { k })
    };

    let mut normals = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            if !land(i, j) {
                normals.push(TerrainVertex::UNLIT_NORMAL);
                continue;
            }
            let (i_lo, i_hi) = neighbor(i, i > 0 && land(i - 1, j), i + 1 < n && land(i + 1, j));
            let (j_lo, j_hi) = neighbor(j, j > 0 && land(i, j - 1), j + 1 < n && land(i, j + 1));
            if i_lo == i_hi || j_lo == j_hi {
                normals.push(TerrainVertex::UNLIT_NORMAL);
                continue;
            }
            let (e0, e1) = (positions[j * n + i_lo], positions[j * n + i_hi]);
            let (n0, n1) = (positions[j_lo * n + i], positions[j_hi * n + i]);
            let east = [e1[0] - e0[0], e1[1] - e0[1], e1[2] - e0[2]];
            let north = [n1[0] - n0[0], n1[1] - n0[1], n1[2] - n0[2]];
            // 東向き x 北向き = 上向き。
            let cross = [
                east[1] * north[2] - east[2] * north[1],
                east[2] * north[0] - east[0] * north[2],
                east[0] * north[1] - east[1] * north[0],
            ];
            let len = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
            if len < 1e-9 || cross[2] <= 0.0 {
                normals.push(TerrainVertex::UNLIT_NORMAL);
                continue;
            }
            let pack = |v: f64| ((v / len).clamp(-1.0, 1.0) * 32767.0).round() as i16;
            normals.push([pack(cross[0]), pack(cross[1])]);
        }
    }
    normals
}

/// グリッドの頂点列を、指定した原点のENU座標で作る。並びは、ノード(行=南→北、列=西→東)の後に、
/// スカート4辺(南・東・北・西)の順。頂点数と並びは原点に依存しない(原点変更時は
/// `TerrainRenderer::update_mesh_vertices`で位置だけを書き換える)。行(緯度)・列(経度)ごとの
/// 三角関数を前計算して、1頂点あたりの計算を軽くしてある(原点変更時に全メッシュの頂点を
/// 作り直すため)。
fn grid_vertices(
    grid: &[i16],
    cells: usize,
    place: &GridPlacement,
    max_elevation: f32,
    transform: &EnuTransform,
) -> Vec<TerrainVertex> {
    let n = cells + 1;
    let (a, e2) = (transform.a, transform.e2);

    // 行(緯度)ごと: (sinφ, cosφ, 卯酉線曲率半径N)。列(経度)ごと: (sinλ, cosλ)。
    let rows: Vec<(f64, f64, f64)> = (0..n)
        .map(|j| {
            let (s, c) = (place.lat_start + j as f64 * place.step_deg).to_radians().sin_cos();
            (s, c, a / (1.0 - e2 * s * s).sqrt())
        })
        .collect();
    let cols: Vec<(f64, f64)> = (0..n)
        .map(|i| (place.lon_start + i as f64 * place.step_deg).to_radians().sin_cos())
        .collect();

    let (slat0, clat0) = transform.origin_lat_rad.sin_cos();
    let (slon0, clon0) = transform.origin_lon_rad.sin_cos();

    let mut vertices = Vec::with_capacity(tile_vertex_count(cells));
    // 法線を求めるためのノードのENU位置(f64。原点から遠いタイルではf32だと隣のノードとの差が
    // 誤差に埋もれて陰影がざらつくので、丸める前の値を使う)。
    let mut positions: Vec<[f64; 3]> = Vec::with_capacity(n * n);
    for j in 0..n {
        let (s, c, prime) = rows[j];
        for i in 0..n {
            let (sl, cl) = cols[i];
            let value = grid[j * n + i];
            // データなし(海域)の頂点位置はNaNだと破綻するため標高0mで配置するが、この頂点を
            // 含む三角形は`grid_indices`で捨てるので描画されない。
            let (h, color) = if value == NO_DATA {
                (0.0, [0.0; 3])
            } else {
                (
                    value as f64,
                    elevation_to_color(value as f32, COLOR_MIN_ELEVATION_M, max_elevation),
                )
            };
            let x = (prime + h) * c * cl - transform.origin_x;
            let y = (prime + h) * c * sl - transform.origin_y;
            let z = (prime * (1.0 - e2) + h) * s - transform.origin_z;
            let east = -slon0 * x + clon0 * y;
            let north = -slat0 * clon0 * x - slat0 * slon0 * y + clat0 * z;
            let up = clat0 * clon0 * x + clat0 * slon0 * y + slat0 * z;
            positions.push([east, north, up]);
            vertices.push(TerrainVertex::unlit([east as f32, north as f32, up as f32], color));
        }
    }
    for (vertex, normal_xy) in vertices.iter_mut().zip(node_normals(grid, n, &positions)) {
        vertex.normal_xy = normal_xy;
    }

    for edge in 0..4 {
        for k in 0..n {
            let mut v = vertices[edge_node(edge, k, cells)];
            v.position[2] -= place.skirt_depth;
            vertices.push(v);
        }
    }
    vertices
}

/// グリッドの三角形インデックスを作る。データなし(海域)のノードを1つでも含む三角形は張らない
/// (海は描画せず背景色のまま見える)。スカートは、隣り合う2ノードがどちらも陸のときだけ壁を張る。
/// 三角形の有無はグリッドだけで決まり原点に依存しない。
fn grid_indices(grid: &[i16], cells: usize) -> Vec<u32> {
    let n = cells + 1;
    let land = |k: usize| grid[k] != NO_DATA;

    let mut indices = Vec::with_capacity(cells * cells * 6);
    for j in 0..cells {
        for i in 0..cells {
            let i0 = j * n + i;
            let i1 = i0 + 1;
            let i2 = i0 + n;
            let i3 = i2 + 1;
            if land(i0) && land(i1) && land(i2) {
                indices.extend_from_slice(&[i0 as u32, i1 as u32, i2 as u32]);
            }
            if land(i1) && land(i3) && land(i2) {
                indices.extend_from_slice(&[i1 as u32, i3 as u32, i2 as u32]);
            }
        }
    }

    let skirt_base = n * n;
    for edge in 0..4 {
        for k in 0..cells {
            let (a, b) = (edge_node(edge, k, cells), edge_node(edge, k + 1, cells));
            if land(a) && land(b) {
                let (sa, sb) =
                    ((skirt_base + edge * n + k) as u32, (skirt_base + edge * n + k + 1) as u32);
                indices.extend_from_slice(&[a as u32, b as u32, sa, b as u32, sb, sa]);
            }
        }
    }
    indices
}

/// タイル全体(レベル0)の頂点列。
pub fn build_whole_tile_vertices(
    data: &TerrainData,
    tile: &TileEntry,
    transform: &EnuTransform,
) -> Vec<TerrainVertex> {
    let cells = data.level_cells(0);
    let place = GridPlacement {
        lat_start: tile.key.0 as f64,
        lon_start: tile.key.1 as f64,
        step_deg: 1.0 / cells as f64,
        skirt_depth: skirt_depth_m(0),
    };
    grid_vertices(data.whole_grid(tile), cells, &place, data.metadata.elevation_max, transform)
}

/// タイル全体(レベル0)のメッシュ。
pub fn build_whole_tile_mesh(
    data: &TerrainData,
    tile: &TileEntry,
    transform: &EnuTransform,
) -> TerrainMesh {
    TerrainMesh {
        vertices: build_whole_tile_vertices(data, tile, transform),
        indices: grid_indices(data.whole_grid(tile), data.level_cells(0)),
    }
}

/// チャンク(行(南→北)*分割数+列(西→東))・レベル(1以上)の頂点列。グリッドが未取得ならNone。
pub fn build_chunk_vertices(
    data: &TerrainData,
    tile: &TileEntry,
    chunk: usize,
    level: usize,
    transform: &EnuTransform,
) -> Option<Vec<TerrainVertex>> {
    let grid = data.chunk_grid(tile, level, chunk)?;
    let k = data.chunks_per_tile();
    let (cx, cy) = (chunk % k, chunk / k);
    let cells = data.chunk_cells(level);
    let step_deg = 1.0 / data.level_cells(level) as f64;
    let place = GridPlacement {
        lat_start: tile.key.0 as f64 + (cy * cells) as f64 * step_deg,
        lon_start: tile.key.1 as f64 + (cx * cells) as f64 * step_deg,
        step_deg,
        skirt_depth: skirt_depth_m(level),
    };
    Some(grid_vertices(&grid, cells, &place, data.metadata.elevation_max, transform))
}

/// チャンク・レベル(1以上)のメッシュ。グリッドが未取得ならNone。
pub fn build_chunk_mesh(
    data: &TerrainData,
    tile: &TileEntry,
    chunk: usize,
    level: usize,
    transform: &EnuTransform,
) -> Option<TerrainMesh> {
    let vertices = build_chunk_vertices(data, tile, chunk, level, transform)?;
    let grid = data.chunk_grid(tile, level, chunk)?;
    Some(TerrainMesh { vertices, indices: grid_indices(&grid, data.chunk_cells(level)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transform_at(lat: f64, lon: f64) -> EnuTransform {
        EnuTransform::new(
            &Origin { lat_deg: lat, lon_deg: lon },
            &Ellipsoid { a_m: 6378137.0, inv_f: 298.257222101 },
        )
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
}
