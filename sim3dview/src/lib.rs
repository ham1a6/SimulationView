//! sim3dview: ALOS DEMベースの3D地形描画(wgpu)・レーダー覆域/見通し計算・
//! レーダー観測点管理を提供するLeptos(WASM/CSR)向けライブラリ。
//!
//! 使い方は`sim3dview/README.md`を参照。
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
//! リポジトリ内の`sim3dview/API_REFERENCE.md`に公開モジュールとcontextの索引、
//! `sim3dview/IMPLEMENTATION_GUIDELINES.md`に実装・検証の作業手順をまとめている。

pub mod terrain;
pub mod ui;
pub mod viewer;
