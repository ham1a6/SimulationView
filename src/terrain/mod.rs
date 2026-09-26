//! 地形と、その上に重ねる情報(観測点・覆域・作図・航跡・3Dモデル)のデータと計算、wgpuでの描画。
//! Leptosのコンポーネントは持たない(画面部品は`ui`)。
//!
//! おおまかなデータの流れ(DETAILED_DESIGN.md 6.4節):
//! `fetch`(HTTP取得)→`loader`(`TerrainData`として保持)→`lod`(カメラに応じて各チャンクの解像度を決める)→
//! `mesh`(グリッドから頂点・インデックスを作る)→`renderer`(GPUへ上げて描く)。
//! 座標はすべて、呼び出し側が渡す原点(`origin`)を中心としたENU座標(東・北・上、メートル)で扱い、
//! 緯度経度との変換は`geodesy`が行う。
//!
//! 公開範囲: アプリから直接使う状態・型(`camera`・`drawing`・`markers`・`tracks`・`models`など)は`pub`、
//! 内部の計算・描画の部品は`pub(crate)`にしている。

// 地図のカメラ(注視点を中心に回る3D、真上から見る2D)と、画面座標⇔ワールド座標の変換。
pub mod camera;
// 地図のスクリーンショット(PNG)・画面録画(WebM)の要求を`TerrainView`へ運ぶcontext。
pub mod capture;
// 図形・線を地図上のクリックで作る対話的な作図ツールの状態。
pub mod draw_tool;
// 作図(図形・線)の一覧と表示設定。
pub mod drawing;
// 作図を描くための三角形・線分のジオメトリ生成(地表への貼り付けを含む)。
pub(crate) mod drawing_geometry;
// 地形データのHTTP取得(起動時の一括取得と、細かいレベルのタイル・チャンク取得)。
pub(crate) mod fetch;
// 楕円体と、緯度経度+楕円体高⇔ENU座標の変換。
pub(crate) mod geodesy;
// 緯度経度・ENU座標での標高のサンプリング(表示中のチャンクのレベルを使う)。
pub(crate) mod heightmap;
// 陰影(ヒルシェード)表示のON/OFF状態。
pub mod hillshade;
// 取得した地形データ(メタデータ・タイル一覧・各レベルのグリッド)の保持と、グリッドの参照。
pub(crate) mod loader;
// カメラに応じた、タイル・チャンクごとの解像度レベルの計画(I/Oを持たない純粋な計算)。
pub(crate) mod lod;
// 見通し(レーダーから見える範囲)と覆域(ある高度で見える範囲)の計算。
pub(crate) mod los;
// レーダー観測点の一覧と、覆域の表示設定。
pub mod markers;
// 地形を使わない距離・方位の計算(WGS84楕円体上の測地線)。
pub mod measurement;
// 地形のグリッドから描画用の頂点・インデックスを作る。
pub(crate) mod mesh;
// glTF(GLB)の3Dモデルの読み込み・種類ごとの割り当て・航跡への配置。
pub mod models;
// 原点(ENU座標の中心)の状態。カメラの注視点とは別物。
pub mod origin;
// 地図のクリックで原点を指定するモードの状態。
pub mod origin_pick;
// 画面上の点から地表の緯度経度を求める(レイと地形の交点)。
pub(crate) mod pick;
// 断面図用の、直線に沿った標高のサンプリング。
pub(crate) mod profile;
// 地図の注視点を指定位置(または原点)へ戻す要求。
pub mod recenter;
// 地表に貼り付けるもの(作図・航跡・観測点・覆域)を、Zファイティングを避けて少し持ち上げる高さの一覧。
pub(crate) mod render_bias;
// wgpuのレンダラー(パイプライン・バッファ・描画パス)。
pub(crate) mod renderer;
// 地形データを1回だけ取得して複数のパネルで共有するための、Leptosのリアクティブな入れ物。
pub mod store;
// 航跡(トラック)の一覧・表示設定と、シンボル・軌跡・ラベルのジオメトリ。
pub mod tracks;
// 作図・航跡・マーカーが共通で使う描画用の頂点(`DrawVertex`。地形の頂点は`mesh::TerrainVertex`)。
pub(crate) mod vertex;
