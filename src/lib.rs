//! sim3dview: ALOS DEMベースの3D地形描画(wgpu)・レーダー覆域/見通し計算・
//! レーダー観測点管理を提供するLeptos(WASM/CSR)向けライブラリ。
//!
//! 使い方は`README.md`を参照。
//! 実際に組み込んだ最小構成のサンプルアプリは`sample/sim_frontend`にある
//! (VAB・状況パネル・メニュー・WebSocket通信など、アプリ固有の部分はこのライブラリには
//! 含まれない。呼び出し側で実装する)。
//!
//! # 公開APIの入口
//!
//! 状態の一括登録は[`viewer::ViewerState`]、地図は[`ui::terrain_view::TerrainView`]。
//! 作図は[`terrain::drawing::DrawingState`]と[`terrain::draw_tool::DrawToolState`]、
//! 航跡は[`terrain::tracks::TracksState`]、モデルは[`terrain::models::ModelsState`]を使う。
//! 地形取得なしの距離・方位計算は[`terrain::measurement`]を参照。
//!
//! リポジトリ内の`API_REFERENCE.md`に公開モジュールとcontextの索引、
//! `IMPLEMENTATION_GUIDELINES.md`に実装・検証の作業手順をまとめている。
//!
//! # 全体の構成
//!
//! 大きく「データと計算(`terrain`)」と「画面部品(`ui`)」に分かれる。`terrain`はLeptosの
//! コンポーネントを持たず、地形の取得・保持・メッシュ化・描画(wgpu)・見通し計算・作図や航跡の
//! 状態を扱う。`ui`はそれらを使うLeptosコンポーネントで、`viewer::ViewerState`がまとめて
//! 登録したcontextを通して`terrain`側の状態を共有する(設計はDETAILED_DESIGN.md 6.0節・9.13節)。
//! ライブラリは通信プロトコルやサーバーのURLを知らない(URLの組み立て・WebSocket・原点状態の
//! 橋渡しは呼び出し側のアプリの責務。DETAILED_DESIGN.md 0.4節)。

// 地形データの取得・保持、LOD、メッシュ生成、wgpu描画、見通し計算、作図・航跡・3Dモデルの状態。
pub mod terrain;
// 地図(`TerrainView`)・見通し図・断面図・ダイアログ・分割ペインなどのLeptosコンポーネント。
pub mod ui;
// 任意のファイル(Blob)をサーバーへ送る汎用API。送信先URLは呼び出し側が決める。
pub mod upload;
// ライブラリが使うcontextを一括で作って登録する入口(`ViewerState`)。
pub mod viewer;
