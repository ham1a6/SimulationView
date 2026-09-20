//! 航跡(トラック)表示: シミュレーションなどから受け取った航空機・艦船・車両等の現在位置を、
//! シンボル(向きつきのアイコン)・ラベル・航跡(軌跡)・高度線で表示する。DETAILED_DESIGN.md 6.12節。
//!
//! アプリは`TracksState`を`provide_context`し、受信のたびに`set`で最新の一覧を渡す(通信プロトコルは
//! このライブラリは知らない。`sample/sim_frontend`が`sample/sim_server`のプロトコルとの橋渡しの例)。
//! `ui::terrain_view::TerrainView`が一覧の変化に追従して描き直す(未提供なら航跡表示なしで動作する)。
//!
//! - **シンボル**: 種別(`SymbolKind`)ごとの形を、所属(`Affiliation`)ごとの色で描く。画面サイズが固定で
//!   (拡大・縮小しても同じ大きさ)、進行方向(`heading_deg`)が画面上の実際の向きを指すよう回る
//!   (`draw.wgsl`の向きつきビルボード。3Dでカメラを回しても、2Dの地図でも正しい)。
//! - **ラベル**: 名前と高度・速度。`TerrainView`が重ねるHTML要素(`ui::terrain_view`)。
//! - **航跡**: 過去の位置をつないだ線。位置は`set`のたびに一定距離動いたものだけを、`TRAIL_MAX_POINTS`まで貯める。
//! - **高度線**: 空中のトラックから地表へ下ろす細い線(3Dのみ)。高度が分かりやすくなる。
//! - **選択**: 地図上のシンボルをクリックすると`TracksState::selected`にそのIDが入り、シンボルに強調の輪が付く
//!   (`TerrainView`が当たり判定`pick_track`を行う)。詳細の表示は呼び出し側(アプリ)が`selected_track`を読んで行う。

use std::collections::HashMap;

use leptos::prelude::*;

use super::drawing::{Altitude, Color};
use super::drawing_geometry::{append_line_strip, ear_clip, signed_area, BuildContext, DrawVertex};

pub type TrackId = u64;

/// シンボルの種別(形が変わる)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// 種別不明(ひし形)。
    Unknown,
    /// 固定翼機。
    Aircraft,
    Helicopter,
    Ship,
    /// 地上車両。
    Vehicle,
    Missile,
}

impl SymbolKind {
    /// 表示用の名前(日本語)。詳細パネルなどに使う。
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "不明",
            Self::Aircraft => "固定翼機",
            Self::Helicopter => "ヘリコプター",
            Self::Ship => "艦船",
            Self::Vehicle => "地上車両",
            Self::Missile => "ミサイル",
        }
    }
}

/// 所属(色が変わる)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Affiliation {
    Unknown,
    /// 友軍(青)。
    Friendly,
    /// 敵(赤)。
    Hostile,
    /// 中立(緑)。
    Neutral,
}

impl Affiliation {
    /// 表示用の名前(日本語)。
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "不明",
            Self::Friendly => "友軍",
            Self::Hostile => "敵",
            Self::Neutral => "中立",
        }
    }

    /// シンボル・航跡・ラベルの色。
    pub fn color(self) -> Color {
        match self {
            Self::Unknown => Color::rgb(1.0, 0.9, 0.3),
            Self::Friendly => Color::rgb(0.35, 0.65, 1.0),
            Self::Hostile => Color::rgb(1.0, 0.3, 0.3),
            Self::Neutral => Color::rgb(0.4, 0.9, 0.45),
        }
    }
}

/// 表示するトラック1つの現在の状態。
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// トラックの識別子(受信のたびに同じ実体は同じIDにする。航跡・ラベルの対応づけに使う)。
    pub id: TrackId,
    pub kind: SymbolKind,
    pub affiliation: Affiliation,
    /// 表示名(コールサイン等)。
    pub label: String,
    pub lat_deg: f64,
    pub lon_deg: f64,
    /// 海抜(`Msl`)か地表から(`AboveGround`。地上車両など、地形の高さがサーバー側にないとき)。
    pub altitude: Altitude,
    /// 進行方向(度、北から時計回り)。
    pub heading_deg: f64,
    /// 対地速度(m/s。ラベルに表示する)。
    pub speed_mps: f64,
}

/// トラックと、その航跡(過去の位置。現在位置は含まない)。
#[derive(Debug, Clone, PartialEq)]
pub struct TrackEntry {
    pub track: Track,
    pub trail: Vec<(f64, f64, Altitude)>,
}

/// 航跡に貯める過去の位置の最大数。
const TRAIL_MAX_POINTS: usize = 400;
/// 航跡に新しい点を足す、直前の点からの最小の距離(メートル)。
const TRAIL_MIN_STEP_M: f64 = 250.0;

/// 緯度経度の2点間のおおよその水平距離(メートル)。航跡の間引き用なので近似(等距円筒)で十分。
fn approx_distance_m(lat0: f64, lon0: f64, lat1: f64, lon1: f64) -> f64 {
    let north = (lat1 - lat0) * 111_320.0;
    let east = (lon1 - lon0) * 111_320.0 * lat0.to_radians().cos();
    east.hypot(north)
}

/// 前回の状態`previous`から、新しい航跡を作る: 前回の位置が航跡の最後の点から`TRAIL_MIN_STEP_M`以上
/// 離れていれば、その位置を航跡に足す(上限を超えたら古い方を捨てる)。
fn advance_trail(previous: Option<TrackEntry>) -> Vec<(f64, f64, Altitude)> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let mut trail = previous.trail;
    let last = previous.track;
    let far_enough = trail.last().is_none_or(|&(lat, lon, _)| {
        approx_distance_m(lat, lon, last.lat_deg, last.lon_deg) >= TRAIL_MIN_STEP_M
    });
    if far_enough {
        trail.push((last.lat_deg, last.lon_deg, last.altitude));
        if trail.len() > TRAIL_MAX_POINTS {
            trail.remove(0);
        }
    }
    trail
}

/// トラックの一覧と表示設定。`TerrainView`が購読して描く。アプリは`provide_context`で1つだけ生成して渡す
/// (`TracksState::new()`)。
#[derive(Clone, Copy)]
pub struct TracksState {
    pub entries: RwSignal<Vec<TrackEntry>>,
    /// ラベル(名前・高度・速度)を出すか。
    pub show_labels: RwSignal<bool>,
    /// 航跡(過去の位置をつないだ線)を出すか。
    pub show_trails: RwSignal<bool>,
    /// 高度線(空中のトラックから地表へ下ろす線。3Dのみ)を出すか。
    pub show_altitude_lines: RwSignal<bool>,
    /// 選択中のトラックのID。地図上のシンボルのクリックで`TerrainView`が設定し(何もない所のクリックで`None`)、
    /// アプリからも`select`/`set`で設定・解除できる。`selected_track`で中身を読める。
    pub selected: RwSignal<Option<TrackId>>,
}

impl TracksState {
    pub fn new() -> Self {
        Self {
            entries: RwSignal::new(Vec::new()),
            show_labels: RwSignal::new(true),
            show_trails: RwSignal::new(true),
            show_altitude_lines: RwSignal::new(true),
            selected: RwSignal::new(None),
        }
    }

    /// 選択中のトラックの最新の状態(選択がない・そのトラックが一覧から消えたら`None`)。
    /// リアクティブに読める(詳細パネルなどが、選択の変更・位置の更新に追従する)。
    pub fn selected_track(&self) -> Option<Track> {
        let id = self.selected.get()?;
        self.entries.with(|entries| entries.iter().find(|e| e.track.id == id).map(|e| e.track.clone()))
    }

    /// トラックを選択する(`None`で解除)。
    pub fn select(&self, id: Option<TrackId>) {
        if self.selected.get_untracked() != id {
            self.selected.set(id);
        }
    }

    /// トラックの一覧を最新のものへ置き換える(受信のたびに全件を渡す)。前回に無かったIDは新規、
    /// 今回に無いIDは消える(選択中のトラックが消えたら、選択も解除する)。航跡は同じIDの間だけ貯まる。
    pub fn set(&self, tracks: Vec<Track>) {
        let selected = self.selected.get_untracked();
        let selected_remains = selected.is_none_or(|id| tracks.iter().any(|t| t.id == id));
        self.entries.update(|entries| {
            let mut previous: HashMap<TrackId, TrackEntry> =
                entries.drain(..).map(|e| (e.track.id, e)).collect();
            for track in tracks {
                let trail = advance_trail(previous.remove(&track.id));
                entries.push(TrackEntry { track, trail });
            }
        });
        if !selected_remains {
            self.selected.set(None);
        }
    }

    /// すべてのトラックと航跡を消す(選択も解除する)。
    pub fn clear(&self) {
        self.entries.update(|entries| entries.clear());
        self.selected.set(None);
    }
}

impl Default for TracksState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------------------------
// ジオメトリ生成
// ---------------------------------------------------------------------------------------------

/// シンボルの大きさ(画面のpx。全体の縦の長さ)。
const SYMBOL_SIZE_PX: f32 = 30.0;
/// 縁取り(暗色)の大きさの倍率。
const SYMBOL_OUTLINE_SCALE: f32 = 1.3;
const SYMBOL_OUTLINE_COLOR: [f32; 4] = [0.04, 0.04, 0.07, 0.9];
/// 地表基準のトラックを、地表から持ち上げる高さ(メートル。地形メッシュに埋まらないように)。
const GROUND_BIAS_M: f64 = 25.0;
/// 高度線を出す、地表からの最小の高さ(メートル)。これより低いトラックは線を出さない。
const ALTITUDE_LINE_MIN_M: f64 = 30.0;
const ALTITUDE_LINE_WIDTH_PX: f32 = 1.0;
const TRAIL_WIDTH_PX: f32 = 1.5;
const LINE_ALPHA: f32 = 0.55;
/// 選択中のシンボルの強調の輪(画面のpx。縁取り→白の輪の順)。
const SELECT_RING_OUTER_PX: f32 = 26.0;
const SELECT_RING_INNER_PX: f32 = 21.0;
const SELECT_RING_BAND_PX: f32 = 3.0;
const SELECT_RING_SEGMENTS: usize = 40;
/// シンボルの当たり判定の半径(画面のpx。シンボルの縁取りより少し大きい)。
pub const PICK_RADIUS_PX: f32 = 20.0;

/// シンボルの形(ポリゴンの集まり)。座標は進行方向が+y・その右が+x、全体がおよそ[-1,1]の範囲。
fn glyph(kind: SymbolKind) -> Vec<Vec<[f64; 2]>> {
    // 左右対称の形は、右半分の点(機首から時計回りに尾まで)だけ書いて、左半分を折り返して閉じる
    // (中心線上の点(x=0)は折り返さない)。
    fn mirrored(right_half: &[[f64; 2]]) -> Vec<[f64; 2]> {
        let mut points: Vec<[f64; 2]> = right_half.to_vec();
        points.extend(right_half.iter().rev().filter(|p| p[0] != 0.0).map(|p| [-p[0], p[1]]));
        points
    }
    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<[f64; 2]> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }
    /// 中心を通り、`angle_deg`(時計回り、+yが0度)の向きに伸びる細長い矩形。
    fn bar(half_length: f64, half_width: f64, angle_deg: f64) -> Vec<[f64; 2]> {
        let (s, c) = angle_deg.to_radians().sin_cos();
        rect(-half_width, -half_length, half_width, half_length)
            .into_iter()
            .map(|[x, y]| [x * c + y * s, -x * s + y * c])
            .collect()
    }
    match kind {
        SymbolKind::Unknown => vec![vec![[0.0, 0.9], [-0.7, 0.0], [0.0, -0.9], [0.7, 0.0]]],
        // 機首・胴体・主翼・尾翼の輪郭(右半分。機首から時計回りに尾まで)。
        SymbolKind::Aircraft => vec![mirrored(&[
            [0.0, 1.0],
            [0.10, 0.62],
            [0.11, 0.18],
            [0.95, -0.22],
            [0.95, -0.40],
            [0.11, -0.14],
            [0.09, -0.62],
            [0.40, -0.86],
            [0.40, -0.98],
            [0.0, -0.86],
        ])],
        SymbolKind::Helicopter => {
            // 胴体(楕円)+ 尾部 + 交差した2本のローター。
            let body: Vec<[f64; 2]> = (0..16)
                .map(|i| {
                    let t = std::f64::consts::TAU * i as f64 / 16.0;
                    [0.32 * t.cos(), 0.1 + 0.5 * t.sin()]
                })
                .collect();
            vec![body, rect(-0.06, -1.0, 0.06, -0.3), bar(0.95, 0.06, 45.0), bar(0.95, 0.06, -45.0)]
        }
        // 船首がとがった船体。
        SymbolKind::Ship => vec![mirrored(&[[0.0, 1.0], [0.42, 0.45], [0.42, -0.85]])],
        // 車体+進行方向の三角。
        SymbolKind::Vehicle => vec![rect(-0.5, -0.7, 0.5, 0.55), vec![[0.0, 1.0], [-0.32, 0.6], [0.32, 0.6]]],
        // 細い弾体と尾部の安定翼。
        SymbolKind::Missile => vec![mirrored(&[
            [0.0, 1.0],
            [0.11, 0.62],
            [0.11, -0.55],
            [0.40, -1.0],
        ])],
    }
}

/// ポリゴンを三角形(3点ずつ)に分ける(向きはどちらでもよい)。
fn triangulate(polygon: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut poly = polygon.to_vec();
    if signed_area(&poly) < 0.0 {
        poly.reverse();
    }
    ear_clip(&poly)
}

/// シンボル1個ぶんの三角形を`out`に追加する(向きつきビルボード)。
fn push_symbol(
    out: &mut Vec<DrawVertex>,
    anchor: [f32; 3],
    heading_rad: f32,
    triangles: &[[f64; 2]],
    scale_px: f32,
    color: [f32; 4],
) {
    for p in triangles {
        let offset = [p[0] as f32 * scale_px, p[1] as f32 * scale_px];
        out.push(DrawVertex::oriented_billboard(anchor, offset, heading_rad, color));
    }
}

/// ラベル1つぶん: 表示位置(ENU座標。メッシュ原点基準)と、名前・詳細・色。`TerrainView`が毎フレーム画面へ射影して置く。
#[derive(Debug, Clone, PartialEq)]
pub struct TrackLabel {
    pub id: TrackId,
    /// 選択中か(ラベルを強調する)。
    pub selected: bool,
    pub position: [f32; 3],
    pub name: String,
    pub detail: String,
    pub color: [f32; 3],
}

/// 表示するもの(`TracksState`の設定+2D/3D)。
#[derive(Debug, Clone, Copy)]
pub struct TrackOptions {
    /// 強調の輪を付けるトラック。
    pub selected: Option<TrackId>,
    pub trails: bool,
    /// 高度線(3Dのみ。2D=真上から見た地図では縦の線が点になるので出さない)。
    pub altitude_lines: bool,
}

/// `build_track_geometry`の結果。
#[derive(Default)]
pub struct TrackGeometry {
    /// 描画用の頂点(TriangleList。シンボル・航跡・高度線)。
    pub vertices: Vec<DrawVertex>,
    pub labels: Vec<TrackLabel>,
}

/// 選択の強調の輪(円環)を`out`に追加する(画面サイズ固定のビルボード)。
fn push_ring(out: &mut Vec<DrawVertex>, anchor: [f32; 3], outer_px: f32, inner_px: f32, color: [f32; 4]) {
    let at = |r: f32, i: usize| {
        let t = std::f32::consts::TAU * i as f32 / SELECT_RING_SEGMENTS as f32;
        [r * t.cos(), r * t.sin()]
    };
    for i in 0..SELECT_RING_SEGMENTS {
        let (a, b, c, d) = (at(outer_px, i), at(outer_px, i + 1), at(inner_px, i + 1), at(inner_px, i));
        for p in [a, b, c, a, c, d] {
            out.push(DrawVertex::billboard(anchor, p, color));
        }
    }
}

/// 画面上の点`point`(canvas内のpx、左上原点)に最も近いシンボルのIDを返す。`anchors`は(ID, シンボルの位置(ENU座標))、
/// `radius_px`以内に無ければ`None`。カメラの後ろのシンボルは対象外。地形の陰に隠れたシンボルも対象になる(深度は見ない)。
pub fn pick_track(
    anchors: &[(TrackId, [f32; 3])],
    view_proj: &glam::Mat4,
    viewport_px: (f32, f32),
    point: (f32, f32),
    radius_px: f32,
) -> Option<TrackId> {
    let mut best: Option<(TrackId, f32)> = None;
    for &(id, [x, y, z]) in anchors {
        let clip = *view_proj * glam::Vec4::new(x, y, z, 1.0);
        if clip.w <= 0.0 {
            continue;
        }
        let sx = (clip.x / clip.w + 1.0) * 0.5 * viewport_px.0;
        let sy = (1.0 - clip.y / clip.w) * 0.5 * viewport_px.1;
        let distance = (sx - point.0).hypot(sy - point.1);
        if distance <= radius_px && best.is_none_or(|(_, d)| distance < d) {
            best = Some((id, distance));
        }
    }
    best.map(|(id, _)| id)
}

fn height_of(ctx: &BuildContext, lat_deg: f64, lon_deg: f64, altitude: Altitude) -> f64 {
    match altitude {
        Altitude::Msl(h) => h,
        Altitude::AboveGround(offset) => (ctx.ground)(lat_deg, lon_deg) + offset + GROUND_BIAS_M,
    }
}

fn label_detail(track: &Track) -> String {
    let (prefix, meters) = match track.altitude {
        Altitude::Msl(h) => ("", h),
        Altitude::AboveGround(o) => ("AGL ", o),
    };
    format!("{prefix}{meters:.0} m  {:.0} km/h", track.speed_mps * 3.6)
}

/// トラック一覧から、描画用の頂点とラベルを作る。
pub fn build_track_geometry(ctx: &BuildContext, entries: &[TrackEntry], options: TrackOptions) -> TrackGeometry {
    let mut geometry = TrackGeometry::default();
    // 種別ごとの三角形(同じ形を何度も三角形分割しないよう、種別ごとに1回だけ作る)。
    let mut glyphs: HashMap<u8, Vec<[f64; 2]>> = HashMap::new();
    let out = &mut geometry.vertices;

    for entry in entries {
        let track = &entry.track;
        let base = track.affiliation.color();
        let color = base.to_array();
        let height = height_of(ctx, track.lat_deg, track.lon_deg, track.altitude);
        let anchor = ctx.mesh_transform.transform(track.lat_deg, track.lon_deg, height);

        // 航跡: 過去の位置→現在位置。
        if options.trails && !entry.trail.is_empty() {
            let mut points: Vec<[f32; 3]> = entry
                .trail
                .iter()
                .map(|&(lat, lon, altitude)| {
                    ctx.mesh_transform.transform(lat, lon, height_of(ctx, lat, lon, altitude))
                })
                .collect();
            points.push(anchor);
            append_line_strip(out, &points, false, base.with_alpha(LINE_ALPHA).to_array(), TRAIL_WIDTH_PX);
        }

        // 高度線: 現在位置から真下(地表)へ。
        if options.altitude_lines {
            let ground = (ctx.ground)(track.lat_deg, track.lon_deg);
            if height - ground > ALTITUDE_LINE_MIN_M {
                let foot = ctx.mesh_transform.transform(track.lat_deg, track.lon_deg, ground + GROUND_BIAS_M);
                append_line_strip(out, &[anchor, foot], false, base.with_alpha(LINE_ALPHA).to_array(), ALTITUDE_LINE_WIDTH_PX);
            }
        }

        // 選択中: シンボルの後ろに強調の輪(縁取り→白)。
        if options.selected == Some(track.id) {
            push_ring(out, anchor, SELECT_RING_OUTER_PX, SELECT_RING_INNER_PX - 1.0, SYMBOL_OUTLINE_COLOR);
            push_ring(out, anchor, SELECT_RING_INNER_PX + SELECT_RING_BAND_PX, SELECT_RING_INNER_PX, [1.0, 1.0, 1.0, 1.0]);
        }

        // シンボル: 縁取り(暗色、少し大きく)→本体。
        let triangles = glyphs.entry(track.kind as u8).or_insert_with(|| {
            glyph(track.kind).iter().flat_map(|polygon| triangulate(polygon)).collect()
        });
        let heading = track.heading_deg.to_radians() as f32;
        let half = SYMBOL_SIZE_PX * 0.5;
        push_symbol(out, anchor, heading, triangles, half * SYMBOL_OUTLINE_SCALE, SYMBOL_OUTLINE_COLOR);
        push_symbol(out, anchor, heading, triangles, half, color);

        geometry.labels.push(TrackLabel {
            id: track.id,
            selected: options.selected == Some(track.id),
            position: anchor,
            name: track.label.clone(),
            detail: label_detail(track),
            color: [base.r, base.g, base.b],
        });
    }
    geometry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::loader::Ellipsoid;
    use crate::terrain::mesh::{EnuTransform, Origin};

    const WGS84: Ellipsoid = Ellipsoid { a_m: 6378137.0, inv_f: 298.257222101 };
    const ORIGIN: Origin = Origin { lat_deg: 35.355556, lon_deg: 138.859722 };

    fn track(id: u64, lat: f64, lon: f64, altitude: Altitude) -> Track {
        Track {
            id,
            kind: SymbolKind::Aircraft,
            affiliation: Affiliation::Friendly,
            label: format!("T{id}"),
            lat_deg: lat,
            lon_deg: lon,
            altitude,
            heading_deg: 90.0,
            speed_mps: 200.0,
        }
    }

    fn build(entries: &[TrackEntry], options: TrackOptions) -> TrackGeometry {
        let transform = EnuTransform::new(&ORIGIN, &WGS84);
        let ground = |_: f64, _: f64| 100.0;
        let ctx = BuildContext { mesh_transform: &transform, ellipsoid: &WGS84, ground: &ground, viewport_px: (800.0, 600.0) };
        build_track_geometry(&ctx, entries, options)
    }

    #[test]
    fn every_glyph_triangulates_with_positive_area_inside_unit_box() {
        for kind in [
            SymbolKind::Unknown,
            SymbolKind::Aircraft,
            SymbolKind::Helicopter,
            SymbolKind::Ship,
            SymbolKind::Vehicle,
            SymbolKind::Missile,
        ] {
            let polygons = glyph(kind);
            assert!(!polygons.is_empty());
            for polygon in &polygons {
                assert!(polygon.len() >= 3, "{kind:?}");
                let triangles = triangulate(polygon);
                assert_eq!(triangles.len(), (polygon.len() - 2) * 3, "{kind:?}: 単純多角形は n-2 個の三角形になる");
                let area: f64 = triangles
                    .chunks_exact(3)
                    .map(|t| ((t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[1][1] - t[0][1]) * (t[2][0] - t[0][0])).abs() * 0.5)
                    .sum();
                assert!((area - signed_area(polygon).abs()).abs() < 1e-9, "{kind:?}: 三角形の面積の和が多角形の面積と一致する");
                assert!(polygon.iter().all(|p| p[0].abs() <= 1.0 && p[1].abs() <= 1.0), "{kind:?}");
            }
        }
    }

    #[test]
    fn aircraft_glyph_is_symmetric_and_points_forward() {
        let polygon = &glyph(SymbolKind::Aircraft)[0];
        // 機首(0,1)が最も前にあり、左右対称(x座標の和が0)。
        assert!(polygon.iter().all(|p| p[1] <= 1.0 + 1e-12));
        assert_eq!(polygon[0], [0.0, 1.0]);
        let sum_x: f64 = polygon.iter().map(|p| p[0]).sum();
        assert!(sum_x.abs() < 1e-9, "sum_x={sum_x}");
    }

    #[test]
    fn symbol_is_oriented_billboard_with_heading_and_outline() {
        let entries = [TrackEntry { track: track(1, 35.4, 138.9, Altitude::Msl(3000.0)), trail: vec![] }];
        let geometry = build(&entries, TrackOptions { selected: None, trails: false, altitude_lines: false });
        assert_eq!(geometry.labels.len(), 1);
        let v = &geometry.vertices;
        assert!(!v.is_empty() && v.len() % 6 == 0, "縁取りと本体で同じ数の三角形");
        // 前半が縁取り(暗色)、後半が本体(所属の色)。全頂点が同じアンカー・進行方向・向きつきビルボード。
        let (outline, body) = v.split_at(v.len() / 2);
        assert!(outline.iter().all(|x| x.color == SYMBOL_OUTLINE_COLOR));
        assert!(body.iter().all(|x| x.color == Affiliation::Friendly.color().to_array()));
        assert!(v.iter().all(|x| x.position == v[0].position && x.params[2] == 2.0));
        assert!((v[0].params[0] - std::f32::consts::FRAC_PI_2).abs() < 1e-6, "進行方向90度=東");
        // 縁取りは本体より大きい。
        let extent = |vs: &[DrawVertex]| vs.iter().map(|x| x.aux[1].abs().max(x.aux[0].abs())).fold(0.0, f32::max);
        assert!(extent(outline) > extent(body));
        assert!((extent(body) - SYMBOL_SIZE_PX * 0.5).abs() < 1e-3, "機首が半分の大きさ");
    }

    #[test]
    fn altitude_line_only_for_aircraft_high_above_ground() {
        let high = TrackEntry { track: track(1, 35.4, 138.9, Altitude::Msl(3000.0)), trail: vec![] };
        let low = TrackEntry { track: track(2, 35.4, 138.9, Altitude::AboveGround(0.0)), trail: vec![] };
        let lines = |entry: &TrackEntry, altitude_lines| {
            build(std::slice::from_ref(entry), TrackOptions { selected: None, trails: false, altitude_lines })
                .vertices
                .iter()
                .filter(|v| v.params[0] > 0.0 && v.params[2] == 0.0)
                .count()
        };
        assert_eq!(lines(&high, true), 6, "線分1本=三角形2枚");
        assert_eq!(lines(&high, false), 0, "設定でOFF");
        assert_eq!(lines(&low, true), 0, "地表すれすれには出さない");
    }

    #[test]
    fn trail_grows_only_when_moved_and_is_capped() {
        // 初回は航跡なし。動かない間は増えない。250m以上動くと、前回の位置が航跡に入る。
        let a = TrackEntry { track: track(1, 35.0, 139.0, Altitude::Msl(0.0)), trail: vec![] };
        assert!(advance_trail(None).is_empty());
        let trail = advance_trail(Some(a.clone()));
        assert_eq!(trail.len(), 1, "空の航跡には最初の位置が入る");
        let b = TrackEntry { track: track(1, 35.0001, 139.0, Altitude::Msl(0.0)), trail: trail.clone() };
        assert_eq!(advance_trail(Some(b)).len(), 1, "11m動いただけでは増えない");
        let c = TrackEntry { track: track(1, 35.01, 139.0, Altitude::Msl(0.0)), trail };
        assert_eq!(advance_trail(Some(c)).len(), 2, "1km動いたら増える");
        // 上限を超えたら古い方から捨てる。
        let long: Vec<(f64, f64, Altitude)> =
            (0..TRAIL_MAX_POINTS).map(|i| (10.0 + i as f64 * 0.01, 139.0, Altitude::Msl(0.0))).collect();
        let d = TrackEntry { track: track(1, 20.0, 139.0, Altitude::Msl(0.0)), trail: long.clone() };
        let capped = advance_trail(Some(d));
        assert_eq!(capped.len(), TRAIL_MAX_POINTS);
        assert_eq!(capped[0], long[1]);
        assert_eq!(capped.last().unwrap().0, 20.0);
    }

    #[test]
    fn trail_line_connects_past_positions_to_current() {
        let trail = vec![(35.30, 138.80, Altitude::Msl(3000.0)), (35.35, 138.85, Altitude::Msl(3000.0))];
        let entry = TrackEntry { track: track(1, 35.4, 138.9, Altitude::Msl(3000.0)), trail };
        let options = TrackOptions { selected: None, trails: true, altitude_lines: false };
        let with = build(std::slice::from_ref(&entry), options).vertices;
        let without = build(&[TrackEntry { trail: vec![], ..entry.clone() }], options).vertices;
        // 航跡の点は3つ(過去2+現在)=線分2本=三角形4枚=12頂点。
        assert_eq!(with.len() - without.len(), 12);
        assert!(with.iter().any(|v| v.params[0] == TRAIL_WIDTH_PX && (v.color[3] - LINE_ALPHA).abs() < 1e-6));
    }

    #[test]
    fn label_text_shows_altitude_and_speed() {
        let msl = track(1, 35.0, 139.0, Altitude::Msl(3000.0));
        assert_eq!(label_detail(&msl), "3000 m  720 km/h");
        let agl = track(2, 35.0, 139.0, Altitude::AboveGround(120.0));
        assert_eq!(label_detail(&agl), "AGL 120 m  720 km/h");
    }

    #[test]
    fn selected_track_gets_a_ring_and_a_highlighted_label() {
        let entries = [
            TrackEntry { track: track(1, 35.4, 138.9, Altitude::Msl(3000.0)), trail: vec![] },
            TrackEntry { track: track(2, 35.5, 139.0, Altitude::Msl(3000.0)), trail: vec![] },
        ];
        let none = build(&entries, TrackOptions { selected: None, trails: false, altitude_lines: false });
        let one = build(&entries, TrackOptions { selected: Some(2), trails: false, altitude_lines: false });
        // 輪は縁取りと白の2本の円環(1本=分割数x三角形2枚x3頂点)。
        assert_eq!(one.vertices.len() - none.vertices.len(), 2 * SELECT_RING_SEGMENTS * 6);
        assert_eq!(one.labels.iter().map(|l| (l.id, l.selected)).collect::<Vec<_>>(), [(1, false), (2, true)]);
        // 輪は選択したトラックの位置(アンカー)にあり、白い輪が含まれ、円環が縁取りの中(縁取りの半径以内)に収まる。
        let anchor = one.labels[1].position;
        let ring: Vec<&DrawVertex> = one.vertices.iter().filter(|v| v.params[2] == 1.0).collect();
        assert!(ring.iter().all(|v| v.position == anchor));
        assert!(ring.iter().any(|v| v.color == [1.0, 1.0, 1.0, 1.0]));
        let max_radius = ring.iter().map(|v| v.aux[0].hypot(v.aux[1])).fold(0.0, f32::max);
        assert!((max_radius - SELECT_RING_OUTER_PX).abs() < 1e-3, "max_radius={max_radius}");
        // 輪はシンボルより大きい(シンボルの縁取りは半径19.5px)。
        assert!(SELECT_RING_INNER_PX - 1.0 > SYMBOL_SIZE_PX * 0.5 * SYMBOL_OUTLINE_SCALE);
    }

    #[test]
    fn pick_chooses_the_nearest_symbol_within_the_radius() {
        // clip = (x, y, z, -z): z<0が視点の前(w=-z>0)、z>0はカメラの後ろ(w<0)。
        let vp = glam::Mat4::from_cols(
            glam::Vec4::new(1.0, 0.0, 0.0, 0.0),
            glam::Vec4::new(0.0, 1.0, 0.0, 0.0),
            glam::Vec4::new(0.0, 0.0, 1.0, -1.0),
            glam::Vec4::ZERO,
        );
        // 画面(800x600)の中心は(0,0)。x=+0.1(z=-1でw=1) → 画面で右へ40px。
        let anchors = [(1, [0.0, 0.0, -1.0]), (2, [0.1, 0.0, -1.0]), (3, [0.0, 0.0, 1.0])];
        let center = (400.0, 300.0);
        assert_eq!(pick_track(&anchors, &vp, (800.0, 600.0), center, PICK_RADIUS_PX), Some(1));
        assert_eq!(pick_track(&anchors, &vp, (800.0, 600.0), (435.0, 300.0), PICK_RADIUS_PX), Some(2), "近い方");
        assert_eq!(pick_track(&anchors, &vp, (800.0, 600.0), (420.0, 300.0), PICK_RADIUS_PX), Some(1), "20px離れた1と、20px離れた2は、先に見つけた近い方(同距離なら先)");
        assert_eq!(pick_track(&anchors, &vp, (800.0, 600.0), (600.0, 100.0), PICK_RADIUS_PX), None, "半径の外");
        // カメラの後ろのアンカー(3)は、画面上の位置が同じでも選ばれない。
        assert_eq!(pick_track(&[(3, [0.0, 0.0, 1.0])], &vp, (800.0, 600.0), center, PICK_RADIUS_PX), None);
    }

    #[test]
    fn labels_are_japanese_and_selected_track_survives_updates() {
        assert_eq!(SymbolKind::Aircraft.label(), "固定翼機");
        assert_eq!(Affiliation::Hostile.label(), "敵");
        // 選択中のIDが次の一覧にも有れば選択は続き、無ければ解除される(リアクティブなownerが要るので、状態の判定だけを確認)。
        let owner = leptos::reactive::owner::Owner::new();
        owner.with(|| {
            let state = TracksState::new();
            state.set(vec![track(1, 35.0, 139.0, Altitude::Msl(0.0)), track(2, 35.1, 139.1, Altitude::Msl(0.0))]);
            state.select(Some(2));
            assert_eq!(state.selected_track().map(|t| t.label), Some("T2".to_string()));
            state.set(vec![track(2, 35.2, 139.2, Altitude::Msl(0.0))]);
            assert_eq!(state.selected.get_untracked(), Some(2));
            assert_eq!(state.selected_track().map(|t| t.lat_deg), Some(35.2), "最新の位置");
            state.set(vec![track(1, 35.0, 139.0, Altitude::Msl(0.0))]);
            assert_eq!(state.selected.get_untracked(), None, "選択中のトラックが消えたら解除");
            state.select(Some(1));
            state.clear();
            assert_eq!(state.selected.get_untracked(), None);
        });
    }
}
