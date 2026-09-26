//! 作図機能(図形・線)のデータモデルと状態管理。DETAILED_DESIGN.md 6.11節。
//!
//! アプリは`DrawingState`を`provide_context`し、`add`/`update`/`remove`で図形を出し入れする。
//! `ui::terrain_view::TerrainView`が一覧の変化に追従して描き直す(未提供なら作図なしで動作する)。
//! ジオメトリの生成は`terrain::drawing_geometry`、GPUへの描画は`terrain::renderer`が行う。
//!
//! # 位置の置き方(`Position`)
//! 図形の位置は3種類の座標で置ける。**1つの図形の中では同じ種類にそろえること**
//! (混ぜた図形は描かれず、警告ログを出す)。
//! - `World`: 緯度経度+高度。**絶対座標に固定**され、地形と同じ奥行きで描く(山の陰に隠れる)。
//!   原点の変更・地形のLOD切り替えにも追従する。
//! - `Screen`: 画面(canvas)のピクセル座標。**カメラ(画面)に固定**され、カメラを動かしても動かない。
//!   常に地形の手前に、`Vec`の順に重ねて描く。2D図形・線だけ置ける。
//! - `View`: カメラからの相対位置(右・上・前方、メートル)。カメラに固定されたまま遠近法で描く
//!   (3D図形も置ける)。常に地形の手前に、図形どうしは奥行きで隠し合う。
//!
//! # 大きさの単位
//! `radius`・`width`等の大きさは、`World`と`View`ではメートル、`Screen`ではピクセル。

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

pub type DrawingId = u64;

/// 色(各成分0〜1、アルファは非乗算)。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const RED: Self = Self::rgb(1.0, 0.2, 0.2);
    pub const GREEN: Self = Self::rgb(0.2, 0.9, 0.3);
    pub const BLUE: Self = Self::rgb(0.2, 0.5, 1.0);
    pub const YELLOW: Self = Self::rgb(1.0, 0.9, 0.2);

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::rgba(r, g, b, 1.0)
    }

    /// 色はそのまま、不透明度だけを変えたもの。
    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    pub(crate) fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// 図形の見た目。
/// - 2D図形: `fill`が内側の塗り、`stroke`が輪郭線。
/// - 3D図形: `fill`が面の塗り(陰影が付く)、`stroke`が主な稜線(ワイヤーフレーム)。
/// - 折れ線: `stroke`が線の色。`fill`は使わない。
///
/// どちらも`None`なら何も描かない。アルファが1未満の色は半透明で描く。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub fill: Option<Color>,
    pub stroke: Option<Color>,
    /// 線・輪郭線の太さ(画面のピクセル)。拡大・縮小しても太さは変わらない。
    pub stroke_width_px: f32,
}

impl Style {
    /// 塗りだけ(輪郭線なし)。
    pub const fn filled(color: Color) -> Self {
        Self {
            fill: Some(color),
            stroke: None,
            stroke_width_px: 2.0,
        }
    }

    /// 線・輪郭線だけ(塗りなし)。折れ線はこれを使う。
    pub const fn stroked(color: Color, width_px: f32) -> Self {
        Self {
            fill: None,
            stroke: Some(color),
            stroke_width_px: width_px,
        }
    }

    pub const fn fill_and_stroke(fill: Color, stroke: Color, width_px: f32) -> Self {
        Self {
            fill: Some(fill),
            stroke: Some(stroke),
            stroke_width_px: width_px,
        }
    }

    /// 描く線の色(太さが0なら線は描かないので`None`)。
    pub(crate) fn visible_stroke(&self) -> Option<Color> {
        self.stroke.filter(|_| self.stroke_width_px > 0.0)
    }
}

impl Default for Style {
    /// 半透明の青い塗り+青い輪郭線(太さ2px)。
    fn default() -> Self {
        Self::fill_and_stroke(
            Color::rgba(0.2, 0.6, 1.0, 0.35),
            Color::rgb(0.2, 0.6, 1.0),
            2.0,
        )
    }
}

/// 高度の基準。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Altitude {
    /// 海抜(地形データの標高と同じ基準)のメートル。2D図形は**この高さの水平な面**になる
    /// (地球の丸みには沿うが、地形の起伏には沿わない)。
    Msl(f64),
    /// 地表からのメートル。2D図形・線は**地形の起伏に沿って貼り付く**(細かく分割して描く)。
    /// 3D図形は、位置の真下の地表の高さを基準に置く(図形自体は変形しない)。
    AboveGround(f64),
}

impl Altitude {
    /// 同じ基準のまま`dh`メートル高くしたもの。
    pub(crate) fn raised(self, dh: f64) -> Self {
        match self {
            Self::Msl(h) => Self::Msl(h + dh),
            Self::AboveGround(o) => Self::AboveGround(o + dh),
        }
    }
}

/// `Position::Screen`の基準になる画面の角。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

/// 図形を置く位置。種類ごとの意味はモジュールの説明を参照。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Position {
    World {
        lat_deg: f64,
        lon_deg: f64,
        altitude: Altitude,
    },
    /// `corner`から見て右へ`x_px`・下へ`y_px`(画面の向き。右下の角なら負の値で内側へ寄る)。
    Screen {
        corner: Corner,
        x_px: f64,
        y_px: f64,
    },
    View {
        right_m: f64,
        up_m: f64,
        forward_m: f64,
    },
}

impl Position {
    pub const fn world(lat_deg: f64, lon_deg: f64, altitude: Altitude) -> Self {
        Self::World {
            lat_deg,
            lon_deg,
            altitude,
        }
    }

    pub const fn screen(corner: Corner, x_px: f64, y_px: f64) -> Self {
        Self::Screen { corner, x_px, y_px }
    }

    pub const fn view(right_m: f64, up_m: f64, forward_m: f64) -> Self {
        Self::View {
            right_m,
            up_m,
            forward_m,
        }
    }

    pub fn space(&self) -> Space {
        match self {
            Self::World { .. } => Space::World,
            Self::Screen { .. } => Space::Screen,
            Self::View { .. } => Space::View,
        }
    }
}

/// 図形が属する座標の種類(`Position`の3種類に対応する)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Space {
    World,
    View,
    Screen,
}

/// 図形。回転角は時計回りの度数で、0度は上(`World`では北)向き。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    // ---- 2D図形(平面) ----
    /// 円。
    Circle { center: Position, radius: f64 },
    /// 矩形。`rotation_deg`だけ時計回りに回す。
    Rect {
        center: Position,
        width: f64,
        height: f64,
        rotation_deg: f64,
    },
    /// 多角形(3点以上、自己交差しない)。全頂点を**先頭の点の高度**の面に置く。
    Polygon { points: Vec<Position> },
    /// 扇形(レーダーの覆域など)。`start_deg`から`end_deg`まで時計回り。
    Sector {
        center: Position,
        radius: f64,
        start_deg: f64,
        end_deg: f64,
    },

    // ---- 3D図形(立体)。`World`では位置を通る鉛直線を軸にする ----
    /// 球。位置は中心。
    Sphere { center: Position, radius: f64 },
    /// 直方体。位置は底面の中心。`size_m`は[東西(横), 南北(奥行), 高さ]で、`heading_deg`だけ時計回りに回す。
    Cuboid {
        base_center: Position,
        size_m: [f64; 3],
        heading_deg: f64,
    },
    /// 円柱。位置は底面の中心。
    Cylinder {
        base_center: Position,
        radius: f64,
        height: f64,
    },
    /// 円錐。位置は底面の中心。
    Cone {
        base_center: Position,
        radius: f64,
        height: f64,
    },

    // ---- 線 ----
    /// 折れ線(2点以上)。`World`では点ごとに高度を持てて、点の間は大円に沿って細かく分割する。
    Polyline { points: Vec<Position> },
}

impl Shape {
    /// 図形を置いている座標の種類。位置が1つもない・種類が混ざっている・その種類に置けない図形
    /// (`Screen`に置いた3D図形)はエラーの理由を返す。
    pub fn validate(&self) -> Result<Space, &'static str> {
        let positions = self.positions();
        let Some(first) = positions.first() else {
            return Err("位置が指定されていない");
        };
        let space = first.space();
        if positions.iter().any(|p| p.space() != space) {
            return Err("位置の種類(World/Screen/View)が混ざっている");
        }
        if space == Space::Screen && self.is_solid() {
            return Err("3D図形は画面座標(Screen)に置けない(ViewかWorldを使う)");
        }
        Ok(space)
    }

    pub fn is_solid(&self) -> bool {
        matches!(
            self,
            Self::Sphere { .. } | Self::Cuboid { .. } | Self::Cylinder { .. } | Self::Cone { .. }
        )
    }

    /// 図形を構成する位置(点の並び、または中心1つ)。
    pub fn positions(&self) -> &[Position] {
        match self {
            Self::Circle { center, .. }
            | Self::Rect { center, .. }
            | Self::Sector { center, .. }
            | Self::Sphere { center, .. } => std::slice::from_ref(center),
            Self::Cuboid { base_center, .. }
            | Self::Cylinder { base_center, .. }
            | Self::Cone { base_center, .. } => std::slice::from_ref(base_center),
            Self::Polygon { points } | Self::Polyline { points } => points,
        }
    }

    /// `positions`の書き換え版(高度をまとめて変えるときなどに使う)。
    pub fn positions_mut(&mut self) -> &mut [Position] {
        match self {
            Self::Circle { center, .. }
            | Self::Rect { center, .. }
            | Self::Sector { center, .. }
            | Self::Sphere { center, .. } => std::slice::from_mut(center),
            Self::Cuboid { base_center, .. }
            | Self::Cylinder { base_center, .. }
            | Self::Cone { base_center, .. } => std::slice::from_mut(base_center),
            Self::Polygon { points } | Self::Polyline { points } => points,
        }
    }

    /// 地形の高さで形が決まるか(`AboveGround`の位置を持つか)。地形のLODが変わったら描き直す必要がある。
    pub fn depends_on_terrain(&self) -> bool {
        self.positions().iter().any(|p| {
            matches!(
                p,
                Position::World {
                    altitude: Altitude::AboveGround(_),
                    ..
                }
            )
        })
    }
}

/// 1つの作図要素(図形+見た目)。
#[derive(Debug, Clone, PartialEq)]
pub struct Drawing {
    pub id: DrawingId,
    pub shape: Shape,
    pub style: Style,
    /// falseなら描かない(一覧には残る)。
    pub visible: bool,
}

/// 作図の一覧。`TerrainView`が購読して描く。アプリは`provide_context`で1つだけ生成して渡す
/// (`DrawingState::new()`)。
#[derive(Clone, Copy)]
pub struct DrawingState {
    pub items: RwSignal<Vec<Drawing>>,
    next_id: RwSignal<u64>,
}

impl DrawingState {
    pub fn new() -> Self {
        Self {
            items: RwSignal::new(Vec::new()),
            next_id: RwSignal::new(1),
        }
    }

    /// 図形を追加して、そのIDを返す。後ろに追加したものほど手前に描く
    /// (`Screen`は重なりがこの順、`World`/`View`は奥行きで決まるので半透明どうしの重なりだけがこの順)。
    pub fn add(&self, shape: Shape, style: Style) -> DrawingId {
        let id = self.next_id.get_untracked();
        self.next_id.set(id + 1);
        self.items.update(|list| {
            list.push(Drawing {
                id,
                shape,
                style,
                visible: true,
            })
        });
        id
    }

    /// IDの図形を書き換える(位置・大きさ・色・表示/非表示など)。無ければ何もしない。
    pub fn update(&self, id: DrawingId, f: impl FnOnce(&mut Drawing)) {
        self.items.update(|list| {
            if let Some(drawing) = list.iter_mut().find(|d| d.id == id) {
                f(drawing);
            }
        });
    }

    /// IDの図形を読む(リアクティブに追跡する)。無ければ`None`。
    pub(crate) fn with<R>(&self, id: DrawingId, f: impl FnOnce(&Drawing) -> R) -> Option<R> {
        self.items
            .with(|list| list.iter().find(|d| d.id == id).map(f))
    }

    /// `with`と同じだが、リアクティブに追跡しない。
    pub(crate) fn with_untracked<R>(
        &self,
        id: DrawingId,
        f: impl FnOnce(&Drawing) -> R,
    ) -> Option<R> {
        untrack(|| self.with(id, f))
    }

    pub fn remove(&self, id: DrawingId) {
        self.items.update(|list| list.retain(|d| d.id != id));
    }

    pub fn clear(&self) {
        self.items.update(|list| list.clear());
    }
}

impl Default for DrawingState {
    fn default() -> Self {
        Self::new()
    }
}
