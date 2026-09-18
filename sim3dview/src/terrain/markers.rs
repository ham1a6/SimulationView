//! レーダー観測点(見通し範囲の観測点)の状態管理と、マーカー・覆域ドームの3D描画用
//! ジオメトリ生成。観測点自体はメインパネル上の右クリックで追加する
//! (`ui::terrain_view::TerrainView`)。`RadarMarkersState`が状態(一覧・選択・覆域高度)を
//! 保持し、本モジュールの残りはその状態から描画用の頂点列を作るだけの純粋関数群。
//! マーカー本体はLineList(`TerrainRenderer::update_markers`)、覆域ドームはTriangleList
//! (`TerrainRenderer::update_dome`、半透明のSurfaceとして描く)と、別々のバッファ・
//! パイプラインを使うため、頂点列を作る関数も分かれている。

use leptos::prelude::*;

use super::loader::TerrainData;
use super::los::{compute_coverage_area, compute_los_dome, LosParams, LosPoint};
use super::mesh::{sample_heightmap, EnuTransform, Origin, TerrainVertex};

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

/// マーカー(四角い枠)の地表からの一辺半分の大きさ(メートル)。
const MARKER_HALF_SIZE_M: f64 = 4000.0;
/// マーカーを地表からわずかに持ち上げて描く高さ(メートル、Zファイティング回避)。
const HEIGHT_BIAS_M: f32 = 25.0;

const SELECTED_MARKER_COLOR: [f32; 3] = [1.0, 0.92, 0.25];
const MARKER_COLOR: [f32; 3] = [1.0, 0.55, 0.15];
const DOME_SURFACE_COLOR: [f32; 3] = [0.3, 0.9, 1.0];

/// 覆域ドーム(半球状の面)の緯度リング仰角(度)。0°=地表付近、値が大きいほど真上に近い。
/// 地形に遮蔽されない方角ではどの仰角でも同じ半径(=滑らかな球面)になり、地表付近だけ
/// 地形の遮蔽で半径が内側に凹む(`terrain::los::compute_los_dome`参照)。遮蔽の有無が
/// 仰角によって切り替わる地表付近をやや密に、開けた上空側を粗くしてある。
const DOME_RING_ELEVATIONS_DEG: [f64; 7] = [0.0, 5.0, 10.0, 20.0, 35.0, 55.0, 80.0];
/// 覆域ドームの面を地表からわずかに持ち上げて描く高さ(メートル)。地形に遮蔽される方角
/// では、ドーム境界が定義上ちょうど地形の表面に接する(遮蔽され始める点の高さ=地形の
/// 高さになる)ため、そのままだと地形メッシュとほぼ同じ深度になりZファイティング
/// (カメラ操作中にチカチカする現象の一因)を起こす。マーカー本体と同じ考え方で、
/// ドーム全体を一律に少し持ち上げることで回避する。
const DOME_HEIGHT_BIAS_M: f64 = 20.0;
/// ドームの面(三角形)を作る際に方位角方向で間引く間隔(`compute_los_dome`自体は360方位角
/// で計算するが、そのすべてを三角形化すると特に最上段リングを1点に閉じる傘の部分で
/// 極端に細い三角形が大量に重なり、半透明合成(アルファブレンド)の描画順依存の副作用で
/// カメラ操作中にチカチカして見えることが分かった。方位角を間引いて三角形数・重なりの
/// 度合いを減らすことでこれを緩和する。値が大きいほど三角形が減り軽く/滑らかでなくなる)。
const DOME_MESH_AZIMUTH_STRIDE: usize = 10;

/// 2D地図モードでの覆域表示(指定した海抜高度での探知可能領域)の塗り色。
/// 3Dの覆域ドーム(`DOME_SURFACE_COLOR`)とは見た目で区別できる色にする。
const COVERAGE_AREA_COLOR: [f32; 3] = [0.35, 0.9, 0.4];
/// 覆域表示(2D)の境界線の色。塗り自体はドームと同じ薄い半透明(fs_dome、アルファ0.22)
/// なので地図上ではかなり控えめにしか見えない(実機のピクセルサンプリングで色の混合
/// 自体は正しいことを確認済み)。境界だけは不透明なLineList(`fs_main`)で太めに描き、
/// 地図上でも一目で領域の輪郭が分かるようにする(`los_view.rs`の2D極座標図が
/// 塗り+輪郭線の両方を持つのと同じ考え方)。
const COVERAGE_OUTLINE_COLOR: [f32; 3] = [0.75, 1.0, 0.4];
/// 覆域表示(2D)を地表からわずかに持ち上げて描く高さ(メートル)。マーカー本体・
/// 覆域ドームと同じ理由(Zファイティング回避、`HEIGHT_BIAS_M`参照)。
const COVERAGE_AREA_HEIGHT_BIAS_M: f64 = 20.0;

/// 1つのマーカーの四角い枠(4辺=8頂点、LineList用)を追加する。
fn push_marker_box(
    out: &mut Vec<TerrainVertex>,
    data: &TerrainData,
    mesh_transform: &EnuTransform,
    marker: &RadarMarker,
    color: [f32; 3],
) {
    let ground_elevation =
        sample_heightmap(data, marker.lat_deg, marker.lon_deg).unwrap_or(0.0) as f64;
    let center = mesh_transform.transform(marker.lat_deg, marker.lon_deg, ground_elevation);
    let up = center[2] + HEIGHT_BIAS_M;
    let (cx, cy) = (center[0] as f64, center[1] as f64);
    let s = MARKER_HALF_SIZE_M;
    let corners = [(cx - s, cy - s), (cx + s, cy - s), (cx + s, cy + s), (cx - s, cy + s)];
    for i in 0..4 {
        let (x0, y0) = corners[i];
        let (x1, y1) = corners[(i + 1) % 4];
        out.push(TerrainVertex { position: [x0 as f32, y0 as f32, up], color });
        out.push(TerrainVertex { position: [x1 as f32, y1 as f32, up], color });
    }
}

/// 選択中マーカーの覆域を、半球状の面(TriangleList)としてSurfaceつきの頂点列に追加する
/// (「ワイヤーフレームではなくSurfaceが存在する多面体に」という要望による)。
/// `compute_los_dome`が仰角ごとに求めるスラントレンジ(地形に遮蔽されない方角では最大観測
/// 範囲まで一定、遮蔽される方角だけ内側に凹む)を使い、隣接する2リング×隣接する2方位角
/// ごとに四角形パッチ(三角形2枚)を貼って球面状の面を作る。最上段リングは、その高さの
/// 平均を頂点(アペックス)として傘状に閉じ、開いた穴のない多面体にする。
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
    let Some(num_azimuths) = rings.first().map(|r| r.points.len()) else {
        return;
    };
    if num_azimuths < 2 || rings.len() < 2 {
        return;
    }
    let observer_height =
        sample_heightmap(data, marker.lat_deg, marker.lon_deg).unwrap_or(0.0) as f64 + marker.height_m;

    // ドーム上の1点(リングインデックス, 方位角インデックス)を地形メッシュのENU座標へ変換する。
    let dome_vertex = |ring_i: usize, az_i: usize| -> TerrainVertex {
        let ring = &rings[ring_i];
        let p = &ring.points[az_i % num_azimuths];
        let az_rad = p.azimuth_deg.to_radians();
        let el_rad = ring.elevation_deg.to_radians();
        let horizontal = p.range_m * el_rad.cos();
        let local_east = horizontal * az_rad.sin();
        let local_north = horizontal * az_rad.cos();
        let (lat, lon) = local_transform.inverse(local_east, local_north);
        let absolute_height = observer_height + p.range_m * el_rad.sin() + DOME_HEIGHT_BIAS_M;
        let pos = mesh_transform.transform(lat, lon, absolute_height);
        TerrainVertex { position: pos, color: DOME_SURFACE_COLOR }
    };

    // 三角形化する方位角のインデックス一覧(間引き済み、`DOME_MESH_AZIMUTH_STRIDE`参照)。
    let steps: Vec<usize> = (0..num_azimuths).step_by(DOME_MESH_AZIMUTH_STRIDE).collect();

    // リング間の四角形パッチ(三角形2枚ずつ)。表裏どちらも見えるよう(cull_mode: None)、
    // 巻き順は特に気にしない。
    for ring_i in 0..rings.len() - 1 {
        for k in 0..steps.len() {
            let az_i = steps[k];
            let az_next = steps[(k + 1) % steps.len()];
            let a = dome_vertex(ring_i, az_i);
            let b = dome_vertex(ring_i, az_next);
            let c = dome_vertex(ring_i + 1, az_i);
            let d = dome_vertex(ring_i + 1, az_next);
            out.push(a);
            out.push(b);
            out.push(c);
            out.push(b);
            out.push(d);
            out.push(c);
        }
    }

    // 最上段リングを、その高さの平均を頂点(アペックス)とする傘状の三角形群で閉じる
    // (最上段リングは仰角ごとに半径が異なる=単一の高さに揃わないため、平均で近似する)。
    let top_ring_i = rings.len() - 1;
    let top_ring = &rings[top_ring_i];
    let el_rad = top_ring.elevation_deg.to_radians();
    let avg_height_above_observer: f64 =
        top_ring.points.iter().map(|p| p.range_m * el_rad.sin()).sum::<f64>() / num_azimuths as f64;
    let apex_pos = mesh_transform.transform(
        marker.lat_deg,
        marker.lon_deg,
        observer_height + avg_height_above_observer + DOME_HEIGHT_BIAS_M,
    );
    let apex = TerrainVertex { position: apex_pos, color: DOME_SURFACE_COLOR };
    for k in 0..steps.len() {
        let az_i = steps[k];
        let az_next = steps[(k + 1) % steps.len()];
        out.push(dome_vertex(top_ring_i, az_i));
        out.push(dome_vertex(top_ring_i, az_next));
        out.push(apex);
    }
}

/// 選択中マーカーの、指定した海抜高度での探知可能領域(2D地図モード用)を、地表面に
/// 沿って貼り付けたSurface(TriangleList)として頂点列に追加する。`compute_coverage_area`が
/// 全方位角について求める水平距離を境界とする、観測点を中心とした星形(star-shaped)
/// 領域なので、観測点から境界上の隣接2点への三角形(ファン)を360個並べるだけで
/// 自己交差のない面になる(3Dの覆域ドームのアペック付近のような、視点回転時の
/// 半透明合成チカチカ対策の間引きは、2Dは常に真上固定視点で回転しないため不要)。
fn push_coverage_area(
    out: &mut Vec<TerrainVertex>,
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

    // 地表面に沿わせるため、各点(観測点自身も含む)は「その地点の地表標高+バイアス」の
    // 高さに置く(覆域そのものの高度target_altitude_mではない。あくまで地図上に貼る
    // 塗り分けのオーバーレイであり、3Dドームのように空間中の実際の高度を表現するもの
    // ではないため)。
    let vertex_at = |lat: f64, lon: f64| -> TerrainVertex {
        let ground = sample_heightmap(data, lat, lon).unwrap_or(0.0) as f64;
        let pos = mesh_transform.transform(lat, lon, ground + COVERAGE_AREA_HEIGHT_BIAS_M);
        TerrainVertex { position: pos, color: COVERAGE_AREA_COLOR }
    };
    let boundary_vertex = |p: &LosPoint| -> TerrainVertex {
        let az_rad = p.azimuth_deg.to_radians();
        let local_east = p.range_m * az_rad.sin();
        let local_north = p.range_m * az_rad.cos();
        let (lat, lon) = local_transform.inverse(local_east, local_north);
        vertex_at(lat, lon)
    };

    let center = vertex_at(marker.lat_deg, marker.lon_deg);
    let n = points.len();
    for i in 0..n {
        let a = boundary_vertex(&points[i]);
        let b = boundary_vertex(&points[(i + 1) % n]);
        out.push(center);
        out.push(a);
        out.push(b);
    }
}

/// `push_coverage_area`と同じ境界(指定した海抜高度での探知可能領域の外周)を、不透明な
/// LineList(マーカー本体と同じ`TerrainRenderer::update_markers`のバッファ・パイプライン)
/// として追加する。塗り(TriangleList、半透明)だけでは地図上で見えにくいため、輪郭線を
/// 別途重ねて一目で分かるようにする(`COVERAGE_OUTLINE_COLOR`の定義コメント参照)。
fn push_coverage_outline(
    out: &mut Vec<TerrainVertex>,
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

    let boundary_vertex = |p: &LosPoint| -> TerrainVertex {
        let az_rad = p.azimuth_deg.to_radians();
        let local_east = p.range_m * az_rad.sin();
        let local_north = p.range_m * az_rad.cos();
        let (lat, lon) = local_transform.inverse(local_east, local_north);
        let ground = sample_heightmap(data, lat, lon).unwrap_or(0.0) as f64;
        let pos = mesh_transform.transform(lat, lon, ground + COVERAGE_AREA_HEIGHT_BIAS_M);
        TerrainVertex { position: pos, color: COVERAGE_OUTLINE_COLOR }
    };

    let n = points.len();
    for i in 0..n {
        out.push(boundary_vertex(&points[i]));
        out.push(boundary_vertex(&points[(i + 1) % n]));
    }
}

/// マーカー一覧 + 選択状態から、マーカー本体(四角い枠、LineList)の頂点列を作る。
/// `mesh_origin`は現在GPUにアップロードされている地形メッシュの原点(マーカー自体の
/// 緯度経度とは無関係。マーカー位置をこの原点基準のENU座標へ変換するために使う)。
pub fn build_marker_geometry(
    data: &TerrainData,
    mesh_origin: &Origin,
    markers: &[RadarMarker],
    selected: Option<u64>,
) -> Vec<TerrainVertex> {
    let mesh_transform = EnuTransform::new(mesh_origin, &data.metadata.ellipsoid);
    let mut out = Vec::new();
    for marker in markers {
        let is_selected = selected == Some(marker.id);
        let color = if is_selected { SELECTED_MARKER_COLOR } else { MARKER_COLOR };
        push_marker_box(&mut out, data, &mesh_transform, marker, color);
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

/// `build_coverage_area_geometry`と同じ境界の、不透明な輪郭線(LineList)の頂点列を作る。
/// `build_marker_geometry`の結果と連結して`TerrainRenderer::update_markers`に渡す想定
/// (マーカー本体と同じパイプラインを使うため)。
pub fn build_coverage_outline_geometry(
    data: &TerrainData,
    mesh_origin: &Origin,
    markers: &[RadarMarker],
    selected: Option<u64>,
    target_altitude_m: f64,
) -> Vec<TerrainVertex> {
    let mesh_transform = EnuTransform::new(mesh_origin, &data.metadata.ellipsoid);
    let mut out = Vec::new();
    if let Some(marker) = selected.and_then(|id| markers.iter().find(|m| m.id == id)) {
        push_coverage_outline(&mut out, data, &mesh_transform, marker, target_altitude_m);
    }
    out
}

/// 選択中マーカーの、指定した海抜高度での探知可能領域(2D地図モード、TriangleList)の
/// 頂点列を作る。3Dの`build_dome_surface_geometry`と同様、選択中のマーカーについてのみ描く。
pub fn build_coverage_area_geometry(
    data: &TerrainData,
    mesh_origin: &Origin,
    markers: &[RadarMarker],
    selected: Option<u64>,
    target_altitude_m: f64,
) -> Vec<TerrainVertex> {
    let mesh_transform = EnuTransform::new(mesh_origin, &data.metadata.ellipsoid);
    let mut out = Vec::new();
    if let Some(marker) = selected.and_then(|id| markers.iter().find(|m| m.id == id)) {
        push_coverage_area(&mut out, data, &mesh_transform, marker, target_altitude_m);
    }
    out
}
