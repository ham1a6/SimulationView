//! 作図機能(`sim3dview::terrain::drawing`)のデモ。表示メニューの「作図デモ」で、原点のまわりに
//! 各種の図形・線を出し入れする(ライブラリの使い方の例)。
//!
//! - 絶対座標(`Position::World`): 地表に貼り付けた円・多角形、海抜の水平な扇形・矩形、3D図形4種、
//!   地形に沿う折れ線。カメラを動かすと地形と一緒に動き、山の陰に隠れる。
//! - カメラ固定(`Position::Screen`/`Position::View`): 画面の隅の枠・十字線・円(画面座標)と、
//!   カメラの手前に浮かぶ球・直方体(視点空間)。カメラを動かしても画面上の位置は変わらない。

use sim3dview::terrain::drawing::{
    Altitude, Color, Corner, DrawingId, DrawingState, Position, Shape, Style,
};

/// 原点(`lat`,`lon`)のまわりにデモの図形を追加して、追加した図形のIDを返す。既存の図形は消さない
/// (消すときはこのIDだけを`remove`する。`clear`だと、ユーザーが作った図形まで消えてしまう)。
pub fn add_demo(state: DrawingState, lat: f64, lon: f64) -> Vec<DrawingId> {
    let mut ids = Vec::new();
    let mut add = |shape: Shape, style: Style| ids.push(state.add(shape, style));
    // 原点から(dlat, dlon)度ずれた位置(0.1度は緯度で約11km、経度で約9km)。
    let at = |dlat: f64, dlon: f64, altitude: Altitude| Position::world(lat + dlat, lon + dlon, altitude);
    let ground = Altitude::AboveGround(0.0);

    // ---- 絶対座標: 2D図形 ----
    // 地形の起伏に沿って貼り付く円(半透明の緑+輪郭線)。
    add(
        Shape::Circle { center: at(0.0, 0.0, ground), radius: 8_000.0 },
        Style::fill_and_stroke(Color::rgba(0.2, 0.9, 0.3, 0.35), Color::rgb(0.6, 1.0, 0.6), 3.0),
    );
    // 海抜4500mの水平な扇形(レーダーの覆域のイメージ。北東側の90度)。
    add(
        Shape::Sector { center: at(0.0, 0.0, Altitude::Msl(4_500.0)), radius: 40_000.0, start_deg: 0.0, end_deg: 90.0 },
        Style::fill_and_stroke(Color::rgba(1.0, 0.3, 0.2, 0.25), Color::rgb(1.0, 0.5, 0.4), 2.0),
    );
    // 海抜3000mの水平な矩形(30度回転)。
    add(
        Shape::Rect { center: at(-0.15, 0.2, Altitude::Msl(3_000.0)), width: 20_000.0, height: 12_000.0, rotation_deg: 30.0 },
        Style::fill_and_stroke(Color::rgba(0.3, 0.5, 1.0, 0.4), Color::rgb(0.6, 0.75, 1.0), 2.0),
    );
    // 地表に貼り付く凹多角形(L字)。塗りだけ(輪郭線なし)。
    let l_shape = [(0.0, 0.0), (0.24, 0.0), (0.24, 0.08), (0.08, 0.08), (0.08, 0.24), (0.0, 0.24)];
    add(
        Shape::Polygon { points: l_shape.iter().map(|&(dlat, dlon)| at(0.1 + dlat, -0.35 + dlon, ground)).collect() },
        Style::filled(Color::rgba(1.0, 0.9, 0.2, 0.45)),
    );

    // ---- 絶対座標: 3D図形(不透明) ----
    let solid = Style::fill_and_stroke(Color::rgb(0.85, 0.85, 0.9), Color::rgb(0.1, 0.1, 0.15), 1.5);
    add(Shape::Sphere { center: at(0.12, 0.1, Altitude::Msl(5_000.0)), radius: 2_500.0 }, solid);
    add(
        Shape::Cuboid { base_center: at(0.12, 0.2, ground), size_m: [4_000.0, 2_500.0, 5_000.0], heading_deg: 30.0 },
        Style::fill_and_stroke(Color::rgb(0.9, 0.6, 0.3), Color::rgb(0.15, 0.1, 0.05), 1.5),
    );
    add(
        Shape::Cylinder { base_center: at(0.02, 0.2, ground), radius: 1_500.0, height: 6_000.0 },
        Style::fill_and_stroke(Color::rgb(0.4, 0.8, 0.7), Color::rgb(0.05, 0.15, 0.1), 1.5),
    );
    // 半透明の円錐(地表からの高さ500mを底面にして浮かせる)。
    add(
        Shape::Cone { base_center: at(0.02, 0.3, Altitude::AboveGround(500.0)), radius: 2_000.0, height: 7_000.0 },
        Style::fill_and_stroke(Color::rgba(0.8, 0.4, 0.9, 0.5), Color::rgb(0.9, 0.7, 1.0), 1.5),
    );

    // ---- 絶対座標: 折れ線(地表から200mの高さで地形に沿う) ----
    let route = [(-0.3, -0.3), (-0.1, -0.1), (0.05, -0.15), (0.2, 0.05), (0.3, 0.3)];
    add(
        Shape::Polyline {
            points: route.iter().map(|&(dlat, dlon)| at(dlat, dlon, Altitude::AboveGround(200.0))).collect(),
        },
        Style::stroked(Color::rgb(1.0, 0.6, 0.1), 4.0),
    );

    // ---- カメラ固定(画面座標): 左上の枠、中央の十字線、右下の円 ----
    add(
        Shape::Rect {
            center: Position::screen(Corner::TopLeft, 100.0, 60.0),
            width: 160.0,
            height: 80.0,
            rotation_deg: 0.0,
        },
        Style::fill_and_stroke(Color::rgba(0.0, 0.0, 0.0, 0.5), Color::rgb(1.0, 1.0, 1.0), 2.0),
    );
    let cross = Style::stroked(Color::rgba(1.0, 1.0, 1.0, 0.8), 1.5);
    add(
        Shape::Polyline { points: vec![Position::screen(Corner::Center, -12.0, 0.0), Position::screen(Corner::Center, 12.0, 0.0)] },
        cross,
    );
    add(
        Shape::Polyline { points: vec![Position::screen(Corner::Center, 0.0, -12.0), Position::screen(Corner::Center, 0.0, 12.0)] },
        cross,
    );
    add(
        Shape::Circle { center: Position::screen(Corner::BottomRight, -60.0, -60.0), radius: 36.0 },
        Style::fill_and_stroke(Color::rgba(1.0, 0.8, 0.0, 0.5), Color::rgb(1.0, 0.9, 0.3), 3.0),
    );

    // ---- カメラ固定(視点空間): カメラの前方100kmの、中央の少し下に浮かぶ球と直方体 ----
    // (視野の広さは前方距離とcanvasの縦横比で決まる。縦長の画面でも入るよう中央寄りに置く)
    add(
        Shape::Sphere { center: Position::view(-8_000.0, -22_000.0, 100_000.0), radius: 6_000.0 },
        Style::fill_and_stroke(Color::rgb(0.9, 0.3, 0.3), Color::rgb(0.3, 0.05, 0.05), 1.5),
    );
    add(
        Shape::Cuboid {
            base_center: Position::view(6_000.0, -38_000.0, 100_000.0),
            size_m: [9_000.0, 9_000.0, 9_000.0],
            heading_deg: 35.0,
        },
        Style::fill_and_stroke(Color::rgba(0.3, 0.6, 1.0, 0.6), Color::rgb(0.8, 0.9, 1.0), 1.5),
    );

    ids
}
