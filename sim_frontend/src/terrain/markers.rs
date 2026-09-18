//! レーダー観測点(見通し範囲の観測点)のマーカー・覆域リングの3D描画用ジオメトリ生成。
//! 観測点自体はメインパネル上の右クリックで追加する(`components/terrain_view.rs`)。
//! `ui_state::RadarMarkersState`が状態(一覧・選択)を保持し、このモジュールはその状態から
//! 描画用の頂点列(LineList、`TerrainRenderer::update_markers`用)を作るだけの純粋関数群。

use super::loader::TerrainData;
use super::los::{compute_los, LosParams};
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

/// マーカー(四角い枠)の地表からの一辺半分の大きさ(メートル)。
const MARKER_HALF_SIZE_M: f64 = 4000.0;
/// マーカーを地表からわずかに持ち上げて描く高さ(メートル、Zファイティング回避)。
const HEIGHT_BIAS_M: f32 = 25.0;

const SELECTED_MARKER_COLOR: [f32; 3] = [1.0, 0.92, 0.25];
const MARKER_COLOR: [f32; 3] = [1.0, 0.55, 0.15];
const COVERAGE_RING_COLOR: [f32; 3] = [0.3, 0.9, 1.0];

/// 覆域ドーム(半球状ワイヤーフレーム)の緯度リング仰角(度)。0°=地表(見通し限界距離
/// そのものが半径)、値が大きいほど真上に近い。「もっと立体的に、半球状のオブジェクトが
/// 表示されるイメージで」という要望により、単なる地表面の等高線的リングから、
/// 複数の仰角リング+経度線で構成する半球ワイヤーフレームに変更した。
const DOME_RING_ELEVATIONS_DEG: [f64; 5] = [0.0, 20.0, 40.0, 60.0, 80.0];
/// ドームの経度線(縦線)の本数(等間隔の方位角に1本ずつ)。
const DOME_MERIDIAN_COUNT: usize = 12;

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

/// 選択中マーカーの覆域を、半球状のワイヤーフレーム(ドーム)としてLineList用の頂点列に
/// 追加する。各方位角の見通し限界距離(`compute_los`、地表面基準)を**その方位角での
/// ドーム半径**とみなし、仰角0°(地表)から`DOME_RING_ELEVATIONS_DEG`の各仰角までの
/// 緯度リング(円周)+経度線(縦線)で球面状に描く(sin/cosで単純に extrude するだけなので、
/// 地形の凹凡に関わらず滑らかな半球の見た目になる。地表の起伏に沿わせていた以前の実装より、
/// 「半球状のオブジェクト」という見た目の要望に合わせた)。
fn push_coverage_dome(
    out: &mut Vec<TerrainVertex>,
    data: &TerrainData,
    mesh_transform: &EnuTransform,
    marker: &RadarMarker,
) {
    let radar_origin = Origin { lat_deg: marker.lat_deg, lon_deg: marker.lon_deg };
    let local_transform = EnuTransform::new(&radar_origin, &data.metadata.ellipsoid);
    let params = LosParams { observer_height_m: marker.height_m, max_range_m: marker.max_range_m };
    let points = compute_los(data, &radar_origin, &params);
    if points.len() < 2 {
        return;
    }
    let observer_height =
        sample_heightmap(data, marker.lat_deg, marker.lon_deg).unwrap_or(0.0) as f64 + marker.height_m;

    // ドーム上の1点(方位角インデックス, 仰角)を地形メッシュのENU座標へ変換する。
    let dome_vertex = |az_i: usize, elevation_deg: f64| -> TerrainVertex {
        let p = &points[az_i % points.len()];
        let az_rad = p.azimuth_deg.to_radians();
        let el_rad = elevation_deg.to_radians();
        let horizontal = p.range_m * el_rad.cos();
        let local_east = horizontal * az_rad.sin();
        let local_north = horizontal * az_rad.cos();
        let (lat, lon) = local_transform.inverse(local_east, local_north);
        let absolute_height = observer_height + p.range_m * el_rad.sin();
        let pos = mesh_transform.transform(lat, lon, absolute_height);
        TerrainVertex { position: pos, color: COVERAGE_RING_COLOR }
    };

    // 緯度リング(各仰角ごとに全方位角を結ぶ閉ループ)。
    for &elevation_deg in &DOME_RING_ELEVATIONS_DEG {
        for i in 0..points.len() {
            out.push(dome_vertex(i, elevation_deg));
            out.push(dome_vertex(i + 1, elevation_deg));
        }
    }

    // 経度線(等間隔の方位角ごとに、地表〜最上段リングを結ぶ縦線)。
    for m in 0..DOME_MERIDIAN_COUNT {
        let az_i = m * points.len() / DOME_MERIDIAN_COUNT;
        for pair in DOME_RING_ELEVATIONS_DEG.windows(2) {
            out.push(dome_vertex(az_i, pair[0]));
            out.push(dome_vertex(az_i, pair[1]));
        }
    }
}

/// マーカー一覧 + 選択状態から、3D描画用の頂点列を作る。覆域リングは選択中のマーカー
/// についてのみ描く(複数マーカーの覆域を同時に重ねると見づらいため)。
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
        if is_selected {
            push_coverage_dome(&mut out, data, &mesh_transform, marker);
        }
    }
    out
}
