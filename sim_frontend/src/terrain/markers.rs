//! レーダー観測点(見通し範囲の観測点)のマーカー・覆域リングの3D描画用ジオメトリ生成。
//! 観測点自体はメインパネル上の右クリックで追加する(`components/terrain_view.rs`)。
//! `ui_state::RadarMarkersState`が状態(一覧・選択)を保持し、このモジュールはその状態から
//! 描画用の頂点列(LineList、`TerrainRenderer::update_markers`用)を作るだけの純粋関数群。

use super::loader::TerrainData;
use super::los::{compute_los_dome, LosParams};
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

/// 覆域ドーム(半球状ワイヤーフレーム)の緯度リング仰角(度)。0°=地表付近、値が大きいほど
/// 真上に近い。地形に遮蔽されない方角ではどの仰角でも同じ半径(=滑らかな球面)になり、
/// 地表付近だけ地形の遮蔽で半径が内側に凹む(`terrain::los::compute_los_dome`参照)。
/// 遮蔽の有無が仰角によって切り替わる地表付近をやや密に、開けた上空側を粗くしてある。
const DOME_RING_ELEVATIONS_DEG: [f64; 7] = [0.0, 5.0, 10.0, 20.0, 35.0, 55.0, 80.0];
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
/// 追加する。`compute_los_dome`が仰角ごとに求めるスラントレンジ(地形に遮蔽されない方角では
/// 最大観測範囲まで一定、遮蔽される方角だけ内側に凹む)を使い、緯度リング(円周)+
/// 経度線(縦線)で球面状に描く。上空は地形にほとんど遮蔽されないため、結果として
/// 「地表面で遮られているところ以外は滑らかな球面」という見た目になる。
fn push_coverage_dome(
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
    if num_azimuths < 2 {
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
        let absolute_height = observer_height + p.range_m * el_rad.sin();
        let pos = mesh_transform.transform(lat, lon, absolute_height);
        TerrainVertex { position: pos, color: COVERAGE_RING_COLOR }
    };

    // 緯度リング(各仰角ごとに全方位角を結ぶ閉ループ)。
    for ring_i in 0..rings.len() {
        for az_i in 0..num_azimuths {
            out.push(dome_vertex(ring_i, az_i));
            out.push(dome_vertex(ring_i, az_i + 1));
        }
    }

    // 経度線(等間隔の方位角ごとに、地表〜最上段リングを結ぶ縦線)。
    for m in 0..DOME_MERIDIAN_COUNT {
        let az_i = m * num_azimuths / DOME_MERIDIAN_COUNT;
        for ring_i in 0..rings.len().saturating_sub(1) {
            out.push(dome_vertex(ring_i, az_i));
            out.push(dome_vertex(ring_i + 1, az_i));
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
