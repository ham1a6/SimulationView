//! 図形の対話作成(地図をクリックして図形を置く)の状態と、作った図形の管理・保存。
//! DETAILED_DESIGN.md 6.11節。作図の一覧(`terrain::drawing::DrawingState`)の上に載る層で、
//! アプリは`DrawToolState`を`provide_context`する。
//!
//! - `ui::terrain_view::TerrainView`が地図のクリック・カーソル移動を`click`/`set_hover`で渡す。
//!   作成中の図形は`DrawingState`に**仮の図形**として置いて見せる(クリックのたびに更新する)。
//! - `ui::drawing_editor::DrawingEditor`が、ツールの選択・作った図形の一覧・数値編集のUIを出す。
//! - ここで作る図形はすべて絶対座標(`Position::World`)。アプリが`DrawingState`へ直接足した図形
//!   (デモ等)は一覧に出ず、保存の対象にもならない。
//!
//! # 点の置き方
//! 図形ごとに何回クリックするかが決まっている(`ToolKind::hint`参照)。多角形・折れ線だけは
//! 何点でも置けて、ダブルクリック/Enter(`finish`)で確定する。確定したあともツールは選んだままで、
//! 続けて次の図形を置ける(`cancel`かEscで抜ける)。

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use super::drawing::{Altitude, Color, Drawing, DrawingId, DrawingState, Position, Shape, Style};
use super::geodesy::Ellipsoid;
use super::geodesy::{from_local, to_local};

/// 直前の点とこれ未満(メートル)しか離れていないクリックは、ダブルクリックの2回目などとみなして無視する。
const MIN_POINT_SPACING_M: f64 = 1.0;

/// 作れる図形の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Circle,
    Rect,
    Polygon,
    Sector,
    Polyline,
    Sphere,
    Cuboid,
    Cylinder,
    Cone,
}

impl ToolKind {
    pub const ALL: [ToolKind; 9] = [
        Self::Circle,
        Self::Rect,
        Self::Polygon,
        Self::Sector,
        Self::Polyline,
        Self::Sphere,
        Self::Cuboid,
        Self::Cylinder,
        Self::Cone,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Circle => "円",
            Self::Rect => "矩形",
            Self::Polygon => "多角形",
            Self::Sector => "扇形",
            Self::Polyline => "折れ線",
            Self::Sphere => "球",
            Self::Cuboid => "直方体",
            Self::Cylinder => "円柱",
            Self::Cone => "円錐",
        }
    }

    /// 確定までに必要なクリック数。多角形・折れ線(何点でも置ける)は`None`。
    fn fixed_points(self) -> Option<usize> {
        match self {
            Self::Circle
            | Self::Rect
            | Self::Sphere
            | Self::Cuboid
            | Self::Cylinder
            | Self::Cone => Some(2),
            Self::Sector => Some(3),
            Self::Polygon | Self::Polyline => None,
        }
    }

    /// 多角形・折れ線を確定できる最小の点数。
    fn min_points(self) -> usize {
        match self {
            Self::Polygon => 3,
            _ => 2,
        }
    }

    /// `placed`個の点を置いたあとに、次に何をクリックするかの案内。
    fn hint(self, placed: usize) -> &'static str {
        match (self, placed) {
            (Self::Circle, 0) => "円の中心をクリック",
            (Self::Circle, _) => "円周上の点をクリック(中心からの距離が半径)",
            (Self::Rect, 0) => "矩形の1つ目の角をクリック",
            (Self::Rect, _) => "対角の角をクリック(東西・南北に沿った矩形)",
            (Self::Sector, 0) => "扇形の中心をクリック",
            (Self::Sector, 1) => "扇の開始側の縁をクリック(半径と開始方位)",
            (Self::Sector, _) => "扇の終了側の縁をクリック(開始から時計回り)",
            (Self::Polygon, 0) => "頂点を順にクリック(3点以上)",
            (Self::Polygon, _) => "次の頂点をクリック / ダブルクリックかEnterで確定",
            (Self::Polyline, 0) => "折れ線の始点をクリック",
            (Self::Polyline, _) => "次の点をクリック / ダブルクリックかEnterで確定",
            (Self::Sphere, 0) => "球の中心をクリック",
            (Self::Sphere, _) => "球の表面の点をクリック(中心からの距離が半径)",
            (Self::Cuboid, 0) => "直方体の底面の1つ目の角をクリック",
            (Self::Cuboid, _) => "底面の対角の角をクリック",
            (Self::Cylinder, 0) => "円柱の底面の中心をクリック",
            (Self::Cylinder, _) => "底面の円周上の点をクリック",
            (Self::Cone, 0) => "円錐の底面の中心をクリック",
            (Self::Cone, _) => "底面の円周上の点をクリック",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// クリックした点(緯度, 経度)から図形を作る(状態を持たない計算)
// ---------------------------------------------------------------------------------------------

type LatLon = (f64, f64);

fn radius_at(lat_deg: f64) -> f64 {
    // 地形データのellipsoidを持たないので、WGS84を使う(厳密に同じでなくても見た目に差は出ない)。
    Ellipsoid::WGS84.mean_radius(lat_deg)
}

/// `from`から見た`to`の位置([東, 北]、メートル)。
fn local(from: LatLon, to: LatLon) -> [f64; 2] {
    to_local(from.0, from.1, to.0, to.1, radius_at(from.0))
}

/// `from`から東へ`east`・北へ`north`メートル進んだ点。
fn offset(from: LatLon, east: f64, north: f64) -> LatLon {
    from_local(from.0, from.1, [east, north], radius_at(from.0))
}

fn distance(a: LatLon, b: LatLon) -> f64 {
    let l = local(a, b);
    l[0].hypot(l[1])
}

/// `from`から見た`to`の方位(北から時計回り、0〜360度)。
fn bearing_deg(from: LatLon, to: LatLon) -> f64 {
    let l = local(from, to);
    l[0].atan2(l[1]).to_degrees().rem_euclid(360.0)
}

/// クリックした点の並びから図形を作る。点が足りない・大きさが0になる場合は`None`。
/// `altitude`は全頂点の高度。球は地表に載るよう、`AboveGround`なら半径だけ持ち上げる。
pub fn build_shape(kind: ToolKind, pts: &[LatLon], altitude: Altitude) -> Option<Shape> {
    let at = |p: LatLon, alt: Altitude| Position::world(p.0, p.1, alt);
    // 半径が(ほぼ)0の図形は作らない。
    let sized = |r: f64| (r >= MIN_POINT_SPACING_M).then_some(r);
    match kind {
        ToolKind::Circle => {
            let [c, e, ..] = pts else { return None };
            Some(Shape::Circle {
                center: at(*c, altitude),
                radius: sized(distance(*c, *e))?,
            })
        }
        ToolKind::Sphere => {
            let [c, e, ..] = pts else { return None };
            let radius = sized(distance(*c, *e))?;
            let alt = match altitude {
                Altitude::AboveGround(offset) => Altitude::AboveGround(offset + radius),
                msl => msl,
            };
            Some(Shape::Sphere {
                center: at(*c, alt),
                radius,
            })
        }
        ToolKind::Cylinder | ToolKind::Cone => {
            let [c, e, ..] = pts else { return None };
            let radius = sized(distance(*c, *e))?;
            let base_center = at(*c, altitude);
            let height = radius * 2.0;
            Some(if kind == ToolKind::Cylinder {
                Shape::Cylinder {
                    base_center,
                    radius,
                    height,
                }
            } else {
                Shape::Cone {
                    base_center,
                    radius,
                    height,
                }
            })
        }
        ToolKind::Rect | ToolKind::Cuboid => {
            let [a, b, ..] = pts else { return None };
            let [dx, dy] = local(*a, *b);
            let (width, depth) = (sized(dx.abs())?, sized(dy.abs())?);
            let center = at(offset(*a, dx / 2.0, dy / 2.0), altitude);
            Some(if kind == ToolKind::Rect {
                Shape::Rect {
                    center,
                    width,
                    height: depth,
                    rotation_deg: 0.0,
                }
            } else {
                Shape::Cuboid {
                    base_center: center,
                    size_m: [width, depth, (width + depth) / 2.0],
                    heading_deg: 0.0,
                }
            })
        }
        ToolKind::Sector => {
            let [c, a, b, ..] = pts else { return None };
            let radius = sized(distance(*c, *a))?;
            sized(distance(*c, *b))?;
            let start_deg = bearing_deg(*c, *a);
            let sweep = (bearing_deg(*c, *b) - start_deg).rem_euclid(360.0);
            (sweep >= 1.0).then_some(Shape::Sector {
                center: at(*c, altitude),
                radius,
                start_deg,
                end_deg: start_deg + sweep,
            })
        }
        ToolKind::Polygon => (pts.len() >= kind.min_points()).then(|| Shape::Polygon {
            points: pts.iter().map(|p| at(*p, altitude)).collect(),
        }),
        ToolKind::Polyline => (pts.len() >= kind.min_points()).then(|| Shape::Polyline {
            points: pts.iter().map(|p| at(*p, altitude)).collect(),
        }),
    }
}

/// 作成中に見せる仮の図形。置いた点+カーソル位置で図形が作れればそれ、作れなければ(2点以上あれば)
/// それらをつないだ折れ線(扇形・多角形の途中経過)。
fn preview_shape(
    kind: ToolKind,
    pts: &[LatLon],
    hover: Option<LatLon>,
    altitude: Altitude,
) -> Option<Shape> {
    let mut all = pts.to_vec();
    all.extend(hover);
    if let Some(shape) = build_shape(kind, &all, altitude) {
        return Some(shape);
    }
    build_shape(ToolKind::Polyline, &all, altitude)
}

/// 図形のおおよその大きさ(メートル)。複製でずらす量の目安に使う。
fn characteristic_size_m(shape: &Shape) -> f64 {
    match shape {
        Shape::Circle { radius, .. }
        | Shape::Sphere { radius, .. }
        | Shape::Sector { radius, .. }
        | Shape::Cylinder { radius, .. }
        | Shape::Cone { radius, .. } => *radius,
        Shape::Rect { width, height, .. } => width.max(*height) / 2.0,
        Shape::Cuboid { size_m, .. } => size_m[0].max(size_m[1]) / 2.0,
        Shape::Polygon { .. } | Shape::Polyline { .. } => 1_000.0,
    }
}

/// 選択中の図形を目立たせる、黄色い太線の枠。図形自身の輪郭と重なって縞にならないよう、2D図形は少し持ち上げる。
fn highlight_shape(shape: &Shape) -> Shape {
    let mut shape = shape.clone();
    if !shape.is_solid() {
        for p in shape.positions_mut() {
            if let Position::World { altitude, .. } = p {
                *altitude = altitude.raised(5.0);
            }
        }
    }
    shape
}

fn highlight_style() -> Style {
    Style::stroked(Color::YELLOW, 4.0)
}

// ---------------------------------------------------------------------------------------------
// 状態
// ---------------------------------------------------------------------------------------------

/// このエディタで作った図形1つ分の情報(図形自体は`DrawingState`にある)。
#[derive(Debug, Clone, PartialEq)]
pub struct UserShape {
    pub id: DrawingId,
    pub name: String,
}

#[derive(Serialize, Deserialize)]
struct SavedShape {
    name: String,
    shape: Shape,
    style: Style,
    visible: bool,
}

#[derive(Serialize, Deserialize)]
struct SavedFile {
    version: u32,
    shapes: Vec<SavedShape>,
}

const SAVE_VERSION: u32 = 1;

/// 作図エディタの状態。`Copy`なので、そのままクロージャへ持ち込める。
#[derive(Clone, Copy)]
pub struct DrawToolState {
    pub drawings: DrawingState,
    /// 選んでいるツール(`None`なら地図のクリックは作図に使わない)。
    pub tool: RwSignal<Option<ToolKind>>,
    /// 作成中の図形に置いた点(緯度, 経度)。
    pub points: RwSignal<Vec<LatLon>>,
    /// 新しく作る図形の見た目。
    pub new_style: RwSignal<Style>,
    /// 新しく作る図形の高度(既定は地表に貼り付く`AboveGround(0)`)。
    pub new_altitude: RwSignal<Altitude>,
    /// このエディタで作った図形の一覧(作った順)。
    pub shapes: RwSignal<Vec<UserShape>>,
    /// 一覧で選択中の図形(地図上で黄色い枠になり、数値編集の対象になる)。
    pub selected: RwSignal<Option<DrawingId>>,
    /// 直近のカーソル位置(作成中の図形の先端)。
    hover: RwSignal<Option<LatLon>>,
    /// 作成中の仮の図形(`drawings`の中にある)。
    draft: RwSignal<Option<DrawingId>>,
    /// 選択中の図形の枠(`drawings`の中にある)。
    highlight: RwSignal<Option<DrawingId>>,
    /// 名前の通し番号。
    serial: RwSignal<u32>,
}

impl DrawToolState {
    pub fn new(drawings: DrawingState) -> Self {
        Self {
            drawings,
            tool: RwSignal::new(None),
            points: RwSignal::new(Vec::new()),
            // 地形(緑・茶)にも海(青)にも埋もれない橙。
            new_style: RwSignal::new(Style::fill_and_stroke(
                Color::rgba(1.0, 0.6, 0.1, 0.35),
                Color::rgb(1.0, 0.7, 0.2),
                2.0,
            )),
            new_altitude: RwSignal::new(Altitude::AboveGround(0.0)),
            shapes: RwSignal::new(Vec::new()),
            selected: RwSignal::new(None),
            hover: RwSignal::new(None),
            draft: RwSignal::new(None),
            highlight: RwSignal::new(None),
            serial: RwSignal::new(0),
        }
    }

    /// 作った図形をブラウザのlocalStorage(`key`)へ保存し、次回起動時に復元する。
    /// 作成直後(コンポーネント内)で1度だけ呼ぶこと。
    pub fn persist(self, key: &'static str) -> Self {
        if let Some(json) = read_storage(key) {
            match serde_json::from_str::<SavedFile>(&json) {
                Ok(file) if file.version == SAVE_VERSION => {
                    for s in file.shapes {
                        let id = self.add_user_shape(s.name, s.shape, s.style);
                        self.drawings.update(id, |d| d.visible = s.visible);
                    }
                }
                Ok(file) => log::warn!(
                    "[draw_tool] 保存された図形の版({})に未対応のため読み込まない",
                    file.version
                ),
                Err(e) => log::warn!("[draw_tool] 保存された図形を読み込めない: {e}"),
            }
        }
        Effect::new(move |prev: Option<String>| {
            let shapes = self.shapes.get();
            let saved = self.drawings.items.with(|items| {
                shapes
                    .iter()
                    .filter_map(|u| {
                        let d = items.iter().find(|d| d.id == u.id)?;
                        Some(SavedShape {
                            name: u.name.clone(),
                            shape: d.shape.clone(),
                            style: d.style,
                            visible: d.visible,
                        })
                    })
                    .collect::<Vec<_>>()
            });
            let json = serde_json::to_string(&SavedFile {
                version: SAVE_VERSION,
                shapes: saved,
            })
            .unwrap_or_default();
            if prev.as_deref() != Some(json.as_str()) {
                write_storage(key, &json);
            }
            json
        });
        self
    }

    // ---- ツール ----

    /// ツールを選ぶ(作成中の図形があれば捨てる)。同じツールをもう一度選ぶと解除する。
    pub fn start(&self, kind: ToolKind) {
        if self.tool.get_untracked() == Some(kind) {
            self.cancel();
            return;
        }
        self.points.set(Vec::new());
        self.tool.set(Some(kind));
        self.refresh_draft();
    }

    /// ツールを選び、地図の(緯度, 経度)を1点目として置く(右クリックメニューの「ここに図形を作成」用)。
    /// 作成中の図形があれば捨てる。`start`と違い、すでに同じツールを選んでいても解除しない。
    pub fn start_at(&self, kind: ToolKind, lat_deg: f64, lon_deg: f64) {
        self.points.set(Vec::new());
        self.hover.set(None);
        self.tool.set(Some(kind));
        self.click(lat_deg, lon_deg);
    }

    /// ツールを解除して、作成中の図形を捨てる。
    pub fn cancel(&self) {
        self.tool.set(None);
        self.points.set(Vec::new());
        self.hover.set(None);
        self.refresh_draft();
    }

    pub fn is_active(&self) -> bool {
        self.tool.get().is_some()
    }

    /// 作成中の点があるか(カーソル位置の追従が必要か)。
    pub fn wants_hover(&self) -> bool {
        self.tool.get_untracked().is_some() && !self.points.with_untracked(|p| p.is_empty())
    }

    /// 次に何をすればよいかの案内(ツール未選択なら空)。
    pub fn hint(&self) -> String {
        match self.tool.get() {
            Some(kind) => format!(
                "{}: {}",
                kind.label(),
                kind.hint(self.points.with(|p| p.len()))
            ),
            None => String::new(),
        }
    }

    /// 多角形・折れ線を確定できる状態か(確定ボタンの有効/無効)。
    pub fn can_finish(&self) -> bool {
        match self.tool.get() {
            Some(kind) if kind.fixed_points().is_none() => {
                self.points.with(|p| p.len() >= kind.min_points())
            }
            _ => false,
        }
    }

    /// 地図上の点(緯度, 経度)をクリックした。必要な点が揃った図形は確定する。
    pub fn click(&self, lat_deg: f64, lon_deg: f64) {
        let Some(kind) = self.tool.get_untracked() else {
            return;
        };
        let p = (lat_deg, lon_deg);
        let mut pts = self.points.get_untracked();
        if pts
            .last()
            .is_some_and(|last| distance(*last, p) < MIN_POINT_SPACING_M)
        {
            return;
        }
        pts.push(p);
        if kind.fixed_points() == Some(pts.len()) {
            match build_shape(kind, &pts, self.new_altitude.get_untracked()) {
                Some(shape) => {
                    self.commit(kind, shape);
                    pts.clear();
                }
                // 大きさが0など。最後の点だけ取り消して、置き直してもらう。
                None => {
                    pts.pop();
                }
            }
        }
        self.points.set(pts);
        self.refresh_draft();
    }

    /// カーソルが動いた(作成中の図形の先端を追従させる)。
    pub fn set_hover(&self, lat_deg: f64, lon_deg: f64) {
        let p = Some((lat_deg, lon_deg));
        if self.hover.get_untracked() != p {
            self.hover.set(p);
            self.refresh_draft();
        }
    }

    /// 多角形・折れ線を確定する。点が足りなければ何もしない。
    pub fn finish(&self) {
        let Some(kind) = self.tool.get_untracked() else {
            return;
        };
        if kind.fixed_points().is_some() {
            return;
        }
        let pts = self.points.get_untracked();
        if let Some(shape) = build_shape(kind, &pts, self.new_altitude.get_untracked()) {
            self.commit(kind, shape);
            self.points.set(Vec::new());
            self.refresh_draft();
        }
    }

    /// 置いた点を1つ戻す。1つも無ければツールを解除する。
    pub fn undo(&self) {
        let mut pts = self.points.get_untracked();
        if pts.pop().is_some() {
            self.points.set(pts);
            self.refresh_draft();
        } else {
            self.cancel();
        }
    }

    fn commit(&self, kind: ToolKind, shape: Shape) {
        let serial = self.serial.get_untracked() + 1;
        self.serial.set(serial);
        let id = self.add_user_shape(
            format!("{} {serial}", kind.label()),
            shape,
            self.new_style.get_untracked(),
        );
        self.select(Some(id));
    }

    fn add_user_shape(&self, name: String, shape: Shape, style: Style) -> DrawingId {
        let id = self.drawings.add(shape, style);
        self.shapes.update(|list| list.push(UserShape { id, name }));
        // 復元した図形の数だけ通し番号を進めておく(名前が重複しにくくなる)。
        self.serial
            .update(|n| *n = (*n).max(self.shapes.with_untracked(|l| l.len() as u32)));
        id
    }

    // ---- 作った図形の管理 ----

    /// 図形を選択する(`None`で解除)。地図上の黄色い枠も追従する。
    pub fn select(&self, id: Option<DrawingId>) {
        self.selected.set(id);
        self.refresh_highlight();
    }

    /// 図形を書き換える(位置・大きさ・見た目・表示/非表示)。選択中なら枠も更新する。
    pub fn update_shape(&self, id: DrawingId, f: impl FnOnce(&mut Drawing)) {
        self.drawings.update(id, f);
        if self.selected.get_untracked() == Some(id) {
            self.refresh_highlight();
        }
    }

    /// 図形を複製して、複製を選択する(重なって見分けが付かないよう、東北へ図形の大きさの半分ほどずらす)。
    /// 一覧に無い図形なら何もしない。
    pub fn duplicate(&self, id: DrawingId) {
        let Some(name) = self
            .shapes
            .with_untracked(|l| l.iter().find(|u| u.id == id).map(|u| u.name.clone()))
        else {
            return;
        };
        let Some((mut shape, style)) = self
            .drawings
            .with_untracked(id, |d| (d.shape.clone(), d.style))
        else {
            return;
        };
        let shift = characteristic_size_m(&shape) * 0.5;
        for p in shape.positions_mut() {
            if let Position::World {
                lat_deg, lon_deg, ..
            } = p
            {
                (*lat_deg, *lon_deg) = offset((*lat_deg, *lon_deg), shift, shift);
            }
        }
        let new_id = self.add_user_shape(format!("{name} のコピー"), shape, style);
        self.select(Some(new_id));
    }

    pub fn rename(&self, id: DrawingId, name: String) {
        self.shapes.update(|list| {
            if let Some(u) = list.iter_mut().find(|u| u.id == id) {
                u.name = name;
            }
        });
    }

    pub fn remove(&self, id: DrawingId) {
        self.drawings.remove(id);
        self.shapes.update(|list| list.retain(|u| u.id != id));
        if self.selected.get_untracked() == Some(id) {
            self.select(None);
        }
    }

    /// 作った図形をすべて消す(アプリが別に足した図形は残る)。
    pub fn remove_all(&self) {
        for u in self.shapes.get_untracked() {
            self.drawings.remove(u.id);
        }
        self.shapes.set(Vec::new());
        self.select(None);
    }

    // ---- 仮の図形・選択の枠(`drawings`の中の1要素を、あれば更新・無ければ追加・不要なら削除する) ----

    fn refresh_draft(&self) {
        let shape = self.tool.get_untracked().and_then(|kind| {
            let pts = self.points.get_untracked();
            if pts.is_empty() {
                return None;
            }
            preview_shape(
                kind,
                &pts,
                self.hover.get_untracked(),
                self.new_altitude.get_untracked(),
            )
        });
        // 仮の図形は、線の色を「なし」にしていても見えるようにする(折れ線は線の色だけで描くため)。
        let mut style = self.new_style.get_untracked();
        style.stroke.get_or_insert(Color::YELLOW);
        Self::sync_temp(self.drawings, self.draft, shape, style);
    }

    fn refresh_highlight(&self) {
        let shape = self.selected.get_untracked().and_then(|id| {
            self.drawings
                .with_untracked(id, |d| highlight_shape(&d.shape))
        });
        Self::sync_temp(self.drawings, self.highlight, shape, highlight_style());
    }

    fn sync_temp(
        drawings: DrawingState,
        slot: RwSignal<Option<DrawingId>>,
        shape: Option<Shape>,
        style: Style,
    ) {
        match (shape, slot.get_untracked()) {
            (Some(shape), Some(id)) => drawings.update(id, |d| {
                d.shape = shape;
                d.style = style;
            }),
            (Some(shape), None) => slot.set(Some(drawings.add(shape, style))),
            (None, Some(id)) => {
                drawings.remove(id);
                slot.set(None);
            }
            (None, None) => {}
        }
    }
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

fn read_storage(key: &str) -> Option<String> {
    storage()?.get_item(key).ok().flatten()
}

fn write_storage(key: &str, value: &str) {
    if let Some(storage) = storage() {
        if storage.set_item(key, value).is_err() {
            log::warn!("[draw_tool] 図形をlocalStorageへ保存できない");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROUND: Altitude = Altitude::AboveGround(0.0);
    const ORIGIN: LatLon = (35.0, 138.0);

    /// `ORIGIN`から東へ`east`・北へ`north`メートルの点。
    fn at(east: f64, north: f64) -> LatLon {
        offset(ORIGIN, east, north)
    }

    #[test]
    fn circle_radius_is_click_distance() {
        let Some(Shape::Circle { radius, .. }) =
            build_shape(ToolKind::Circle, &[ORIGIN, at(3_000.0, 4_000.0)], GROUND)
        else {
            panic!("円が作れない");
        };
        assert!((radius - 5_000.0).abs() < 1.0, "radius={radius}");
    }

    #[test]
    fn zero_size_shapes_are_rejected() {
        assert!(build_shape(ToolKind::Circle, &[ORIGIN, ORIGIN], GROUND).is_none());
        // 矩形は東西・南北のどちらかが0でも作れない。
        assert!(build_shape(ToolKind::Rect, &[ORIGIN, at(1_000.0, 0.0)], GROUND).is_none());
        assert!(build_shape(ToolKind::Circle, &[ORIGIN], GROUND).is_none());
    }

    #[test]
    fn rect_is_centered_between_corners() {
        let Some(Shape::Rect {
            center: Position::World {
                lat_deg, lon_deg, ..
            },
            width,
            height,
            rotation_deg,
        }) = build_shape(ToolKind::Rect, &[ORIGIN, at(2_000.0, -1_000.0)], GROUND)
        else {
            panic!("矩形が作れない");
        };
        assert!((width - 2_000.0).abs() < 1.0 && (height - 1_000.0).abs() < 1.0);
        assert_eq!(rotation_deg, 0.0);
        let mid = at(1_000.0, -500.0);
        assert!(distance((lat_deg, lon_deg), mid) < 1.0);
    }

    #[test]
    fn sector_sweeps_clockwise_from_start_to_end() {
        // 北(0度)→東(90度)。
        let Some(Shape::Sector {
            radius,
            start_deg,
            end_deg,
            ..
        }) = build_shape(
            ToolKind::Sector,
            &[ORIGIN, at(0.0, 1_000.0), at(1_000.0, 0.0)],
            GROUND,
        )
        else {
            panic!("扇形が作れない");
        };
        assert!((radius - 1_000.0).abs() < 1.0);
        assert!(
            start_deg.abs() < 0.1 && (end_deg - 90.0).abs() < 0.1,
            "{start_deg} {end_deg}"
        );
        // 東(90度)→北(0度)は、時計回りに270度。
        let Some(Shape::Sector {
            start_deg, end_deg, ..
        }) = build_shape(
            ToolKind::Sector,
            &[ORIGIN, at(1_000.0, 0.0), at(0.0, 1_000.0)],
            GROUND,
        )
        else {
            panic!("扇形が作れない");
        };
        assert!(
            (end_deg - start_deg - 270.0).abs() < 0.1,
            "{start_deg} {end_deg}"
        );
    }

    #[test]
    fn sphere_rests_on_ground() {
        let Some(Shape::Sphere {
            center: Position::World { altitude, .. },
            radius,
        }) = build_shape(ToolKind::Sphere, &[ORIGIN, at(500.0, 0.0)], GROUND)
        else {
            panic!("球が作れない");
        };
        assert_eq!(altitude, Altitude::AboveGround(radius));
    }

    #[test]
    fn polygon_and_polyline_need_enough_points() {
        let pts = [ORIGIN, at(1_000.0, 0.0), at(0.0, 1_000.0)];
        assert!(build_shape(ToolKind::Polygon, &pts[..2], GROUND).is_none());
        assert!(build_shape(ToolKind::Polygon, &pts, GROUND).is_some());
        assert!(build_shape(ToolKind::Polyline, &pts[..1], GROUND).is_none());
        assert!(build_shape(ToolKind::Polyline, &pts[..2], GROUND).is_some());
    }

    #[test]
    fn preview_falls_back_to_polyline() {
        // 扇形は中心+カーソルだけの段階では作れないので、線で見せる。
        let shape = preview_shape(ToolKind::Sector, &[ORIGIN], Some(at(1_000.0, 0.0)), GROUND);
        assert!(matches!(shape, Some(Shape::Polyline { .. })));
        // 点が1つでカーソルも無ければ何も見せない。
        assert!(preview_shape(ToolKind::Polygon, &[ORIGIN], None, GROUND).is_none());
    }

    #[test]
    fn shapes_survive_json_round_trip() {
        let file = SavedFile {
            version: SAVE_VERSION,
            shapes: vec![SavedShape {
                name: "円 1".into(),
                shape: build_shape(ToolKind::Circle, &[ORIGIN, at(1_000.0, 0.0)], GROUND).unwrap(),
                style: Style::default(),
                visible: true,
            }],
        };
        let json = serde_json::to_string(&file).unwrap();
        let back: SavedFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.shapes[0].shape, file.shapes[0].shape);
        assert_eq!(back.shapes[0].style, file.shapes[0].style);
    }
}
