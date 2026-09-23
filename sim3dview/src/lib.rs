//! sim3dview: ALOS DEMベースの3D地形描画(wgpu)・レーダー覆域/見通し計算・
//! レーダー観測点管理を提供するLeptos(WASM/CSR)向けライブラリ。
//!
//! 使い方は`sim3dview/README.md`を参照。
//! 実際に組み込んだ最小構成のサンプルアプリは`sample/sim_frontend`にある
//! (VAB・状況パネル・メニュー・WebSocket通信など、アプリ固有の部分はこのライブラリには
//! 含まれない。呼び出し側で実装する)。

pub mod terrain;
pub mod ui;
pub mod viewer;
