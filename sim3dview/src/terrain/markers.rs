//! レーダー観測点(見通し範囲の観測点)の状態管理と、マーカー・覆域の描画用ジオメトリ生成。
//! 観測点自体はメインパネル上の右クリックで追加する(`ui::terrain_view::TerrainView`)。
//! `RadarMarkersState`が状態(一覧・選択・覆域高度)を保持し、本モジュールの残りはその状態から
//! 描画用の頂点列を作るだけの純粋関数群。
//! - 観測点のマーカー: 画面サイズ固定のピン(ビルボード、`DrawVertex::billboard`)。`TerrainRenderer::update_markers`。
//! - 3Dの覆域ドーム: TriangleListの半透明の面(`TerrainVertex`)。`TerrainRenderer::update_dome`。
//!   覆域の計算(`start_dome_computation`・`start_coverage_computation`。重いので小分けに進める)と、
//!   計算結果からの頂点列の生成(`dome_geometry`・`coverage_2d_geometry`)は別の関数で、計算結果は使い回せる。
//! - 2Dの覆域: 塗り(半透明の三角形)+輪郭線(太い線)を`DrawVertex`で。深度テストなしで描く
//!   (`TerrainRenderer::update_coverage_2d`)。
//!
//! 別々のバッファ・パイプラインを使うため、頂点列を作る関数も分かれている。

use leptos::prelude::*;

use super::drawing_geometry::append_line_strip;
use super::geodesy::EnuTransform;
use super::heightmap::sample_heightmap;
use super::loader::TerrainData;
use super::los::{DomeComputation, DomeRing, LosParams, LosPoint, RangeComputation, RangeKind};
use super::mesh::TerrainVertex;
use super::origin::Origin;
use super::render_bias::{COVERAGE_AREA_M, DOME_M, MARKER_M};
use super::vertex::DrawVertex;
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
    /// 覆域を、選択中の観測点だけでなく**すべての観測点について同時に**表示するか(既定は選択中のみ)。
    /// 観測点ごとに色が違う(`coverage_colors`)。
    pub show_all_coverage: RwSignal<bool>,
}

impl RadarMarkersState {
    pub fn new() -> Self {
        Self {
            markers: RwSignal::new(Vec::new()),
            selected: RwSignal::new(None),
            next_id: RwSignal::new(1),
            coverage_altitude_m: RwSignal::new(1000.0),
            show_all_coverage: RwSignal::new(false),
        }
    }

    /// 指定した緯度経度に既定パラメータ(アンテナ高10m・最大観測範囲50km)のレーダーを
    /// 追加し、選択状態にする。
    pub fn add(&self, lat_deg: f64, lon_deg: f64) -> u64 {
        let id = self.next_id.get_untracked();
        self.next_id.set(id + 1);
        self.markers.update(|list| {
            list.push(RadarMarker {
                id,
                lat_deg,
                lon_deg,
                height_m: 10.0,
                max_range_m: 50_000.0,
            });
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

/// 観測点ごとの覆域の色(3Dドームの色, 2D覆域の塗りの色)。複数の覆域を同時に出すとき(`show_all_coverage`)に見分けるため、
/// 観測点の`id`で決める。1番目の観測点は、これまでどおりドームが水色・2Dが緑。
const COVERAGE_PALETTE: [([f32; 3], [f32; 3]); 6] = [
    ([0.30, 0.90, 1.00], [0.35, 0.90, 0.40]), // 水色 / 緑
    ([1.00, 0.55, 0.90], [1.00, 0.70, 0.25]), // 桃色 / 橙
    ([1.00, 0.95, 0.40], [0.45, 0.70, 1.00]), // 黄 / 青
    ([0.60, 1.00, 0.40], [1.00, 0.50, 0.70]), // 黄緑 / 桃
    ([1.00, 0.65, 0.30], [0.30, 0.90, 0.95]), // 橙 / 水色
    ([0.70, 0.55, 1.00], [0.95, 0.90, 0.30]), // 紫 / 黄
];

/// 観測点`id`の覆域の色: (3Dドームの色, 2D覆域の塗りの色, 2D覆域の輪郭線の色)。
pub fn coverage_colors(marker_id: u64) -> ([f32; 3], [f32; 3], [f32; 4]) {
    let (dome, area) = COVERAGE_PALETTE[(marker_id.max(1) as usize - 1) % COVERAGE_PALETTE.len()];
    // 輪郭線は、塗りの色を白へ寄せた不透明色。
    let outline = [
        0.5 * area[0] + 0.5,
        0.5 * area[1] + 0.5,
        0.5 * area[2] + 0.5,
        1.0,
    ];
    (dome, area, outline)
}

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
pub const DOME_RING_ELEVATIONS_DEG: [f64; 38] = [
    0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, // 1度刻み
    12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 30.0, // 2度刻み
    33.0, 36.0, 39.0, 42.0, 45.0, 48.0, 51.0, 54.0, 57.0, 60.0, // 3度刻み
    64.0, 68.0, 72.0, 76.0, 80.0, 84.0, 87.0, // 4度刻み(最上段は87度)
];
/// ドームの方位角の刻み(mil)。4なら1,600方位(50km先で約196m間隔)。計算量(方位数に比例)と頂点数を減らすために間引く。
pub const DOME_AZIMUTH_STEP: usize = 4;
/// 2D覆域の境界の方位角の刻み(mil)。2なら3,200方位(50km先で約98m間隔)。
pub const COVERAGE_AZIMUTH_STEP: usize = 2;
/// 最上段リングを1点(アペックス)に閉じる傘の三角形の数。全方位角を傘に使うと、
/// 極端に細い三角形が大量に1点へ重なり、半透明合成(アルファブレンド)の描画順依存の副作用で
/// カメラ操作中にチカチカして見えることが分かった(360方位角のときに確認)ので、方位角を間引いて
/// この数の三角形にする。リング間の四角形パッチは互いに重ならないので間引かない。
const DOME_APEX_SEGMENTS: usize = 48;
/// ドームのメッシュの頂点を置く方位の間引き(計算は`DOME_AZIMUTH_STEP`の刻みのまま、面を張るときだけ間引く。
/// 50km先で約390m間隔、半径50kmの円の弦の誤差は約0.4m)。
const DOME_MIN_RING_STRIDE: usize = 2;
/// 高いリングほど円周が短いので、方位の頂点をさらに間引く(隣のリングとは整数倍。2のべき乗個おき)。この間隔の上限。
/// 間引かないと最上部の細い三角形が大量に重なり、無駄が多く筋が出る。
const DOME_MAX_RING_STRIDE: usize = 32;

/// 2D地図モードでの覆域表示(指定した海抜高度での探知可能領域)の塗りの不透明度。色は観測点ごと(`coverage_colors`)で、
/// 3Dの覆域ドームとは別の色にして、見た目で区別できるようにする。
const COVERAGE_AREA_ALPHA: f32 = 0.32;
/// 覆域表示(2D)の境界線の太さ(画面のpx)。塗りは半透明で控えめなので、境界だけは不透明な太い線で描き、
/// 領域の輪郭が一目で分かるようにする(`los_view.rs`の2D極座標図が塗り+輪郭線の両方を持つのと同じ考え方)。
const COVERAGE_OUTLINE_WIDTH_PX: f32 = 2.5;

/// ドームのリングの方位の間引き間隔(何方位おきに頂点を置くか)。`DOME_MIN_RING_STRIDE`以上で、高い(円周が短い)リングほど
/// 大きくして、頂点の間隔が赤道側と同じくらいになるようにする。2のべき乗で、隣のリングとは整数倍になる。
fn ring_stride(elevation_deg: f64, num_azimuths: usize) -> usize {
    let inverse_cos = 1.0 / elevation_deg.to_radians().cos().max(1e-6);
    let mut stride = DOME_MIN_RING_STRIDE;
    // 浮動小数点の誤差(cos(60°)が0.5より少し大きい等)で、ちょうど2倍の仰角が1段手前になるのを防ぐ。
    while stride * 2 <= DOME_MAX_RING_STRIDE
        && (stride * 2) as f64 <= inverse_cos + 1e-9
        && num_azimuths.is_multiple_of(stride * 2)
    {
        stride *= 2;
    }
    stride
}

/// 隣り合う2リングの間の帯を三角形で埋める。`lower`・`upper`はリングの頂点(円環)で、`upper`の頂点数は`lower`の
/// 約数(`lower.len() / upper.len()`が整数)。下のリングの頂点が多い分は、上のリングの1辺に複数の三角形を
/// 扇状に付けてつなぐ(頂点の間引きの違うリングの間に、すき間もT字の継ぎ目も作らない)。表裏どちらも見えるので巻き順は問わない。
fn stitch_rings<V: Copy>(lower: &[V], upper: &[V], mut push: impl FnMut(V, V, V)) {
    let (n_lower, n_upper) = (lower.len(), upper.len());
    if n_upper == 0 || n_lower % n_upper != 0 {
        return;
    }
    let ratio = n_lower / n_upper;
    for j in 0..n_upper {
        let (u0, u1) = (upper[j], upper[(j + 1) % n_upper]);
        for i in 0..ratio {
            let (a, b) = (j * ratio + i, (j * ratio + i + 1) % n_lower);
            push(lower[a], lower[b], u0);
        }
        push(u0, u1, lower[((j + 1) * ratio) % n_lower]);
    }
}

/// 三角形を1つ追加する。
fn push_tri(out: &mut Vec<DrawVertex>, anchor: [f32; 3], color: [f32; 4], points: [[f32; 2]; 3]) {
    for p in points {
        out.push(DrawVertex::billboard(anchor, p, color));
    }
}

/// ピンの形(頭の円+先端の三角形)を、`anchor`の画面上に大きさ`head_radius`px・先端の高さ`tip_y`pxで積む。
/// 座標は先端の基準位置(0,0)から画面のpx(右・上が正)。先端の三角形の辺は頭の円の接線。
fn push_pin_shape(
    out: &mut Vec<DrawVertex>,
    anchor: [f32; 3],
    color: [f32; 4],
    head_radius: f32,
    tip_y: f32,
) {
    let center_y = PIN_HEAD_CENTER_PX;
    let circle = |i: usize| {
        let t = std::f32::consts::TAU * i as f32 / PIN_HEAD_SEGMENTS as f32;
        [head_radius * t.cos(), center_y + head_radius * t.sin()]
    };
    for i in 0..PIN_HEAD_SEGMENTS {
        push_tri(
            out,
            anchor,
            color,
            [[0.0, center_y], circle(i), circle(i + 1)],
        );
    }
    // 先端(0,tip_y)から頭の円へ引いた接線の接点。
    let beta = (head_radius / (center_y - tip_y)).acos();
    let tangent_x = head_radius * beta.sin();
    let tangent_y = center_y - head_radius * beta.cos();
    push_tri(
        out,
        anchor,
        color,
        [
            [0.0, tip_y],
            [-tangent_x, tangent_y],
            [tangent_x, tangent_y],
        ],
    );
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
    let anchor =
        mesh_transform.transform(marker.lat_deg, marker.lon_deg, ground_elevation + MARKER_M);
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
            [
                PIN_DOT_RADIUS_PX * t.cos(),
                PIN_HEAD_CENTER_PX + PIN_DOT_RADIUS_PX * t.sin(),
            ]
        };
        push_tri(
            out,
            anchor,
            MARKER_OUTLINE_COLOR,
            [[0.0, PIN_HEAD_CENTER_PX], point(i), point(i + 1)],
        );
    }
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
        let color = if is_selected {
            SELECTED_MARKER_COLOR
        } else {
            MARKER_COLOR
        };
        push_marker_pin(&mut out, data, &mesh_transform, marker, color);
    }
    out
}

/// 観測点の覆域ドームの計算(`compute_los_dome`)を始める(`azimuth_step`は`DOME_AZIMUTH_STEP`)。結果は`dome_geometry`へ渡す。
/// 全方位の計算は重いので、`advance`を時間で区切って呼ぶこと(`ui::terrain_view::coverage`)。
pub fn start_dome_computation(data: &TerrainData, marker: &RadarMarker) -> DomeComputation {
    let origin = Origin {
        lat_deg: marker.lat_deg,
        lon_deg: marker.lon_deg,
    };
    let params = LosParams {
        observer_height_m: marker.height_m,
        max_range_m: marker.max_range_m,
    };
    DomeComputation::new(
        data,
        &origin,
        &params,
        &DOME_RING_ELEVATIONS_DEG,
        DOME_AZIMUTH_STEP,
    )
}

/// 観測点の2D覆域(指定した海抜高度での探知可能領域)の計算を始める(`azimuth_step`は`COVERAGE_AZIMUTH_STEP`)。
/// 結果は`coverage_2d_geometry`へ渡す。
pub fn start_coverage_computation(
    data: &TerrainData,
    marker: &RadarMarker,
    target_altitude_m: f64,
) -> RangeComputation {
    let origin = Origin {
        lat_deg: marker.lat_deg,
        lon_deg: marker.lon_deg,
    };
    let params = LosParams {
        observer_height_m: marker.height_m,
        max_range_m: marker.max_range_m,
    };
    RangeComputation::new(
        data,
        &origin,
        &params,
        RangeKind::AtAltitude(target_altitude_m),
        COVERAGE_AZIMUTH_STEP,
    )
}

/// 覆域ドーム(半球状の面、TriangleList)の頂点列を作る。複数マーカーの覆域を同時に重ねると見づらいため、
/// 呼び出し側は選択中のマーカーについてのみ作る。`rings`は`start_dome_computation`の結果。
/// `mesh_origin`は現在GPUにアップロードされている地形メッシュの原点(頂点をこの原点基準のENU座標へ変換する)。
///
/// `compute_los_dome`が仰角ごとに求めるスラントレンジ(地形に遮蔽されない方角では最大観測
/// 範囲まで一定、遮蔽される方角だけ内側に凹む)をそのまま使い、隣接する2リングの間を三角形で
/// 埋めて球面状の面を作る(「ワイヤーフレームではなくSurfaceが存在する多面体に」という要望による)。
/// 高いリングほど方位の頂点を間引き(`ring_stride`)、最上段リングは、その半径の平均を高さとする頂点(アペックス)へ
/// 傘状に閉じて、開いた穴のない多面体にする。
pub fn dome_geometry(
    data: &TerrainData,
    mesh_origin: &Origin,
    marker: &RadarMarker,
    rings: &[DomeRing],
) -> Vec<TerrainVertex> {
    let mut out = Vec::new();
    let Some(num_azimuths) = rings.first().map(|r| r.points.len()) else {
        return out;
    };
    if num_azimuths < 2 || rings.len() < 2 {
        return out;
    }
    let mesh_transform = EnuTransform::new(mesh_origin, &data.metadata.ellipsoid);
    let radar_origin = Origin {
        lat_deg: marker.lat_deg,
        lon_deg: marker.lon_deg,
    };
    let local_transform = EnuTransform::new(&radar_origin, &data.metadata.ellipsoid);
    let observer_height = sample_heightmap(data, marker.lat_deg, marker.lon_deg).unwrap_or(0.0)
        as f64
        + marker.height_m;
    let (dome_color, _, _) = coverage_colors(marker.id);

    // ドーム上の頂点(リングごと・方位ごと。高いリングは間引く)を、地形メッシュのENU座標へ変換しておく。
    // パッチが各頂点を複数回使うので、先に1回ずつだけ計算する。
    let dome_vertices: Vec<Vec<TerrainVertex>> = rings
        .iter()
        .map(|ring| {
            let el_rad = ring.elevation_deg.to_radians();
            let stride = ring_stride(ring.elevation_deg, num_azimuths);
            (0..num_azimuths)
                .step_by(stride)
                .map(|az_i| {
                    let az_rad = ring.points[az_i].azimuth_deg.to_radians();
                    let range_m = ring.points[az_i].range_m;
                    let horizontal = range_m * el_rad.cos();
                    let (lat, lon) = local_transform
                        .inverse(horizontal * az_rad.sin(), horizontal * az_rad.cos());
                    let absolute_height = observer_height + range_m * el_rad.sin() + DOME_M;
                    TerrainVertex::unlit(
                        mesh_transform.transform(lat, lon, absolute_height),
                        dome_color,
                    )
                })
                .collect()
        })
        .collect();

    // リング間の帯(三角形)。
    for pair in dome_vertices.windows(2) {
        stitch_rings(&pair[0], &pair[1], |a, b, c| out.extend([a, b, c]));
    }

    // 最上段リングを、その半径の平均を高さとする頂点(アペックス)へ傘状の三角形群で閉じる
    // (最上段リングは仰角87度で、ほぼ真上。遮蔽がなければ半径=最大観測範囲=球の頂点の高さ)。
    let top = dome_vertices.last().expect("リングは2つ以上ある");
    let top_ring = rings.last().expect("リングは2つ以上ある");
    let avg_range =
        top_ring.points.iter().map(|p| p.range_m).sum::<f64>() / top_ring.points.len() as f64;
    let apex_pos = mesh_transform.transform(
        marker.lat_deg,
        marker.lon_deg,
        observer_height + avg_range + DOME_M,
    );
    let apex = TerrainVertex::unlit(apex_pos, dome_color);
    let steps: Vec<usize> = (0..top.len())
        .step_by((top.len() / DOME_APEX_SEGMENTS).max(1))
        .collect();
    for k in 0..steps.len() {
        out.extend([top[steps[k]], top[steps[(k + 1) % steps.len()]], apex]);
    }
    out
}

/// 選択中マーカーの、指定した海抜高度での探知可能領域(2D地図モード用)の塗り(地表面に
/// 沿って貼り付けた半透明のSurface)と、その外周の輪郭線(不透明な太い線)の頂点列を作る。
/// どちらも`DrawVertex`で、深度テストなしで描く(`TerrainRenderer::update_coverage_2d`)。2Dは真上からの
/// 正射影で地形に隠れることがないので、深度テストをすると、観測点から境界への大きな三角形が
/// 地形の起伏に埋まって、塗りが場所によって欠けて不均一になる。
///
/// `points`は`start_coverage_computation`の結果(全方位角の水平距離)。そのままの値を境界とする、
/// 観測点を中心とした星形(star-shaped)領域なので、観測点から境界上の隣接2点への三角形
/// (ファン)を並べるだけで自己交差のない面になる(3Dの覆域ドームのアペックス付近のような、
/// 視点回転時の半透明合成チカチカ対策の間引きは、2Dは常に真上固定視点で回転しないため不要)。
pub fn coverage_2d_geometry(
    data: &TerrainData,
    mesh_origin: &Origin,
    marker: &RadarMarker,
    points: &[LosPoint],
) -> Vec<DrawVertex> {
    let mut out = Vec::new();
    if points.len() < 2 {
        return out;
    }
    let mesh_transform = EnuTransform::new(mesh_origin, &data.metadata.ellipsoid);
    let radar_origin = Origin {
        lat_deg: marker.lat_deg,
        lon_deg: marker.lon_deg,
    };
    let local_transform = EnuTransform::new(&radar_origin, &data.metadata.ellipsoid);

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
        .map(|p| {
            let az_rad = p.azimuth_deg.to_radians();
            let (lat, lon) =
                local_transform.inverse(p.range_m * az_rad.sin(), p.range_m * az_rad.cos());
            position_at(lat, lon)
        })
        .collect();

    let (_, area_color, outline_color) = coverage_colors(marker.id);
    let fill = [
        area_color[0],
        area_color[1],
        area_color[2],
        COVERAGE_AREA_ALPHA,
    ];
    let center = position_at(marker.lat_deg, marker.lon_deg);
    let n = boundary.len();
    for i in 0..n {
        let (a, b) = (boundary[i], boundary[(i + 1) % n]);
        for position in [center, a, b] {
            out.push(DrawVertex::surface(position, fill, None));
        }
    }
    append_line_strip(
        &mut out,
        &boundary,
        true,
        outline_color,
        COVERAGE_OUTLINE_WIDTH_PX,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_stride_thins_high_rings_by_powers_of_two() {
        let n = 1600;
        assert_eq!(ring_stride(0.0, n), DOME_MIN_RING_STRIDE);
        assert_eq!(ring_stride(45.0, n), DOME_MIN_RING_STRIDE);
        assert_eq!(ring_stride(64.0, n), 2);
        assert_eq!(ring_stride(80.0, n), 4);
        assert_eq!(ring_stride(87.0, n), 16);
        // 上限を超えず、仰角が上がっても減らない。隣のリングとは整数倍(2のべき乗どうし)。
        let strides: Vec<usize> = DOME_RING_ELEVATIONS_DEG
            .iter()
            .map(|&e| ring_stride(e, n))
            .collect();
        assert!(strides
            .iter()
            .all(|&s| (DOME_MIN_RING_STRIDE..=DOME_MAX_RING_STRIDE).contains(&s) && n % s == 0));
        assert!(strides.windows(2).all(|w| w[1] >= w[0] && w[1] % w[0] == 0));
    }

    /// 頂点の間引きが違う2つのリングの間の帯に、すき間も継ぎ目のずれも無い(帯の辺は、リングの辺が1回、それ以外は2回)。
    #[test]
    fn stitched_band_between_rings_of_different_density_is_watertight() {
        use std::collections::HashMap;
        for (n_lower, n_upper) in [(8usize, 8usize), (8, 4), (16, 4), (12, 3)] {
            let lower: Vec<usize> = (0..n_lower).collect();
            let upper: Vec<usize> = (100..100 + n_upper).collect();
            let mut edges: HashMap<(usize, usize), usize> = HashMap::new();
            let mut triangles = 0;
            stitch_rings(&lower, &upper, |a, b, c| {
                triangles += 1;
                for (p, q) in [(a, b), (b, c), (c, a)] {
                    *edges.entry((p.min(q), p.max(q))).or_default() += 1;
                }
            });
            // 三角形の数 = 下のリングの辺 + 上のリングの辺。
            assert_eq!(triangles, n_lower + n_upper, "{n_lower}/{n_upper}");
            for (&(p, q), &count) in &edges {
                let is_ring_edge = (p < 100 && q < 100) || (p >= 100 && q >= 100);
                assert_eq!(
                    count,
                    if is_ring_edge { 1 } else { 2 },
                    "{n_lower}/{n_upper} edge {p}-{q}"
                );
            }
        }
    }

    /// 遮蔽のない平地では、ドームは観測点を中心とした球面になる(最大観測範囲の半径)。頂点数は方位を間引いた分だけ少ない。
    #[test]
    fn dome_over_flat_ground_is_a_smooth_sphere_with_thinned_vertices() {
        let data = TerrainData::synthetic(30, 120, 3, 3, |_, _| 0);
        let marker = RadarMarker {
            id: 1,
            lat_deg: 31.5,
            lon_deg: 121.5,
            height_m: 10.0,
            max_range_m: 30_000.0,
        };
        let mut computation = start_dome_computation(&data, &marker);
        computation.advance(&data, usize::MAX);
        let rings = computation.finish();
        assert_eq!(rings.len(), DOME_RING_ELEVATIONS_DEG.len());
        assert_eq!(rings[0].points.len(), 6400 / DOME_AZIMUTH_STEP);

        let mesh_origin = Origin {
            lat_deg: marker.lat_deg,
            lon_deg: marker.lon_deg,
        };
        let vertices = dome_geometry(&data, &mesh_origin, &marker, &rings);
        assert!(!vertices.is_empty() && vertices.len().is_multiple_of(3));
        // 以前(3200方位を間引かずに38リング)の頂点数(約71万)の3分の1未満。
        assert!(vertices.len() < 240_000, "{}", vertices.len());
        for v in &vertices {
            let [x, y, z] = v.position;
            let distance = (x * x + y * y + (z - 10.0) * (z - 10.0)).sqrt();
            // 地球の丸みで、最大観測範囲30kmの端は数十m下がる。球面から大きく外れる頂点(筋・突起)は無い。
            assert!((distance - 30_000.0).abs() < 400.0, "distance={distance}");
        }
    }

    #[test]
    fn each_marker_gets_its_own_coverage_color() {
        // 1番目は、これまでどおり(ドームが水色・2Dが緑)。
        let (dome1, area1, _) = coverage_colors(1);
        assert_eq!(dome1, [0.30, 0.90, 1.00]);
        assert_eq!(area1, [0.35, 0.90, 0.40]);
        // 隣り合う観測点は色が違い、パレットを使い切ったら最初に戻る。
        let colors: Vec<_> = (1..=6).map(|id| coverage_colors(id).0).collect();
        for i in 0..colors.len() {
            for j in i + 1..colors.len() {
                assert_ne!(colors[i], colors[j], "{i} と {j}");
            }
        }
        assert_eq!(coverage_colors(7).0, dome1);
        // 輪郭線は不透明で、塗りより明るい。
        let (_, area, outline) = coverage_colors(2);
        assert_eq!(outline[3], 1.0);
        assert!(outline[0] >= area[0] && outline[1] >= area[1] && outline[2] >= area[2]);
    }

    #[test]
    fn pin_is_built_from_billboard_triangles() {
        let mut out = Vec::new();
        push_pin_shape(&mut out, [1.0, 2.0, 3.0], [1.0, 0.0, 0.0, 1.0], 10.0, 0.0);
        // 頭の円(24分割)+先端の三角形。全頂点がアンカーを共有し、画面のオフセットだけが違う。
        assert_eq!(out.len(), (PIN_HEAD_SEGMENTS + 1) * 3);
        assert!(out
            .iter()
            .all(|v| v.position == [1.0, 2.0, 3.0] && v.params[2] == 1.0 && v.params[0] == 0.0));
        // 先端(0,0)が一番下、頭のてっぺんは中心+半径。
        let ys: Vec<f32> = out.iter().map(|v| v.aux[1]).collect();
        assert!(ys.iter().cloned().fold(f32::MAX, f32::min) >= -1e-4);
        let top = ys.iter().cloned().fold(f32::MIN, f32::max);
        assert!((top - (PIN_HEAD_CENTER_PX + 10.0)).abs() < 0.1);
    }
}
