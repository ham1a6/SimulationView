//! 作図機能(`terrain::drawing`)の描画用ジオメトリ生成。DETAILED_DESIGN.md 6.11節。
//! `Drawing`の一覧から、GPUへそのまま上げられる頂点列(`DrawVertex`のTriangleList)を作る純粋関数群。
//! 面も太い線も三角形で表し、線の太さ(画面のピクセル)への展開は頂点シェーダー(`draw.wgsl`)が行う。
//!
//! 頂点列は座標の種類ごと(`Space`)に分ける。`World`はENU座標(現在の地形メッシュの原点基準)、
//! `View`はカメラから見た座標(右・上・-前方)、`Screen`は画面のピクセル座標(左上原点)。
//! 不透明(アルファ1)と半透明は別の列にする(半透明は深度を書かずに描くため。`Batch`)。
//! `Screen`は重なりを追加順にするので1列にまとめる。
//!
//! 2D図形は、置いた位置を中心とする平面のローカル座標(x=右/東, y=上/北、メートルまたはピクセル)で
//! 三角形・輪郭線を作り(`Geom2d`)、座標の種類ごとの変換(`Frame2d`)で出力の座標にする。
//! `World`では、ローカル座標を基準点からの方位・距離とみなして緯度経度へ戻し(球面の直接解、
//! 半径は基準点の平均曲率半径)、高度を与えてENUへ変換する。地表に貼り付ける図形は、地形の
//! 起伏に沿うよう細かく分割する。3D図形は、位置における局所ENU座標で作って厳密に変換する。

use std::f64::consts::{PI, TAU};

use super::drawing::{Altitude, Corner, Drawing, Position, Shape, Space, Style};
use super::geodesy::Ellipsoid;
use super::geodesy::EnuTransform;
use super::origin::Origin;
/// 描画用の頂点。面と線(太さ付き)を同じ頂点形式・同じパイプラインで描く(`draw.wgsl`)。
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DrawVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
    /// 面: 単位法線(陰影を付けるとき)。線: 反対側の端点。
    pub aux: [f32; 3],
    /// x: 線の太さ(px。0なら面)。y: 線の側(-1/+1)。z: ビルボードの種類(0=面・線、1=画面サイズ固定のマーカー、
    /// 2=向きつきシンボル。`draw.wgsl`の`vs_main`が見る)。w: 陰影を付けるなら1(面のみ)。
    pub params: [f32; 4],
}

impl DrawVertex {
    pub(crate) fn surface(position: [f32; 3], color: [f32; 4], normal: Option<[f32; 3]>) -> Self {
        match normal {
            Some(aux) => Self { position, color, aux, params: [0.0, 0.0, 0.0, 1.0] },
            None => Self { position, color, aux: [0.0; 3], params: [0.0; 4] },
        }
    }

    fn line(position: [f32; 3], other: [f32; 3], color: [f32; 4], width_px: f32, side: f32) -> Self {
        Self { position, color, aux: other, params: [width_px, side, 0.0, 0.0] }
    }

    /// ビルボード(画面サイズ固定のマーカー)の頂点。`anchor`は3D空間の位置、`offset_px`はそこからの
    /// 画面上のずれ(px、右・上が正)。拡大・縮小しても大きさが変わらず、常に画面の正面を向く。
    pub(crate) fn billboard(anchor: [f32; 3], offset_px: [f32; 2], color: [f32; 4]) -> Self {
        Self { position: anchor, color, aux: [offset_px[0], offset_px[1], 0.0], params: [0.0, 0.0, 1.0, 0.0] }
    }

    /// 向きつきビルボード(`billboard`と同じだが、`offset_px`を「進行方向が画面のどちらを向くか」に合わせて回す)。
    /// `offset_px`は進行方向が上(+y)・その右が+xの座標で、`heading_rad`は北から時計回りの進行方向(ENU座標の水平)。
    /// 画面上の向きは、シェーダーがアンカーとアンカーから進行方向へ少し進んだ点を射影して求める
    /// (3Dでカメラを回しても、2Dの地図でも、シンボルの向きが実際の進行方向を指す)。
    pub(crate) fn oriented_billboard(
        anchor: [f32; 3],
        offset_px: [f32; 2],
        heading_rad: f32,
        color: [f32; 4],
    ) -> Self {
        Self {
            position: anchor,
            color,
            aux: [offset_px[0], offset_px[1], 0.0],
            params: [heading_rad, 0.0, 2.0, 0.0],
        }
    }
}

/// 座標の種類ごとの頂点列。アルファがこの値以上の色は不透明として`opaque`に入れる。
const OPAQUE_ALPHA: f32 = 0.999;

#[derive(Default)]
pub struct Batch {
    /// 不透明(深度を書いて描く)。
    pub opaque: Vec<DrawVertex>,
    /// 半透明(深度は書かずに、アルファブレンドで描く)。
    pub blend: Vec<DrawVertex>,
}

#[derive(Default)]
pub struct DrawingBatches {
    pub world: Batch,
    pub view: Batch,
    /// 画面座標の頂点列。追加順に重ねて描く(深度なし)。
    pub screen: Vec<DrawVertex>,
}

/// ジオメトリ生成に必要な、地形・座標変換・画面サイズ。
pub struct BuildContext<'a> {
    /// 現在GPUにある地形メッシュの原点のENU変換(`World`の出力座標)。
    pub mesh_transform: &'a EnuTransform,
    pub ellipsoid: &'a Ellipsoid,
    /// (緯度, 経度)の地表標高(メートル)。地形データの範囲外・海は0を返すこと。
    pub ground: &'a dyn Fn(f64, f64) -> f64,
    /// 描画先(canvas)の大きさ(px)。`Screen`の角の位置を決める。
    pub viewport_px: (f32, f32),
}

/// 一覧の図形から描画用の頂点列を作る。不可視・不正な図形(`Shape::validate`)は飛ばす。
pub fn build(ctx: &BuildContext, drawings: &[Drawing]) -> DrawingBatches {
    let mut out = DrawingBatches::default();
    for drawing in drawings.iter().filter(|d| d.visible) {
        let space = match drawing.shape.validate() {
            Ok(space) => space,
            Err(reason) => {
                log::warn!("[drawing] 図形{}を描かない: {reason}", drawing.id);
                continue;
            }
        };
        let mut sink = match space {
            Space::World => Sink::Split(&mut out.world),
            Space::View => Sink::Split(&mut out.view),
            Space::Screen => Sink::Single(&mut out.screen),
        };
        build_one(ctx, &drawing.shape, &drawing.style, &mut sink);
    }
    out
}

/// 頂点の出力先。色のアルファに応じて不透明/半透明の列を選ぶ(`Screen`は1列)。
enum Sink<'a> {
    Split(&'a mut Batch),
    Single(&'a mut Vec<DrawVertex>),
}

impl Sink<'_> {
    fn list(&mut self, alpha: f32) -> &mut Vec<DrawVertex> {
        match self {
            Sink::Split(batch) => {
                if alpha >= OPAQUE_ALPHA {
                    &mut batch.opaque
                } else {
                    &mut batch.blend
                }
            }
            Sink::Single(list) => list,
        }
    }
}

fn build_one(ctx: &BuildContext, shape: &Shape, style: &Style, sink: &mut Sink) {
    match shape {
        Shape::Circle { center, radius } => {
            let (frame, steps) = Frame2d::at(ctx, center);
            emit_geom2d(sink, ctx, &frame, &sector_geom(*radius, 0.0, 360.0, steps), style);
        }
        Shape::Sector { center, radius, start_deg, end_deg } => {
            let (frame, steps) = Frame2d::at(ctx, center);
            emit_geom2d(sink, ctx, &frame, &sector_geom(*radius, *start_deg, *end_deg, steps), style);
        }
        Shape::Rect { center, width, height, rotation_deg } => {
            let (frame, steps) = Frame2d::at(ctx, center);
            emit_geom2d(sink, ctx, &frame, &rect_geom(*width, *height, *rotation_deg, steps), style);
        }
        Shape::Polygon { points } => {
            let (frame, steps) = Frame2d::at(ctx, &points[0]);
            let local: Vec<[f64; 2]> = points.iter().map(|p| frame.local_of(ctx, p)).collect();
            emit_geom2d(sink, ctx, &frame, &polygon_geom(&local, steps), style);
        }
        Shape::Sphere { center, radius } => {
            emit_solid(sink, ctx, center, sphere_solid(*radius), style);
        }
        Shape::Cuboid { base_center, size_m, heading_deg } => {
            emit_solid(sink, ctx, base_center, cuboid_solid(*size_m, *heading_deg), style);
        }
        Shape::Cylinder { base_center, radius, height } => {
            emit_solid(sink, ctx, base_center, cylinder_solid(*radius, *height), style);
        }
        Shape::Cone { base_center, radius, height } => {
            emit_solid(sink, ctx, base_center, cone_solid(*radius, *height), style);
        }
        Shape::Polyline { points } => emit_polyline(sink, ctx, points, style),
    }
}

// ---------------------------------------------------------------------------------------------
// 頂点の出力(面・太い線)
// ---------------------------------------------------------------------------------------------

fn push_triangle(
    sink: &mut Sink,
    color: [f32; 4],
    positions: [[f32; 3]; 3],
    normals: Option<[[f32; 3]; 3]>,
) {
    let list = sink.list(color[3]);
    for i in 0..3 {
        list.push(DrawVertex::surface(positions[i], color, normals.map(|n| n[i])));
    }
}

/// 折れ線(閉じるなら最後の点から最初の点へも)を、太さ`width_px`の帯(線分1本につき三角形2枚)にして出力する。
/// 帯の幅への展開は頂点シェーダーが行う。各線分の頂点は「この端点」と「反対側の端点」を持ち、
/// 端点側の頂点は`side`の符号で左右に振り分ける(端の頂点で符号が逆になるのは、線の向きを
/// 「反対側→この端点」で取るため。物理的に同じ側に揃う)。
fn push_line_strip(sink: &mut Sink, points: &[[f32; 3]], closed: bool, color: [f32; 4], width_px: f32) {
    append_line_strip(sink.list(color[3]), points, closed, color, width_px);
}

/// `push_line_strip`の本体(出力先の頂点列を直接指定する)。観測点の覆域の輪郭線(`terrain::markers`)も使う。
pub(crate) fn append_line_strip(
    list: &mut Vec<DrawVertex>,
    points: &[[f32; 3]],
    closed: bool,
    color: [f32; 4],
    width_px: f32,
) {
    let n = points.len();
    if n < 2 {
        return;
    }
    let segments = if closed { n } else { n - 1 };
    for i in 0..segments {
        let (a, b) = (points[i], points[(i + 1) % n]);
        if a == b {
            continue;
        }
        let a_plus = DrawVertex::line(a, b, color, width_px, 1.0);
        let a_minus = DrawVertex::line(a, b, color, width_px, -1.0);
        let b_plus = DrawVertex::line(b, a, color, width_px, -1.0);
        let b_minus = DrawVertex::line(b, a, color, width_px, 1.0);
        list.extend_from_slice(&[a_plus, a_minus, b_plus, b_plus, a_minus, b_minus]);
    }
}

// ---------------------------------------------------------------------------------------------
// 緯度経度と、基準点からの方位・距離(方位角等距離図法。球面)
// ---------------------------------------------------------------------------------------------

/// 緯度`lat_deg`での平均曲率半径(子午線と卯酉線の曲率半径の幾何平均、メートル)。
pub(crate) fn mean_radius(ellipsoid: &Ellipsoid, lat_deg: f64) -> f64 {
    let f = 1.0 / ellipsoid.inv_f;
    let e2 = f * (2.0 - f);
    let s = lat_deg.to_radians().sin();
    ellipsoid.a_m * (1.0 - e2).sqrt() / (1.0 - e2 * s * s)
}

/// 基準点から方位`bearing_rad`(北から時計回り)へ距離`dist_m`進んだ点の(緯度, 経度)(度)。
pub(crate) fn destination(lat_deg: f64, lon_deg: f64, bearing_rad: f64, dist_m: f64, radius_m: f64) -> (f64, f64) {
    let (lat1, lon1) = (lat_deg.to_radians(), lon_deg.to_radians());
    let delta = dist_m / radius_m;
    let lat2 = (lat1.sin() * delta.cos() + lat1.cos() * delta.sin() * bearing_rad.cos()).asin();
    let lon2 = lon1
        + (bearing_rad.sin() * delta.sin() * lat1.cos()).atan2(delta.cos() - lat1.sin() * lat2.sin());
    (lat2.to_degrees(), lon2.to_degrees())
}

/// `destination`の逆: 基準点から見た(緯度, 経度)の位置を、ローカル座標[東, 北](メートル)で返す。
pub(crate) fn to_local(ref_lat_deg: f64, ref_lon_deg: f64, lat_deg: f64, lon_deg: f64, radius_m: f64) -> [f64; 2] {
    let (lat1, lat2) = (ref_lat_deg.to_radians(), lat_deg.to_radians());
    let mut dlon = (lon_deg - ref_lon_deg).to_radians();
    dlon = (dlon + PI).rem_euclid(TAU) - PI;
    let a = ((lat2 - lat1) * 0.5).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon * 0.5).sin().powi(2);
    let dist = 2.0 * radius_m * a.sqrt().min(1.0).asin();
    let bearing = (dlon.sin() * lat2.cos()).atan2(lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos());
    [dist * bearing.sin(), dist * bearing.cos()]
}

/// 地表に貼り付ける図形・線を、地表よりわずかに持ち上げる高さ(メートル)。地形メッシュ(粗いLODを含む)と
/// 同じ深度になって縞模様(Zファイティング)になるのを避ける(マーカー・覆域と同じ考え方)。
const GROUND_BIAS_M: f64 = 15.0;

/// 高度を楕円体高(メートル)にする。`AboveGround`は地表より`GROUND_BIAS_M`だけ持ち上げる。
fn height_of(ctx: &BuildContext, lat_deg: f64, lon_deg: f64, altitude: Altitude) -> f64 {
    match altitude {
        Altitude::Msl(h) => h,
        Altitude::AboveGround(offset) => (ctx.ground)(lat_deg, lon_deg) + offset + GROUND_BIAS_M,
    }
}

fn resolve_screen(ctx: &BuildContext, corner: Corner, x_px: f64, y_px: f64) -> [f64; 2] {
    let (w, h) = (ctx.viewport_px.0 as f64, ctx.viewport_px.1 as f64);
    let (ox, oy) = match corner {
        Corner::TopLeft => (0.0, 0.0),
        Corner::TopRight => (w, 0.0),
        Corner::BottomLeft => (0.0, h),
        Corner::BottomRight => (w, h),
        Corner::Center => (w * 0.5, h * 0.5),
    };
    [ox + x_px, oy + y_px]
}

// ---------------------------------------------------------------------------------------------
// 2D図形: ローカル座標の三角形・輪郭線(Geom2d)と、出力座標への変換(Frame2d)
// ---------------------------------------------------------------------------------------------

/// 2D図形のローカル座標(x=右/東, y=上/北)での形。
#[derive(Default)]
struct Geom2d {
    /// 塗りの三角形(3点ずつ)。
    fill: Vec<[f64; 2]>,
    outlines: Vec<Outline>,
}

struct Outline {
    points: Vec<[f64; 2]>,
    closed: bool,
}

/// 2D図形を分割する細かさ(ローカル座標の単位)。地形・地球の丸みへ沿わせるための上限で、
/// `World`以外(平面のままでよい)は`INFINITY`。
#[derive(Clone, Copy)]
struct Steps {
    /// 塗りの三角形の辺の長さの上限。
    fill: f64,
    /// 輪郭線(円弧・直線の辺)の分割の長さの上限。
    arc: f64,
}

/// 海抜の水平な面: 地球の丸みで面が地表の弦になる誤差(20kmで約8m)に収まる粗さで十分。
const STEPS_FLAT: Steps = Steps { fill: 20_000.0, arc: 2_000.0 };
/// 地表に貼り付ける面・線: 地形の起伏に沿うよう細かく分割する。
const STEPS_GROUND: Steps = Steps { fill: 500.0, arc: 250.0 };
const STEPS_NONE: Steps = Steps { fill: f64::INFINITY, arc: f64::INFINITY };

/// 1つの図形の塗りの三角形数の目安の上限。大きな地表貼り付け図形で頂点数が増え過ぎないよう、
/// 面積から分割の細かさの下限を決める。
const MAX_FILL_TRIANGLES: f64 = 50_000.0;
const MIN_CIRCLE_SEGMENTS: usize = 48;
const MAX_CIRCLE_SEGMENTS: usize = 720;
/// 塗りの分割数・辺の分割数の上限(異常な入力で頂点数が爆発しないための保険)。
const MAX_GRID_CELLS: usize = 512;
const MAX_EDGE_PARTS: usize = 2000;
/// 多角形の塗りの三角形を細分化した後の、頂点数の上限。
const MAX_REFINED_VERTICES: usize = 300_000;

fn effective_fill_step(area: f64, fill: f64) -> f64 {
    fill.max((2.0 * area / MAX_FILL_TRIANGLES).sqrt())
}

/// 時計回りに`deg`度回す(x=右, y=上の座標で)。
fn rotate_cw(p: [f64; 2], deg: f64) -> [f64; 2] {
    let (s, c) = deg.to_radians().sin_cos();
    [p[0] * c + p[1] * s, -p[0] * s + p[1] * c]
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// 辺を長さ`max_len`以下に等分割した点列。閉じるなら最後の辺(最後→最初)も分割し、最初の点は重ねない。
fn densify(points: &[[f64; 2]], closed: bool, max_len: f64) -> Vec<[f64; 2]> {
    if !max_len.is_finite() || points.len() < 2 {
        return points.to_vec();
    }
    let n = points.len();
    let segments = if closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(n);
    for i in 0..segments {
        let (a, b) = (points[i], points[(i + 1) % n]);
        let parts = ((dist(a, b) / max_len).ceil() as usize).clamp(1, MAX_EDGE_PARTS);
        for k in 0..parts {
            let t = k as f64 / parts as f64;
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    if !closed {
        out.push(points[n - 1]);
    }
    out
}

/// 円(`start_deg`〜`end_deg`が360度以上)または扇形。半径方向にリングで分割した三角形。
/// 方位は時計回り(0度=上)。扇形の輪郭は中心→弧→中心。
fn sector_geom(radius: f64, start_deg: f64, end_deg: f64, steps: Steps) -> Geom2d {
    let mut geom = Geom2d::default();
    if !(radius > 0.0) {
        return geom;
    }
    let raw = end_deg - start_deg;
    let full = raw >= 360.0 - 1e-9;
    let sweep = if full { 360.0 } else { raw.rem_euclid(360.0) };
    if sweep < 1e-9 {
        return geom;
    }
    let arc = steps.arc.min(radius * 0.09);
    let full_segments = ((TAU * radius / arc).ceil() as usize).clamp(MIN_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS);
    let n = ((full_segments as f64 * sweep / 360.0).ceil() as usize).max(1);
    let area = 0.5 * radius * radius * sweep.to_radians();
    let rings = ((radius / effective_fill_step(area, steps.fill)).ceil() as usize).clamp(1, MAX_GRID_CELLS);

    let at = |r: f64, i: usize| {
        let theta = (start_deg + sweep * i as f64 / n as f64).to_radians();
        [r * theta.sin(), r * theta.cos()]
    };
    for k in 1..=rings {
        let (r0, r1) = (radius * (k - 1) as f64 / rings as f64, radius * k as f64 / rings as f64);
        for i in 0..n {
            if k == 1 {
                geom.fill.extend([[0.0, 0.0], at(r1, i), at(r1, i + 1)]);
            } else {
                geom.fill.extend([at(r0, i), at(r1, i), at(r1, i + 1), at(r0, i), at(r1, i + 1), at(r0, i + 1)]);
            }
        }
    }
    let mut outline: Vec<[f64; 2]> = Vec::new();
    if full {
        outline.extend((0..n).map(|i| at(radius, i)));
    } else {
        outline.push([0.0, 0.0]);
        outline.extend((0..=n).map(|i| at(radius, i)));
    }
    geom.outlines.push(Outline { points: outline, closed: true });
    geom
}

fn rect_geom(width: f64, height: f64, rotation_deg: f64, steps: Steps) -> Geom2d {
    let mut geom = Geom2d::default();
    if !(width > 0.0 && height > 0.0) {
        return geom;
    }
    let s = effective_fill_step(width * height, steps.fill);
    let nx = ((width / s).ceil() as usize).clamp(1, MAX_GRID_CELLS);
    let ny = ((height / s).ceil() as usize).clamp(1, MAX_GRID_CELLS);
    let p = |ix: usize, iy: usize| {
        rotate_cw(
            [-width * 0.5 + width * ix as f64 / nx as f64, -height * 0.5 + height * iy as f64 / ny as f64],
            rotation_deg,
        )
    };
    for iy in 0..ny {
        for ix in 0..nx {
            let (a, b, c, d) = (p(ix, iy), p(ix + 1, iy), p(ix + 1, iy + 1), p(ix, iy + 1));
            geom.fill.extend([a, b, c, a, c, d]);
        }
    }
    let corners = [p(0, 0), p(nx, 0), p(nx, ny), p(0, ny)];
    geom.outlines.push(Outline { points: densify(&corners, true, steps.arc), closed: true });
    geom
}

/// 多角形: 三角形に分け(`triangulate`)、辺の長さが`steps.fill`を超える三角形は4分割で細分化する。
fn polygon_geom(points: &[[f64; 2]], steps: Steps) -> Geom2d {
    let mut geom = Geom2d::default();
    let mut poly: Vec<[f64; 2]> = points.to_vec();
    if poly.len() > 1 && poly.first() == poly.last() {
        poly.pop();
    }
    if poly.len() < 3 {
        return geom;
    }
    let area = signed_area(&poly);
    if area == 0.0 {
        return geom;
    }
    if area < 0.0 {
        poly.reverse(); // 反時計回りにそろえる。
    }
    let step = effective_fill_step(area.abs(), steps.fill);
    geom.fill = refine_triangles(triangulate(&poly), step);
    geom.outlines.push(Outline { points: densify(&poly, true, steps.arc), closed: true });
    geom
}

pub(crate) fn signed_area(poly: &[[f64; 2]]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| cross2(poly[i], poly[(i + 1) % n])).sum::<f64>() * 0.5
}

fn cross2(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// oから見てa→bが左回りなら正。
fn orient(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// 単純多角形(向きは問わない)を三角形に分ける。戻り値は3点ずつ並べた三角形で、どれも反時計回り。
/// 分割は`earcutr`(Mapboxのearcutの移植)に任せる。凹多角形や共線の頂点を含む入力も扱える。
/// 三角形を作れない入力(頂点が3つ未満など)は空を返す。
pub(crate) fn triangulate(poly: &[[f64; 2]]) -> Vec<[f64; 2]> {
    if poly.len() < 3 {
        return Vec::new();
    }
    let flat: Vec<f64> = poly.iter().flatten().copied().collect();
    let Ok(indices) = earcutr::earcut(&flat, &[], 2) else {
        return Vec::new();
    };
    indices
        .chunks_exact(3)
        .flat_map(|t| {
            let (a, b, c) = (poly[t[0]], poly[t[1]], poly[t[2]]);
            // earcutの出力の向きは入力に依らないので、反時計回りにそろえる。
            if orient(a, b, c) < 0.0 { [a, c, b] } else { [a, b, c] }
        })
        .collect()
}

/// 最長辺が`max_edge`を超える三角形を、辺の中点で4つに分けることを繰り返す。
fn refine_triangles(tris: Vec<[f64; 2]>, max_edge: f64) -> Vec<[f64; 2]> {
    if !max_edge.is_finite() {
        return tris;
    }
    let mut stack: Vec<[[f64; 2]; 3]> = tris.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
    let mut out: Vec<[f64; 2]> = Vec::new();
    let mid = |a: [f64; 2], b: [f64; 2]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    while let Some([a, b, c]) = stack.pop() {
        let longest = dist(a, b).max(dist(b, c)).max(dist(c, a));
        if longest <= max_edge || out.len() + stack.len() * 3 >= MAX_REFINED_VERTICES {
            out.extend([a, b, c]);
        } else {
            let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
            stack.extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
        }
    }
    out
}

/// 2D図形の置き場所(基準点)と、ローカル座標→出力座標の変換。
enum Frame2d {
    World { lat_deg: f64, lon_deg: f64, radius_m: f64, altitude: Altitude },
    View { right: f64, up: f64, forward: f64 },
    Screen { x: f64, y: f64 },
}

impl Frame2d {
    /// 基準点`pos`の変換と、その種類に合った分割の細かさ。
    fn at(ctx: &BuildContext, pos: &Position) -> (Self, Steps) {
        match *pos {
            Position::World { lat_deg, lon_deg, altitude } => {
                let frame = Self::World {
                    lat_deg,
                    lon_deg,
                    radius_m: mean_radius(ctx.ellipsoid, lat_deg),
                    altitude,
                };
                let steps = match altitude {
                    Altitude::Msl(_) => STEPS_FLAT,
                    Altitude::AboveGround(_) => STEPS_GROUND,
                };
                (frame, steps)
            }
            Position::View { right_m, up_m, forward_m } => {
                (Self::View { right: right_m, up: up_m, forward: forward_m }, STEPS_NONE)
            }
            Position::Screen { corner, x_px, y_px } => {
                let [x, y] = resolve_screen(ctx, corner, x_px, y_px);
                (Self::Screen { x, y }, STEPS_NONE)
            }
        }
    }

    /// 位置`pos`(この基準点と同じ種類)の、基準点から見たローカル座標。
    fn local_of(&self, ctx: &BuildContext, pos: &Position) -> [f64; 2] {
        match (self, *pos) {
            (Self::World { lat_deg, lon_deg, radius_m, .. }, Position::World { lat_deg: lat, lon_deg: lon, .. }) => {
                to_local(*lat_deg, *lon_deg, lat, lon, *radius_m)
            }
            (Self::View { right, up, .. }, Position::View { right_m, up_m, .. }) => {
                [right_m - right, up_m - up]
            }
            (Self::Screen { x, y }, Position::Screen { corner, x_px, y_px }) => {
                let [px, py] = resolve_screen(ctx, corner, x_px, y_px);
                [px - x, y - py]
            }
            _ => [0.0, 0.0], // validateで種類がそろっているので来ない。
        }
    }

    fn map(&self, ctx: &BuildContext, p: [f64; 2]) -> [f32; 3] {
        match self {
            Self::World { lat_deg, lon_deg, radius_m, altitude } => {
                let (lat, lon) = destination(*lat_deg, *lon_deg, p[0].atan2(p[1]), p[0].hypot(p[1]), *radius_m);
                ctx.mesh_transform.transform(lat, lon, height_of(ctx, lat, lon, *altitude))
            }
            Self::View { right, up, forward } => [(right + p[0]) as f32, (up + p[1]) as f32, -*forward as f32],
            Self::Screen { x, y } => [(x + p[0]) as f32, (y - p[1]) as f32, 0.0],
        }
    }
}

fn emit_geom2d(sink: &mut Sink, ctx: &BuildContext, frame: &Frame2d, geom: &Geom2d, style: &Style) {
    if let Some(fill) = style.fill {
        let color = fill.to_array();
        for tri in geom.fill.chunks_exact(3) {
            let positions = [frame.map(ctx, tri[0]), frame.map(ctx, tri[1]), frame.map(ctx, tri[2])];
            push_triangle(sink, color, positions, None);
        }
    }
    if let Some(stroke) = style.stroke.filter(|_| style.stroke_width_px > 0.0) {
        for outline in &geom.outlines {
            let points: Vec<[f32; 3]> = outline.points.iter().map(|&p| frame.map(ctx, p)).collect();
            push_line_strip(sink, &points, outline.closed, stroke.to_array(), style.stroke_width_px);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// 折れ線
// ---------------------------------------------------------------------------------------------

/// `World`の折れ線の点の間を分割する長さの上限(メートル)。地表に貼り付ける点があれば細かく。
const POLYLINE_STEP_FLAT_M: f64 = 2_000.0;
const POLYLINE_STEP_GROUND_M: f64 = 250.0;

fn emit_polyline(sink: &mut Sink, ctx: &BuildContext, points: &[Position], style: &Style) {
    let Some(stroke) = style.stroke.filter(|_| style.stroke_width_px > 0.0) else {
        return;
    };
    let mapped: Vec<[f32; 3]> = match points[0] {
        Position::World { .. } => world_polyline(ctx, points),
        Position::View { .. } => points
            .iter()
            .filter_map(|p| match *p {
                Position::View { right_m, up_m, forward_m } => {
                    Some([right_m as f32, up_m as f32, -forward_m as f32])
                }
                _ => None,
            })
            .collect(),
        Position::Screen { .. } => points
            .iter()
            .filter_map(|p| match *p {
                Position::Screen { corner, x_px, y_px } => {
                    let [x, y] = resolve_screen(ctx, corner, x_px, y_px);
                    Some([x as f32, y as f32, 0.0])
                }
                _ => None,
            })
            .collect(),
    };
    push_line_strip(sink, &mapped, false, stroke.to_array(), style.stroke_width_px);
}

/// `World`の折れ線: 点の間を大円(球面)に沿って分割し、各点をENUへ変換する。高度は点の間で補間する
/// (どちらも海抜なら海抜を直線補間、片方でも地表基準なら「地表からの高さ」を直線補間して地形に沿わせる)。
fn world_polyline(ctx: &BuildContext, points: &[Position]) -> Vec<[f32; 3]> {
    let geodetic: Vec<(f64, f64, Altitude)> = points
        .iter()
        .filter_map(|p| match *p {
            Position::World { lat_deg, lon_deg, altitude } => Some((lat_deg, lon_deg, altitude)),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    for pair in geodetic.windows(2) {
        let ((lat0, lon0, alt0), (lat1, lon1, alt1)) = (pair[0], pair[1]);
        let radius = mean_radius(ctx.ellipsoid, lat0);
        let [east, north] = to_local(lat0, lon0, lat1, lon1, radius);
        let (length, bearing) = (east.hypot(north), east.atan2(north));
        let grounded = matches!(alt0, Altitude::AboveGround(_)) || matches!(alt1, Altitude::AboveGround(_));
        let step = if grounded { POLYLINE_STEP_GROUND_M } else { POLYLINE_STEP_FLAT_M };
        let parts = ((length / step).ceil() as usize).clamp(1, MAX_EDGE_PARTS * 2);
        // 地表基準で補間するときの、両端の「地表からの高さ」(海抜の端点は、その地点の地表との差)。
        let offset = |lat: f64, lon: f64, alt: Altitude| match alt {
            Altitude::Msl(h) => h - (ctx.ground)(lat, lon),
            Altitude::AboveGround(o) => o,
        };
        let (off0, off1) = (offset(lat0, lon0, alt0), offset(lat1, lon1, alt1));
        let (h0, h1) = (height_of(ctx, lat0, lon0, alt0), height_of(ctx, lat1, lon1, alt1));
        for k in 0..parts {
            let t = k as f64 / parts as f64;
            let (lat, lon) = destination(lat0, lon0, bearing, length * t, radius);
            let h = if grounded {
                (ctx.ground)(lat, lon) + off0 + (off1 - off0) * t + GROUND_BIAS_M
            } else {
                h0 + (h1 - h0) * t
            };
            out.push(ctx.mesh_transform.transform(lat, lon, h));
        }
    }
    if let Some(&(lat, lon, alt)) = geodetic.last() {
        out.push(ctx.mesh_transform.transform(lat, lon, height_of(ctx, lat, lon, alt)));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// 3D図形: ローカル座標(x=東/右, y=北/前方, z=上)の立体(Solid)と、出力座標への変換
// ---------------------------------------------------------------------------------------------

const SPHERE_SEGMENTS: usize = 48;
const SPHERE_RINGS: usize = 24;
const SOLID_SEGMENTS: usize = 48;

/// 立体のローカル座標での形。頂点は位置と単位法線。
#[derive(Default)]
struct Solid {
    verts: Vec<([f64; 3], [f64; 3])>,
    indices: Vec<u32>,
    /// 稜線(ワイヤーフレーム)。(点列, 閉じるか)。
    lines: Vec<(Vec<[f64; 3]>, bool)>,
}

impl Solid {
    fn push_quad(&mut self, corners: [[f64; 3]; 4], normal: [f64; 3]) {
        let base = self.verts.len() as u32;
        self.verts.extend(corners.map(|c| (c, normal)));
        self.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// 水平面(z一定)上の円周(閉じた点列)。
    fn ring(radius: f64, z: f64) -> Vec<[f64; 3]> {
        (0..SOLID_SEGMENTS)
            .map(|i| {
                let t = TAU * i as f64 / SOLID_SEGMENTS as f64;
                [radius * t.cos(), radius * t.sin(), z]
            })
            .collect()
    }

    /// 鉛直軸まわりに時計回りに`deg`度回す。
    fn rotate_heading(&mut self, deg: f64) {
        let turn = |v: [f64; 3]| {
            let [x, y] = rotate_cw([v[0], v[1]], deg);
            [x, y, v[2]]
        };
        for (p, n) in &mut self.verts {
            *p = turn(*p);
            *n = turn(*n);
        }
        for (line, _) in &mut self.lines {
            for p in line.iter_mut() {
                *p = turn(*p);
            }
        }
    }
}

fn sphere_solid(radius: f64) -> Solid {
    let mut solid = Solid::default();
    if !(radius > 0.0) {
        return solid;
    }
    for i in 0..=SPHERE_RINGS {
        let phi = PI * i as f64 / SPHERE_RINGS as f64;
        for j in 0..=SPHERE_SEGMENTS {
            let theta = TAU * j as f64 / SPHERE_SEGMENTS as f64;
            let n = [phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos()];
            solid.verts.push(([radius * n[0], radius * n[1], radius * n[2]], n));
        }
    }
    for i in 0..SPHERE_RINGS {
        for j in 0..SPHERE_SEGMENTS {
            let a = (i * (SPHERE_SEGMENTS + 1) + j) as u32;
            let b = a + SPHERE_SEGMENTS as u32 + 1;
            solid.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    // 稜線の代わりに、互いに直交する3つの大円(赤道と2つの子午線)。
    let equator = Solid::ring(radius, 0.0);
    solid.lines.push((equator.iter().map(|p| [p[0], p[2], p[1]]).collect(), true));
    solid.lines.push((equator.iter().map(|p| [p[2], p[0], p[1]]).collect(), true));
    solid.lines.push((equator, true));
    solid
}

/// 底面の中心が原点、+zが上。`size`=[x方向(東西), y方向(南北), 高さ]。
fn cuboid_solid(size: [f64; 3], heading_deg: f64) -> Solid {
    let mut solid = Solid::default();
    if !(size.iter().all(|&s| s > 0.0)) {
        return solid;
    }
    let (hx, hy, h) = (size[0] * 0.5, size[1] * 0.5, size[2]);
    solid.push_quad([[hx, -hy, 0.0], [hx, hy, 0.0], [hx, hy, h], [hx, -hy, h]], [1.0, 0.0, 0.0]);
    solid.push_quad([[-hx, hy, 0.0], [-hx, -hy, 0.0], [-hx, -hy, h], [-hx, hy, h]], [-1.0, 0.0, 0.0]);
    solid.push_quad([[hx, hy, 0.0], [-hx, hy, 0.0], [-hx, hy, h], [hx, hy, h]], [0.0, 1.0, 0.0]);
    solid.push_quad([[-hx, -hy, 0.0], [hx, -hy, 0.0], [hx, -hy, h], [-hx, -hy, h]], [0.0, -1.0, 0.0]);
    solid.push_quad([[-hx, -hy, h], [hx, -hy, h], [hx, hy, h], [-hx, hy, h]], [0.0, 0.0, 1.0]);
    solid.push_quad([[-hx, hy, 0.0], [hx, hy, 0.0], [hx, -hy, 0.0], [-hx, -hy, 0.0]], [0.0, 0.0, -1.0]);
    let bottom = vec![[-hx, -hy, 0.0], [hx, -hy, 0.0], [hx, hy, 0.0], [-hx, hy, 0.0]];
    let top: Vec<[f64; 3]> = bottom.iter().map(|p| [p[0], p[1], h]).collect();
    for (b, t) in bottom.iter().zip(&top) {
        solid.lines.push((vec![*b, *t], false));
    }
    solid.lines.push((bottom, true));
    solid.lines.push((top, true));
    solid.rotate_heading(heading_deg);
    solid
}

/// 底面の中心が原点、+zが上。
fn cylinder_solid(radius: f64, height: f64) -> Solid {
    let mut solid = Solid::default();
    if !(radius > 0.0 && height > 0.0) {
        return solid;
    }
    for j in 0..=SOLID_SEGMENTS {
        let t = TAU * j as f64 / SOLID_SEGMENTS as f64;
        let (c, s) = (t.cos(), t.sin());
        solid.verts.push(([radius * c, radius * s, 0.0], [c, s, 0.0]));
        solid.verts.push(([radius * c, radius * s, height], [c, s, 0.0]));
    }
    for j in 0..SOLID_SEGMENTS as u32 {
        let (bottom, top, next_bottom, next_top) = (2 * j, 2 * j + 1, 2 * j + 2, 2 * j + 3);
        solid.indices.extend([bottom, next_bottom, top, top, next_bottom, next_top]);
    }
    for (z, normal) in [(height, [0.0, 0.0, 1.0]), (0.0, [0.0, 0.0, -1.0])] {
        let center = solid.verts.len() as u32;
        solid.verts.push(([0.0, 0.0, z], normal));
        for p in Solid::ring(radius, z) {
            solid.verts.push((p, normal));
        }
        for j in 0..SOLID_SEGMENTS as u32 {
            solid.indices.extend([center, center + 1 + j, center + 1 + (j + 1) % SOLID_SEGMENTS as u32]);
        }
    }
    for k in 0..4 {
        let t = TAU * k as f64 / 4.0;
        solid.lines.push((vec![[radius * t.cos(), radius * t.sin(), 0.0], [radius * t.cos(), radius * t.sin(), height]], false));
    }
    solid.lines.push((Solid::ring(radius, 0.0), true));
    solid.lines.push((Solid::ring(radius, height), true));
    solid
}

/// 底面の中心が原点、頂点が(0,0,height)。
fn cone_solid(radius: f64, height: f64) -> Solid {
    let mut solid = Solid::default();
    if !(radius > 0.0 && height > 0.0) {
        return solid;
    }
    let slant = radius.hypot(height);
    // 側面の法線(外向き): 底面の円の接線方向に垂直で、斜面に沿った向き。
    let side_normal = |t: f64| [height * t.cos() / slant, height * t.sin() / slant, radius / slant];
    let segments = SOLID_SEGMENTS as u32;
    for j in 0..=segments {
        let t = TAU * j as f64 / SOLID_SEGMENTS as f64;
        solid.verts.push(([radius * t.cos(), radius * t.sin(), 0.0], side_normal(t)));
    }
    for j in 0..segments {
        let mid = TAU * (j as f64 + 0.5) / SOLID_SEGMENTS as f64;
        let apex = solid.verts.len() as u32;
        solid.verts.push(([0.0, 0.0, height], side_normal(mid)));
        solid.indices.extend([j, j + 1, apex]);
    }
    let center = solid.verts.len() as u32;
    solid.verts.push(([0.0, 0.0, 0.0], [0.0, 0.0, -1.0]));
    for p in Solid::ring(radius, 0.0) {
        solid.verts.push((p, [0.0, 0.0, -1.0]));
    }
    for j in 0..segments {
        solid.indices.extend([center, center + 1 + j, center + 1 + (j + 1) % segments]);
    }
    for k in 0..4 {
        let t = TAU * k as f64 / 4.0;
        solid.lines.push((vec![[radius * t.cos(), radius * t.sin(), 0.0], [0.0, 0.0, height]], false));
    }
    solid.lines.push((Solid::ring(radius, 0.0), true));
    solid
}

/// 3D図形の置き場所と、ローカル座標(位置・法線)→出力座標の変換。
enum Frame3d {
    /// 位置における局所ENU(この位置を通る鉛直線が+z)から測地座標を経てメッシュ原点のENUへ。
    /// 地球の丸みで遠方ほど局所の上向きが傾くのを、厳密に扱う。
    World { local: EnuTransform, base_height: f64 },
    View { right: f64, up: f64, forward: f64 },
}

impl Frame3d {
    fn at(ctx: &BuildContext, pos: &Position) -> Option<Self> {
        match *pos {
            Position::World { lat_deg, lon_deg, altitude } => {
                let base_height = match altitude {
                    Altitude::Msl(h) => h,
                    Altitude::AboveGround(offset) => (ctx.ground)(lat_deg, lon_deg) + offset,
                };
                let origin = Origin { lat_deg, lon_deg };
                Some(Self::World { local: EnuTransform::new(&origin, ctx.ellipsoid), base_height })
            }
            Position::View { right_m, up_m, forward_m } => {
                Some(Self::View { right: right_m, up: up_m, forward: forward_m })
            }
            Position::Screen { .. } => None,
        }
    }

    fn map_position(&self, ctx: &BuildContext, p: [f64; 3]) -> [f32; 3] {
        match self {
            Self::World { local, base_height } => {
                let (lat, lon, h) = local.enu_to_geodetic(p[0], p[1], base_height + p[2]);
                ctx.mesh_transform.transform(lat, lon, h)
            }
            // カメラの前方(+y)は視点空間では-z。
            Self::View { right, up, forward } => {
                [(right + p[0]) as f32, (up + p[2]) as f32, -(forward + p[1]) as f32]
            }
        }
    }

    fn map_normal(&self, n: [f64; 3]) -> [f32; 3] {
        match self {
            Self::World { .. } => [n[0] as f32, n[1] as f32, n[2] as f32],
            Self::View { .. } => [n[0] as f32, n[2] as f32, -n[1] as f32],
        }
    }
}

fn emit_solid(sink: &mut Sink, ctx: &BuildContext, pos: &Position, solid: Solid, style: &Style) {
    let Some(frame) = Frame3d::at(ctx, pos) else {
        return;
    };
    if let Some(fill) = style.fill {
        let color = fill.to_array();
        let mapped: Vec<([f32; 3], [f32; 3])> = solid
            .verts
            .iter()
            .map(|&(p, n)| (frame.map_position(ctx, p), frame.map_normal(n)))
            .collect();
        for tri in solid.indices.chunks_exact(3) {
            let v = [mapped[tri[0] as usize], mapped[tri[1] as usize], mapped[tri[2] as usize]];
            push_triangle(sink, color, [v[0].0, v[1].0, v[2].0], Some([v[0].1, v[1].1, v[2].1]));
        }
    }
    if let Some(stroke) = style.stroke.filter(|_| style.stroke_width_px > 0.0) {
        for (line, closed) in &solid.lines {
            let points: Vec<[f32; 3]> = line.iter().map(|&p| frame.map_position(ctx, p)).collect();
            push_line_strip(sink, &points, *closed, stroke.to_array(), style.stroke_width_px);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::drawing::Color;

    const ORIGIN: Origin = Origin { lat_deg: 35.355556, lon_deg: 138.859722 };

    fn with_ctx<R>(viewport: (f32, f32), f: impl FnOnce(&BuildContext) -> R) -> R {
        let transform = EnuTransform::new(&ORIGIN, &Ellipsoid::WGS84);
        // 地表は標高100mの平地とみなす。
        let ground = |_: f64, _: f64| 100.0;
        f(&BuildContext { mesh_transform: &transform, ellipsoid: &Ellipsoid::WGS84, ground: &ground, viewport_px: viewport })
    }

    fn drawing(id: u64, shape: Shape, style: Style) -> Drawing {
        Drawing { id, shape, style, visible: true }
    }

    fn triangle_area(t: &[[f64; 2]]) -> f64 {
        orient(t[0], t[1], t[2]).abs() * 0.5
    }

    fn total_area(tris: &[[f64; 2]]) -> f64 {
        tris.chunks_exact(3).map(triangle_area).sum()
    }

    #[test]
    fn local_coordinates_round_trip() {
        let r = mean_radius(&Ellipsoid::WGS84, ORIGIN.lat_deg);
        for &(dx, dy) in &[(1000.0, 0.0), (-25_000.0, 40_000.0), (0.0, -120_000.0), (300_000.0, 200_000.0)] {
            let (lat, lon) = destination(ORIGIN.lat_deg, ORIGIN.lon_deg, f64::atan2(dx, dy), f64::hypot(dx, dy), r);
            let [x, y] = to_local(ORIGIN.lat_deg, ORIGIN.lon_deg, lat, lon, r);
            assert!((x - dx).abs() < 1e-3 && (y - dy).abs() < 1e-3, "({dx},{dy}) -> ({x},{y})");
        }
    }

    // 東へ10km進んだ点は、ENU(接平面)の東10kmから丸みで少し下がった位置になる。
    #[test]
    fn destination_matches_enu_scale() {
        let r = mean_radius(&Ellipsoid::WGS84, ORIGIN.lat_deg);
        let (lat, lon) = destination(ORIGIN.lat_deg, ORIGIN.lon_deg, PI / 2.0, 10_000.0, r);
        let t = EnuTransform::new(&ORIGIN, &Ellipsoid::WGS84);
        let [e, n, u] = t.transform_f64(lat, lon, 0.0);
        // 球(平均曲率半径)と楕円体の東西方向の曲率半径の差(約0.3%)による誤差が数十m出る。
        assert!((e - 10_000.0).abs() < 60.0, "e={e}");
        assert!(n.abs() < 60.0, "n={n}");
        assert!((u + 10_000.0f64.powi(2) / (2.0 * r)).abs() < 3.0, "u={u}");
    }

    #[test]
    fn circle_and_rect_fill_areas() {
        let circle = sector_geom(1000.0, 0.0, 360.0, STEPS_NONE);
        let expected = PI * 1000.0 * 1000.0;
        assert!((total_area(&circle.fill) - expected).abs() / expected < 0.01);
        let ground = sector_geom(1000.0, 0.0, 360.0, STEPS_GROUND);
        assert!(ground.fill.len() > circle.fill.len(), "地表貼り付けは細かく分割される");
        assert!((total_area(&ground.fill) - expected).abs() / expected < 0.01);

        let rect = rect_geom(300.0, 200.0, 30.0, Steps { fill: 50.0, arc: 50.0 });
        assert_eq!(rect.fill.len(), 6 * 4 * 6);
        assert!((total_area(&rect.fill) - 60_000.0).abs() < 1.0);
    }

    #[test]
    fn sector_covers_the_requested_angle() {
        // 北から東まで(0〜90度): 面積は円の1/4で、点は第1象限(東・北とも0以上)にある。
        let sector = sector_geom(1000.0, 0.0, 90.0, STEPS_NONE);
        let expected = PI * 1000.0 * 1000.0 / 4.0;
        assert!((total_area(&sector.fill) - expected).abs() / expected < 0.01);
        assert!(sector.fill.iter().all(|p| p[0] > -1e-6 && p[1] > -1e-6));
        // 終端が始端より小さければ、360度をまたいで時計回りに進む(270度→90度=180度分)。
        let wrap = sector_geom(1000.0, 270.0, 90.0, STEPS_NONE);
        assert!((total_area(&wrap.fill) - PI * 1000.0 * 1000.0 / 2.0).abs() / (PI * 1e6 / 2.0) < 0.01);
    }

    #[test]
    fn polygon_triangulation_keeps_concave_area() {
        // L字(凹多角形)。面積は 300*100 + 100*200。時計回りで与えても同じ。
        let l: Vec<[f64; 2]> =
            [[0.0, 0.0], [300.0, 0.0], [300.0, 100.0], [100.0, 100.0], [100.0, 300.0], [0.0, 300.0]].to_vec();
        let expected = 50_000.0;
        let fill = polygon_geom(&l, STEPS_NONE).fill;
        assert_eq!(fill.len(), 4 * 3);
        assert!((total_area(&fill) - expected).abs() < 1e-6);
        let cw: Vec<[f64; 2]> = l.iter().rev().copied().collect();
        assert!((total_area(&polygon_geom(&cw, STEPS_NONE).fill) - expected).abs() < 1e-6);
        // 細分化しても面積は変わらず、最長辺が上限以下になる。
        let fine = polygon_geom(&l, Steps { fill: 60.0, arc: 60.0 }).fill;
        assert!((total_area(&fine) - expected).abs() < 1e-6);
        let longest = fine
            .chunks_exact(3)
            .flat_map(|t| [dist(t[0], t[1]), dist(t[1], t[2]), dist(t[2], t[0])])
            .fold(0.0, f64::max);
        assert!(longest <= 60.0 + 1e-9, "longest={longest}");
    }

    #[test]
    fn opaque_and_translucent_go_to_separate_lists() {
        with_ctx((800.0, 600.0), |ctx| {
            let center = Position::world(ORIGIN.lat_deg, ORIGIN.lon_deg, Altitude::Msl(500.0));
            let translucent = drawing(1, Shape::Circle { center, radius: 2000.0 }, Style::filled(Color::rgba(1.0, 0.0, 0.0, 0.4)));
            let opaque = drawing(2, Shape::Circle { center, radius: 2000.0 }, Style::filled(Color::rgb(0.0, 1.0, 0.0)));
            let only_translucent = build(ctx, &[translucent.clone()]);
            assert!(only_translucent.world.opaque.is_empty() && !only_translucent.world.blend.is_empty());
            let only_opaque = build(ctx, &[opaque]);
            assert!(!only_opaque.world.opaque.is_empty() && only_opaque.world.blend.is_empty());
            assert!(only_opaque.view.opaque.is_empty() && only_opaque.screen.is_empty());
        });
    }

    #[test]
    fn invalid_or_hidden_shapes_are_skipped() {
        with_ctx((800.0, 600.0), |ctx| {
            let world = Position::world(ORIGIN.lat_deg, ORIGIN.lon_deg, Altitude::Msl(0.0));
            let screen = Position::screen(Corner::TopLeft, 10.0, 10.0);
            let mixed = drawing(1, Shape::Polygon { points: vec![world, screen, world] }, Style::default());
            let solid_on_screen = drawing(2, Shape::Sphere { center: screen, radius: 10.0 }, Style::default());
            let mut hidden = drawing(3, Shape::Circle { center: world, radius: 100.0 }, Style::default());
            hidden.visible = false;
            let batches = build(ctx, &[mixed, solid_on_screen, hidden]);
            assert!(batches.world.opaque.is_empty() && batches.world.blend.is_empty());
            assert!(batches.screen.is_empty());
        });
    }

    #[test]
    fn polyline_makes_two_triangles_per_segment() {
        with_ctx((800.0, 600.0), |ctx| {
            let stroke = Style::stroked(Color::rgb(1.0, 1.0, 0.0), 3.0);
            let points = vec![
                Position::screen(Corner::TopLeft, 0.0, 0.0),
                Position::screen(Corner::TopLeft, 100.0, 0.0),
                Position::screen(Corner::TopLeft, 100.0, 50.0),
            ];
            let batches = build(ctx, &[drawing(1, Shape::Polyline { points }, stroke)]);
            assert_eq!(batches.screen.len(), 2 * 6);
            assert!(batches.screen.iter().all(|v| v.params[0] == 3.0 && v.params[1].abs() == 1.0));
            // 端点は「この端点」と「反対側」で対になっている(片側の頂点の反対側は、もう片側の位置)。
            let a = &batches.screen[0];
            assert_eq!(a.position, [0.0, 0.0, 0.0]);
            assert_eq!(a.aux, [100.0, 0.0, 0.0]);
        });
    }

    #[test]
    fn screen_corner_and_offsets() {
        with_ctx((800.0, 600.0), |ctx| {
            // 右下の角から(-100,-50)内側へ寄せた位置に置いた、幅40・高さ20の矩形。
            let center = Position::screen(Corner::BottomRight, -100.0, -50.0);
            let rect = drawing(1, Shape::Rect { center, width: 40.0, height: 20.0, rotation_deg: 0.0 }, Style::filled(Color::rgb(1.0, 1.0, 1.0)));
            let screen = build(ctx, &[rect]).screen;
            let (min_x, max_x) = screen.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.position[0]), hi.max(v.position[0])));
            let (min_y, max_y) = screen.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.position[1]), hi.max(v.position[1])));
            assert_eq!((min_x, max_x, min_y, max_y), (680.0, 720.0, 540.0, 560.0));
        });
    }

    #[test]
    fn view_space_solid_faces_forward() {
        with_ctx((800.0, 600.0), |ctx| {
            // カメラの右に2m・上に1m・前方10mの球。視点空間では前方は-z。
            let center = Position::view(2.0, 1.0, 10.0);
            let sphere = drawing(1, Shape::Sphere { center, radius: 0.5 }, Style::filled(Color::rgb(1.0, 0.0, 0.0)));
            let view = build(ctx, &[sphere]).view.opaque;
            assert!(!view.is_empty());
            for v in &view {
                assert!((v.position[0] - 2.0).abs() <= 0.5 + 1e-4);
                assert!((v.position[1] - 1.0).abs() <= 0.5 + 1e-4);
                assert!((v.position[2] + 10.0).abs() <= 0.5 + 1e-4);
                let n = v.aux;
                assert!(((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() - 1.0).abs() < 1e-4);
                assert_eq!(v.params[3], 1.0);
            }
        });
    }

    #[test]
    fn solids_have_valid_indices_and_unit_normals() {
        let solids = [
            sphere_solid(10.0),
            cuboid_solid([10.0, 20.0, 30.0], 33.0),
            cylinder_solid(10.0, 20.0),
            cone_solid(10.0, 20.0),
        ];
        for solid in &solids {
            assert!(!solid.verts.is_empty() && !solid.indices.is_empty());
            assert!(solid.indices.iter().all(|&i| (i as usize) < solid.verts.len()));
            for (_, n) in &solid.verts {
                assert!(((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() - 1.0).abs() < 1e-9);
            }
            assert!(!solid.lines.is_empty());
        }
    }

    #[test]
    fn world_solid_stands_on_the_ground() {
        with_ctx((800.0, 600.0), |ctx| {
            // 標高100mの地表(ground)に、地表から50mの高さを底面とする高さ200mの円柱。
            let base = Position::world(ORIGIN.lat_deg, ORIGIN.lon_deg, Altitude::AboveGround(50.0));
            let cylinder = drawing(1, Shape::Cylinder { base_center: base, radius: 100.0, height: 200.0 }, Style::filled(Color::rgb(0.5, 0.5, 0.5)));
            let world = build(ctx, &[cylinder]).world.opaque;
            let (lo, hi) = world.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.position[2]), hi.max(v.position[2])));
            // 原点の真上なのでENUの上=楕円体高。底面150m〜上面350m(法線方向の丸みは100mの半径で0.8mほど)。
            assert!((lo - 150.0).abs() < 2.0 && (hi - 350.0).abs() < 2.0, "lo={lo} hi={hi}");
        });
    }

    #[test]
    fn ground_shapes_follow_ground_and_flat_shapes_do_not() {
        with_ctx((800.0, 600.0), |ctx| {
            let at = |altitude| Position::world(ORIGIN.lat_deg, ORIGIN.lon_deg, altitude);
            let style = Style::filled(Color::rgb(1.0, 1.0, 1.0));
            let heights = |altitude| {
                let d = drawing(1, Shape::Circle { center: at(altitude), radius: 1000.0 }, style);
                let mut z: Vec<f32> = build(ctx, &[d]).world.opaque.iter().map(|v| v.position[2]).collect();
                z.sort_by(f32::total_cmp);
                (z[0], *z.last().unwrap())
            };
            // 地表(標高100m)+0mの貼り付けは、標高+バイアス(15m)の高さ(1km先で丸みが0.08m下がる)。
            let (lo, hi) = heights(Altitude::AboveGround(0.0));
            assert!((lo - 115.0).abs() < 0.5 && (hi - 115.0).abs() < 0.5, "lo={lo} hi={hi}");
            // 海抜の水平面は、指定した楕円体高そのもの。
            let (lo, hi) = heights(Altitude::Msl(500.0));
            assert!((lo - 500.0).abs() < 0.5 && (hi - 500.0).abs() < 0.5, "lo={lo} hi={hi}");
        });
    }

    #[test]
    fn long_polyline_is_subdivided_along_the_ground() {
        with_ctx((800.0, 600.0), |ctx| {
            let a = Position::world(35.0, 138.0, Altitude::AboveGround(0.0));
            let b = Position::world(35.0, 139.0, Altitude::AboveGround(0.0));
            let points = world_polyline(ctx, &[a, b]);
            // 約91kmを250m以下に分割する。
            assert!(points.len() > 300, "len={}", points.len());
            // 端点はそれぞれの位置(標高100m+バイアス)。
            let first = points[0];
            let expected = ctx.mesh_transform.transform(35.0, 138.0, 115.0);
            assert!((first[0] - expected[0]).abs() < 0.1 && (first[2] - expected[2]).abs() < 0.1);
        });
    }
    /// 三角形(3点ずつ)の面積の和と、すべて反時計回りであること。
    fn triangle_areas(tris: &[[f64; 2]]) -> (f64, bool) {
        let mut sum = 0.0;
        let mut all_ccw = true;
        for t in tris.chunks_exact(3) {
            let a = orient(t[0], t[1], t[2]) * 0.5;
            sum += a;
            all_ccw &= a >= 0.0;
        }
        (sum, all_ccw)
    }

    #[test]
    fn triangulate_handles_both_orientations_and_concave_shapes() {
        // L字(凹)。反時計回りでも時計回りでも、面積の和は多角形の面積(3)に等しく、三角形は反時計回り。
        let l_shape = [[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]];
        let mut reversed = l_shape;
        reversed.reverse();
        for poly in [l_shape, reversed] {
            let tris = triangulate(&poly);
            assert_eq!(tris.len(), 3 * 4); // n-2 = 4三角形
            let (area, all_ccw) = triangle_areas(&tris);
            assert!((area - 3.0).abs() < 1e-9, "area={area}");
            assert!(all_ccw);
        }
    }

    #[test]
    fn triangulate_keeps_collinear_vertices_without_losing_area() {
        // 辺の途中に共線の頂点がある正方形。
        let poly = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
        let (area, all_ccw) = triangle_areas(&triangulate(&poly));
        assert!((area - 4.0).abs() < 1e-9, "area={area}");
        assert!(all_ccw);
    }

    #[test]
    fn triangulate_returns_nothing_for_degenerate_input() {
        assert!(triangulate(&[]).is_empty());
        assert!(triangulate(&[[0.0, 0.0], [1.0, 1.0]]).is_empty());
        // 面積0(一直線)の多角形に、面積のある三角形は作れない。
        let (area, _) = triangle_areas(&triangulate(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]]));
        assert!(area.abs() < 1e-12);
    }
}
