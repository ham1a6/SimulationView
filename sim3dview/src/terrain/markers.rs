//! レーダー観測点(見通し範囲の観測点)の状態管理と、マーカー・覆域の描画用ジオメトリ生成。
//! 観測点自体はメインパネル上の右クリックで追加する(`ui::terrain_view::TerrainView`)。
//! `RadarMarkersState`が状態(一覧・選択・覆域高度)を保持し、本モジュールの残りはその状態から
//! 描画用の頂点列を作るだけの純粋関数群。
//! - 観測点のマーカー: 画面サイズ固定のピン(ビルボード、`DrawVertex::billboard`)。`TerrainRenderer::update_markers`。
//! - 3Dの覆域ドーム: TriangleListの半透明の面(`TerrainVertex`)。`TerrainRenderer::update_dome`。
//! - 2Dの覆域: 塗り(半透明の三角形)+輪郭線(太い線)を`DrawVertex`で。深度テストなしで描く
//!   (`TerrainRenderer::update_coverage_2d`)。
//!
//! 別々のバッファ・パイプラインを使うため、頂点列を作る関数も分かれている。

use leptos::prelude::*;

use super::drawing_geometry::append_line_strip;
use super::vertex::DrawVertex;
use super::loader::TerrainData;
use super::render_bias::{COVERAGE_AREA_M, DOME_M, MARKER_M};
use super::los::{compute_coverage_area, compute_los_dome, LosParams};
use super::mesh::TerrainVertex;
use super::geodesy::EnuTransform;
use super::heightmap::sample_heightmap;
use super::origin::Origin;
/// 地図上に配置したレーダー観測点1つ分の情報。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadarMarker {
    pub id: u64,
    pub lat_deg: f64,
    pub lon_deg: f64,
    /// アンテナ高(地表からのメートル)。
    pub height_m: f64,
    /// 最大観測範囲(メートル)。
    pub max_range_m: f64,
}

/// メインパネル(3D地形)上への右クリックで追加するレーダー観測点(見通し範囲)の一覧・選択状態。
/// `ui::terrain_view::TerrainView`(追加・3D/2D描画)・`ui::los_view::LosView`
/// (一覧・編集・削除)で共有する。アプリ側は`leptos::prelude::provide_context`で
/// 1つだけ生成して渡す(`RadarMarkersState::new()`)。
#[derive(Clone, Copy)]
pub struct RadarMarkersState {
    pub markers: RwSignal<Vec<RadarMarker>>,
    pub selected: RwSignal<Option<u64>>,
    next_id: RwSignal<u64>,
    /// メインパネルが2D地図モードのときに覆域表示で使う、対象の海抜高度(メートル)。
    /// 個々のレーダーのパラメータ(アンテナ高・最大観測範囲)とは別に、表示側の設定
    /// として1つだけ持つ(複数レーダーがあっても「今見たい高度」は1つのため)。
    pub coverage_altitude_m: RwSignal<f64>,
}

impl RadarMarkersState {
    pub fn new() -> Self {
        Self {
            markers: RwSignal::new(Vec::new()),
            selected: RwSignal::new(None),
            next_id: RwSignal::new(1),
            coverage_altitude_m: RwSignal::new(1000.0),
        }
    }

    /// 指定した緯度経度に既定パラメータ(アンテナ高10m・最大観測範囲50km)のレーダーを
    /// 追加し、選択状態にする。
    pub fn add(&self, lat_deg: f64, lon_deg: f64) -> u64 {
        let id = self.next_id.get_untracked();
        self.next_id.set(id + 1);
        self.markers.update(|list| {
            list.push(RadarMarker { id, lat_deg, lon_deg, height_m: 10.0, max_range_m: 50_000.0 });
        });
        self.selected.set(Some(id));
        id
    }

    pub fn remove(&self, id: u64) {
        self.markers.update(|list| list.retain(|m| m.id != id));
        if self.selected.get_untracked() == Some(id) {
            self.selected.set(None);
        }
    }
}

impl Default for RadarMarkersState {
    fn default() -> Self {
        Self::new()
    }
}

const SELECTED_MARKER_COLOR: [f32; 4] = [1.0, 0.92, 0.25, 1.0];
const MARKER_COLOR: [f32; 4] = [1.0, 0.55, 0.15, 1.0];
/// ピンの縁取りと中の点の色。
const MARKER_OUTLINE_COLOR: [f32; 4] = [0.08, 0.08, 0.1, 1.0];
const DOME_SURFACE_COLOR: [f32; 3] = [0.3, 0.9, 1.0];

/// ピンの頭の円の中心の高さ(先端から、画面のpx)と半径・縁取りの太さ・中の点の半径。
const PIN_HEAD_CENTER_PX: f32 = 26.0;
const PIN_HEAD_RADIUS_PX: f32 = 10.0;
const PIN_OUTLINE_PX: f32 = 2.5;
const PIN_DOT_RADIUS_PX: f32 = 4.0;
const PIN_HEAD_SEGMENTS: usize = 24;

/// 覆域ドーム(半球状の面)の緯度リング仰角(度、0以上90未満の昇順)。0°=地表付近、値が大きいほど
/// 真上に近い。地形に遮蔽されない方角ではどの仰角でも同じ半径(=滑らかな球面)になり、
/// 地表付近だけ地形の遮蔽で半径が内側に凹む(`terrain::los::compute_los_dome`参照)。凹みの
/// 形が出る低い仰角(遮蔽物の仰角は多くの場合10〜20度以下)を細かく、開けた上空側を粗くしてあるが、
/// 最上部でも4度刻み以下にして、輪郭が多角形に見えないようにしてある。
/// リング数が多いほどドームの頂点数(リング数 x 方位数 x 6/リング間)が増える。
const DOME_RING_ELEVATIONS_DEG: [f64; 38] = [
    0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, // 1度刻み
    12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 30.0, // 2度刻み
    33.0, 36.0, 39.0, 42.0, 45.0, 48.0, 51.0, 54.0, 57.0, 60.0, // 3度刻み
    64.0, 68.0, 72.0, 76.0, 80.0, 84.0, 87.0, // 4度刻み(最上段は87度)
];
/// ドームの方位角の間引き(計算は1mil刻み・6400方位のまま、なめらかにした後で描くときだけ間引く)。
/// 2なら3200方位(50km先で約98m間隔)で、平滑化後の形は間引いても変わらない。
const DOME_AZIMUTH_STRIDE: usize = 2;
/// 最上段リングを1点(アペックス)に閉じる傘の三角形の数。`compute_los_dome`の全方位角を傘に使うと、
/// 極端に細い三角形が大量に1点へ重なり、半透明合成(アルファブレンド)の描画順依存の副作用で
/// カメラ操作中にチカチカして見えることが分かった(360方位角のときに確認)ので、方位角を間引いて
/// この数の三角形にする。リング間の四角形パッチは互いに重ならないので間引かない。
const DOME_APEX_SEGMENTS: usize = 48;
/// ドームの半径(方位角方向)の平滑化: 1〜数方位だけの外れ値を除くメディアン(前後この方位ずつ)→平均(同)。
/// 生の計算結果は1mil(0.056度)ごとの遮蔽判定なので、地形の細かい凹凸や、1方位だけ遮蔽される所が
/// 観測点へ向かう細い三角形(放射状の筋)になって、ドームが滑らかに見えない。地形に遮蔽される境目も
/// 数十m〜数百mの幅でなだらかになるだけで、遮蔽されない方角の半径(最大観測範囲)は変わらない。
const DOME_SMOOTH_MEDIAN_HALF: usize = 3;
const DOME_SMOOTH_MEAN_HALF: usize = 6;

/// 2D地図モードでの覆域表示(指定した海抜高度での探知可能領域)の塗り色(RGB)と不透明度。
/// 3Dの覆域ドーム(`DOME_SURFACE_COLOR`)とは見た目で区別できる色にする。
const COVERAGE_AREA_COLOR: [f32; 3] = [0.35, 0.9, 0.4];
const COVERAGE_AREA_ALPHA: f32 = 0.32;
/// 覆域表示(2D)の境界線の色と太さ(画面のpx)。塗りは半透明で控えめなので、境界だけは不透明な太い線で描き、
/// 領域の輪郭が一目で分かるようにする(`los_view.rs`の2D極座標図が塗り+輪郭線の両方を持つのと同じ考え方)。
const COVERAGE_OUTLINE_COLOR: [f32; 4] = [0.75, 1.0, 0.4, 1.0];
const COVERAGE_OUTLINE_WIDTH_PX: f32 = 2.5;
/// 2Dの覆域の境界の平滑化(前後この方位ずつ。ドームより弱く、境界の形が残る程度)。
const COVERAGE_SMOOTH_MEDIAN_HALF: usize = 2;
const COVERAGE_SMOOTH_MEAN_HALF: usize = 3;

/// 円環上の値(方位角ごとの半径など)を平滑化する: 前後`median_half`個ずつのメディアン(外れ値の除去、
/// 段差の位置は保つ)→前後`mean_half`個ずつの平均(段差をなだらかにする)。端は反対側へつながる。
fn smooth_circular(values: &[f64], median_half: usize, mean_half: usize) -> Vec<f64> {
    let n = values.len();
    if n == 0 {
        return Vec::new();
    }
    let at = |v: &[f64], i: isize| v[i.rem_euclid(n as isize) as usize];
    let (mh, ah) = (median_half as isize, mean_half as isize);
    let medians: Vec<f64> = (0..n as isize)
        .map(|i| {
            let mut window: Vec<f64> = (-mh..=mh).map(|d| at(values, i + d)).collect();
            window.sort_by(f64::total_cmp);
            window[window.len() / 2]
        })
        .collect();
    (0..n as isize)
        .map(|i| (-ah..=ah).map(|d| at(&medians, i + d)).sum::<f64>() / (2 * ah + 1) as f64)
        .collect()
}

/// 三角形を1つ追加する。
fn push_tri(out: &mut Vec<DrawVertex>, anchor: [f32; 3], color: [f32; 4], points: [[f32; 2]; 3]) {
    for p in points {
        out.push(DrawVertex::billboard(anchor, p, color));
    }
}

/// ピンの形(頭の円+先端の三角形)を、`anchor`の画面上に大きさ`head_radius`px・先端の高さ`tip_y`pxで積む。
/// 座標は先端の基準位置(0,0)から画面のpx(右・上が正)。先端の三角形の辺は頭の円の接線。
fn push_pin_shape(out: &mut Vec<DrawVertex>, anchor: [f32; 3], color: [f32; 4], head_radius: f32, tip_y: f32) {
    let center_y = PIN_HEAD_CENTER_PX;
    let circle = |i: usize| {
        let t = std::f32::consts::TAU * i as f32 / PIN_HEAD_SEGMENTS as f32;
        [head_radius * t.cos(), center_y + head_radius * t.sin()]
    };
    for i in 0..PIN_HEAD_SEGMENTS {
        push_tri(out, anchor, color, [[0.0, center_y], circle(i), circle(i + 1)]);
    }
    // 先端(0,tip_y)から頭の円へ引いた接線の接点。
    let beta = (head_radius / (center_y - tip_y)).acos();
    let tangent_x = head_radius * beta.sin();
    let tangent_y = center_y - head_radius * beta.cos();
    push_tri(out, anchor, color, [[0.0, tip_y], [-tangent_x, tangent_y], [tangent_x, tangent_y]]);
}

/// 1つの観測点のマーカー(ピン。先端が観測点)を追加する。縁取り→本体→中の点の順に重ねる。
/// 画面のサイズが固定で、拡大・縮小・回転しても同じ大きさで常に正面を向く(`draw.wgsl`のビルボード)。
fn push_marker_pin(
    out: &mut Vec<DrawVertex>,
    data: &TerrainData,
    mesh_transform: &EnuTransform,
    marker: &RadarMarker,
    color: [f32; 4],
) {
    let ground_elevation =
        sample_heightmap(data, marker.lat_deg, marker.lon_deg).unwrap_or(0.0) as f64;
    let anchor = mesh_transform.transform(
        marker.lat_deg,
        marker.lon_deg,
        ground_elevation + MARKER_M,
    );
    push_pin_shape(
        out,
        anchor,
        MARKER_OUTLINE_COLOR,
        PIN_HEAD_RADIUS_PX + PIN_OUTLINE_PX,
        -PIN_OUTLINE_PX * 1.4,
    );
    push_pin_shape(out, anchor, color, PIN_HEAD_RADIUS_PX, 0.0);
    for i in 0..PIN_HEAD_SEGMENTS {
        let point = |i: usize| {
            let t = std::f32::consts::TAU * i as f32 / PIN_HEAD_SEGMENTS as f32;
            [PIN_DOT_RADIUS_PX * t.cos(), PIN_HEAD_CENTER_PX + PIN_DOT_RADIUS_PX * t.sin()]
        };
        push_tri(out, anchor, MARKER_OUTLINE_COLOR, [[0.0, PIN_HEAD_CENTER_PX], point(i), point(i + 1)]);
    }
}

/// 選択中マーカーの覆域を、半球状の面(TriangleList)としてSurfaceつきの頂点列に追加する
/// (「ワイヤーフレームではなくSurfaceが存在する多面体に」という要望による)。
/// `compute_los_dome`が仰角ごとに求めるスラントレンジ(地形に遮蔽されない方角では最大観測
/// 範囲まで一定、遮蔽される方角だけ内側に凹む)を、方位角方向に平滑化(`DOME_SMOOTH_*`)してから、
/// 隣接する2リング×隣接する2方位角ごとに四角形パッチ(三角形2枚)を貼って球面状の面を作る。
/// 最上段リングは、その半径の平均を高さとする頂点(アペックス)へ傘状に閉じ、開いた穴のない多面体にする。
fn push_dome_surface(
    out: &mut Vec<TerrainVertex>,
    data: &TerrainData,
    mesh_transform: &EnuTransform,
    marker: &RadarMarker,
) {
    let radar_origin = Origin { lat_deg: marker.lat_deg, lon_deg: marker.lon_deg };
    let local_transform = EnuTransform::new(&radar_origin, &data.metadata.ellipsoid);
    let params = LosParams { observer_height_m: marker.height_m, max_range_m: marker.max_range_m };
    let rings = compute_los_dome(data, &radar_origin, &params, &DOME_RING_ELEVATIONS_DEG);
    let Some(num_raw_azimuths) = rings.first().map(|r| r.points.len()) else {
        return;
    };
    let azimuth_indices: Vec<usize> = (0..num_raw_azimuths).step_by(DOME_AZIMUTH_STRIDE).collect();
    let num_azimuths = azimuth_indices.len();
    if num_azimuths < 2 || rings.len() < 2 {
        return;
    }
    let observer_height =
        sample_heightmap(data, marker.lat_deg, marker.lon_deg).unwrap_or(0.0) as f64 + marker.height_m;

    // リングごとに、半径を方位角方向に平滑化する。
    let smoothed: Vec<Vec<f64>> = rings
        .iter()
        .map(|ring| {
            let ranges: Vec<f64> = ring.points.iter().map(|p| p.range_m).collect();
            smooth_circular(&ranges, DOME_SMOOTH_MEDIAN_HALF, DOME_SMOOTH_MEAN_HALF)
        })
        .collect();

    // ドーム上の全頂点(リングごと・方位角ごと)を地形メッシュのENU座標へ変換しておく。
    // 四角形パッチが各頂点を最大4回使うので、先に1回ずつだけ計算する。
    let dome_vertices: Vec<Vec<TerrainVertex>> = rings
        .iter()
        .zip(&smoothed)
        .map(|(ring, ranges)| {
            let el_rad = ring.elevation_deg.to_radians();
            azimuth_indices
                .iter()
                .map(|&az_i| {
                    let az_rad = ring.points[az_i].azimuth_deg.to_radians();
                    let range_m = ranges[az_i];
                    let horizontal = range_m * el_rad.cos();
                    let local_east = horizontal * az_rad.sin();
                    let local_north = horizontal * az_rad.cos();
                    let (lat, lon) = local_transform.inverse(local_east, local_north);
                    let absolute_height = observer_height + range_m * el_rad.sin() + DOME_M;
                    let pos = mesh_transform.transform(lat, lon, absolute_height);
                    TerrainVertex::unlit(pos, DOME_SURFACE_COLOR)
                })
                .collect()
        })
        .collect();

    // リング間の四角形パッチ(三角形2枚ずつ)。表裏どちらも見えるよう(cull_mode: None)、
    // 巻き順は特に気にしない。
    for ring_i in 0..rings.len() - 1 {
        let (lower, upper) = (&dome_vertices[ring_i], &dome_vertices[ring_i + 1]);
        for az_i in 0..num_azimuths {
            let az_next = (az_i + 1) % num_azimuths;
            let (a, b, c, d) = (lower[az_i], lower[az_next], upper[az_i], upper[az_next]);
            out.push(a);
            out.push(b);
            out.push(c);
            out.push(b);
            out.push(d);
            out.push(c);
        }
    }

    // 最上段リングを、その半径の平均を高さとする頂点(アペックス)へ傘状の三角形群で閉じる
    // (最上段リングは仰角87度で、ほぼ真上。遮蔽がなければ半径=最大観測範囲=球の頂点の高さ)。
    let top_ring_i = rings.len() - 1;
    let top_ranges = &smoothed[top_ring_i];
    let avg_range = top_ranges.iter().sum::<f64>() / top_ranges.len() as f64;
    let apex_pos = mesh_transform.transform(
        marker.lat_deg,
        marker.lon_deg,
        observer_height + avg_range + DOME_M,
    );
    let apex = TerrainVertex::unlit(apex_pos, DOME_SURFACE_COLOR);
    let steps: Vec<usize> = (0..num_azimuths).step_by((num_azimuths / DOME_APEX_SEGMENTS).max(1)).collect();
    for k in 0..steps.len() {
        let az_i = steps[k];
        let az_next = steps[(k + 1) % steps.len()];
        out.push(dome_vertices[top_ring_i][az_i]);
        out.push(dome_vertices[top_ring_i][az_next]);
        out.push(apex);
    }
}

/// 選択中マーカーの、指定した海抜高度での探知可能領域(2D地図モード用)の塗り(地表面に
/// 沿って貼り付けた半透明のSurface)と、その外周の輪郭線(不透明な太い線)の頂点列を`out`に追加する。
/// どちらも`DrawVertex`で、深度テストなしで描く(`TerrainRenderer::update_coverage_2d`)。2Dは真上からの
/// 正射影で地形に隠れることがないので、深度テストをすると、観測点から境界への大きな三角形が
/// 地形の起伏に埋まって、塗りが場所によって欠けて不均一になる。
///
/// `compute_coverage_area`が全方位角(1mil刻み、6400方向)について求める水平距離を、方位角方向に
/// 平滑化(`COVERAGE_SMOOTH_*`。1方位だけの外れ値による細い切れ込み・突起を除く)したものを境界とする、
/// 観測点を中心とした星形(star-shaped)領域なので、観測点から境界上の隣接2点への三角形
/// (ファン)を並べるだけで自己交差のない面になる(3Dの覆域ドームのアペックス付近のような、
/// 視点回転時の半透明合成チカチカ対策の間引きは、2Dは常に真上固定視点で回転しないため不要)。
/// 境界の計算(6400本のレイ)が重いので、塗りと輪郭線で1回の結果を共有する。
fn push_coverage_2d(
    out: &mut Vec<DrawVertex>,
    data: &TerrainData,
    mesh_transform: &EnuTransform,
    marker: &RadarMarker,
    target_altitude_m: f64,
) {
    let radar_origin = Origin { lat_deg: marker.lat_deg, lon_deg: marker.lon_deg };
    let local_transform = EnuTransform::new(&radar_origin, &data.metadata.ellipsoid);
    let params = LosParams { observer_height_m: marker.height_m, max_range_m: marker.max_range_m };
    let points = compute_coverage_area(data, &radar_origin, &params, target_altitude_m);
    if points.len() < 2 {
        return;
    }
    let ranges: Vec<f64> = points.iter().map(|p| p.range_m).collect();
    let ranges = smooth_circular(&ranges, COVERAGE_SMOOTH_MEDIAN_HALF, COVERAGE_SMOOTH_MEAN_HALF);

    // 地表面に沿わせるため、各点(観測点自身も含む)は「その地点の地表標高+バイアス」の
    // 高さに置く(覆域そのものの高度target_altitude_mではない。あくまで地図上に貼る
    // 塗り分けのオーバーレイであり、3Dドームのように空間中の実際の高度を表現するもの
    // ではないため)。
    let position_at = |lat: f64, lon: f64| -> [f32; 3] {
        let ground = sample_heightmap(data, lat, lon).unwrap_or(0.0) as f64;
        mesh_transform.transform(lat, lon, ground + COVERAGE_AREA_M)
    };
    let boundary: Vec<[f32; 3]> = points
        .iter()
        .zip(&ranges)
        .map(|(p, &range_m)| {
            let az_rad = p.azimuth_deg.to_radians();
            let local_east = range_m * az_rad.sin();
            let local_north = range_m * az_rad.cos();
            let (lat, lon) = local_transform.inverse(local_east, local_north);
            position_at(lat, lon)
        })
        .collect();

    let fill = [COVERAGE_AREA_COLOR[0], COVERAGE_AREA_COLOR[1], COVERAGE_AREA_COLOR[2], COVERAGE_AREA_ALPHA];
    let center = position_at(marker.lat_deg, marker.lon_deg);
    let n = boundary.len();
    for i in 0..n {
        let (a, b) = (boundary[i], boundary[(i + 1) % n]);
        for position in [center, a, b] {
            out.push(DrawVertex::surface(position, fill, None));
        }
    }
    append_line_strip(out, &boundary, true, COVERAGE_OUTLINE_COLOR, COVERAGE_OUTLINE_WIDTH_PX);
}

/// マーカー一覧 + 選択状態から、マーカー(ピン)の頂点列を作る。
/// `mesh_origin`は現在GPUにアップロードされている地形メッシュの原点(マーカー自体の
/// 緯度経度とは無関係。マーカー位置をこの原点基準のENU座標へ変換するために使う)。
pub fn build_marker_geometry(
    data: &TerrainData,
    mesh_origin: &Origin,
    markers: &[RadarMarker],
    selected: Option<u64>,
) -> Vec<DrawVertex> {
    let mesh_transform = EnuTransform::new(mesh_origin, &data.metadata.ellipsoid);
    let mut out = Vec::new();
    for marker in markers {
        let is_selected = selected == Some(marker.id);
        let color = if is_selected { SELECTED_MARKER_COLOR } else { MARKER_COLOR };
        push_marker_pin(&mut out, data, &mesh_transform, marker, color);
    }
    out
}

/// 選択中マーカーの覆域ドーム(半球状の面、TriangleList)の頂点列を作る。複数マーカーの
/// 覆域を同時に重ねると見づらいため、選択中のマーカーについてのみ描く。
pub fn build_dome_surface_geometry(
    data: &TerrainData,
    mesh_origin: &Origin,
    markers: &[RadarMarker],
    selected: Option<u64>,
) -> Vec<TerrainVertex> {
    let mesh_transform = EnuTransform::new(mesh_origin, &data.metadata.ellipsoid);
    let mut out = Vec::new();
    if let Some(marker) = selected.and_then(|id| markers.iter().find(|m| m.id == id)) {
        push_dome_surface(&mut out, data, &mesh_transform, marker);
    }
    out
}

/// 選択中マーカーの、指定した海抜高度での探知可能領域(2D地図モード)の頂点列(塗り+輪郭線)を作る。
/// 3Dの`build_dome_surface_geometry`と同様、選択中のマーカーについてのみ描く。
pub fn build_coverage_2d_geometry(
    data: &TerrainData,
    mesh_origin: &Origin,
    markers: &[RadarMarker],
    selected: Option<u64>,
    target_altitude_m: f64,
) -> Vec<DrawVertex> {
    let mesh_transform = EnuTransform::new(mesh_origin, &data.metadata.ellipsoid);
    let mut out = Vec::new();
    if let Some(marker) = selected.and_then(|id| markers.iter().find(|m| m.id == id)) {
        push_coverage_2d(&mut out, data, &mesh_transform, marker, target_altitude_m);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothing_keeps_constant_and_removes_single_spikes() {
        // 一定の値は変わらない。
        let flat = vec![50_000.0; 100];
        assert!(smooth_circular(&flat, 3, 6).iter().all(|&v| (v - 50_000.0).abs() < 1e-9));
        // 1方位だけの落ち込み(観測点へ向かう細い三角形になる)は消える。
        let mut spiky = flat.clone();
        spiky[40] = 0.0;
        spiky[70] = 0.0;
        spiky[71] = 0.0;
        assert!(smooth_circular(&spiky, 3, 6).iter().all(|&v| (v - 50_000.0).abs() < 1e-9));
    }

    #[test]
    fn smoothing_wraps_around_and_softens_steps() {
        // 端(0と99)をまたぐ段差: 前半だけ遮蔽(0)、後半は一定。円環なので端でも同じように滑らかになる。
        let mut v = vec![10_000.0; 100];
        for i in (0..20).chain(90..100) {
            v[i] = 2_000.0;
        }
        let s = smooth_circular(&v, 3, 6);
        // 段差の中心付近で中間の値、両側は元の値のまま(段差の幅の外は影響を受けない)。
        assert!(s[20] > 2_000.0 && s[20] < 10_000.0);
        assert!(s[5] < 2_000.0 + 1e-9 && s[50] > 10_000.0 - 1e-9);
        // 単調(段差の間で値が上下しない)。
        assert!(s[10..40].windows(2).all(|w| w[1] >= w[0] - 1e-9));
        assert!(s[70..100].windows(2).all(|w| w[1] <= w[0] + 1e-9));
    }

    #[test]
    fn pin_is_built_from_billboard_triangles() {
        let mut out = Vec::new();
        push_pin_shape(&mut out, [1.0, 2.0, 3.0], [1.0, 0.0, 0.0, 1.0], 10.0, 0.0);
        // 頭の円(24分割)+先端の三角形。全頂点がアンカーを共有し、画面のオフセットだけが違う。
        assert_eq!(out.len(), (PIN_HEAD_SEGMENTS + 1) * 3);
        assert!(out.iter().all(|v| v.position == [1.0, 2.0, 3.0] && v.params[2] == 1.0 && v.params[0] == 0.0));
        // 先端(0,0)が一番下、頭のてっぺんは中心+半径。
        let ys: Vec<f32> = out.iter().map(|v| v.aux[1]).collect();
        assert!(ys.iter().cloned().fold(f32::MAX, f32::min) >= -1e-4);
        let top = ys.iter().cloned().fold(f32::MIN, f32::max);
        assert!((top - (PIN_HEAD_CENTER_PX + 10.0)).abs() < 0.1);
    }
}
