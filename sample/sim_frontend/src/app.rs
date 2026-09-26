//! 全体レイアウト(AppLayout)。DETAILED_DESIGN.md 7.1節: 3カラムの横並びは崩さない。
//! 左パネルは固定幅、メインパネル・右パネルはレスポンシブ対応 + ドラッグでサイズ変更できる。
//! 最下部には画面幅いっぱいのログパネル(7.10節)を置く。

use leptos::prelude::*;

use sim3dview::terrain::models::ModelSource;
use sim3dview::terrain::origin_pick::OriginPickState;
use sim3dview::terrain::tracks::SymbolKind;
use sim3dview::ui::context_menu::{ContextMenu, ContextMenuState};
use sim3dview::ui::coverage_altitude_dialog::{
    CoverageAltitudeDialog, CoverageAltitudeDialogState,
};
use sim3dview::ui::model_settings_dialog::{ModelSettingsDialog, ModelSettingsDialogState};
use sim3dview::ui::origin_dialog::{OriginDialog, OriginDialogState};
use sim3dview::ui::split_pane::SplitPane;
use sim3dview::viewer::ViewerState;

use crate::components::drawing_window::{DrawingWindow, DrawingWindowState};
use crate::components::log_panel::{LogPanel, LogState};
use crate::components::main_panel::MainPanel;
use crate::components::map_menu::provide_map_menu;
use crate::components::menu_bar::MenuBar;
use crate::components::operation_panel::SimulationStatusPanel;
use crate::components::right_panel::{BottomStatusPanel, TopStatusPanel};
use crate::components::slow_wasm_notice::SlowWasmNotice;
use crate::components::upload_window::{UploadWindow, UploadWindowState};
use crate::components::vab::VabPanel;
use crate::log_bridge::bridge_logs;
use crate::protocol::ClientCommand;
use crate::track_bridge::bridge_tracks;
use crate::ws::{default_terrain_base_url, default_ws_url, WsConnection, WsHandle, WsSignals};

// 地図・右パネルの最小幅(DETAILED_DESIGN.md 7.2節: これを下回ったら外側コンテナを横スクロールさせる)。
const MIN_CENTER_PX: f64 = 320.0;
const MIN_RIGHT_PX: f64 = 260.0;

#[component]
pub fn App() -> impl IntoView {
    let show_left_panel = RwSignal::new(true);
    let show_right_panel = RwSignal::new(true);
    let signals = WsSignals::new();
    provide_context(signals);
    // 画面下部のログパネルの内容。`log`クレートの警告・エラーもここへ流す(main.rsのinit_logger)。
    let log = LogState::new();
    provide_context(log);
    log.install_as_log_sink();
    bridge_logs(signals, log);
    // 原点設定フローティングパネルの開閉状態(メニューバーから開く。sim3dviewライブラリの型)。
    provide_context(OriginDialogState(RwSignal::new(false)));
    // 覆域高度設定フローティングパネルの開閉状態(メニューバーから開く。sim3dviewライブラリの型)。
    provide_context(CoverageAltitudeDialogState(RwSignal::new(false)));
    // 作図ウインドウ(移動できる非モーダルのウインドウ)の開閉状態(表示メニューの「作図...」から開く)。
    provide_context(DrawingWindowState(RwSignal::new(false)));
    provide_context(UploadWindowState(RwSignal::new(false)));
    // 地図機能の共有状態。通信・保存キー・モデルの選択はアプリ側で決める。
    let viewer = ViewerState::new(default_terrain_base_url())
        .persist_drawings("sim3dview.user_drawings")
        .provide();
    bridge_tracks(signals, viewer.tracks);
    let models_state = viewer.models;
    for (kind, file) in [
        (SymbolKind::Aircraft, "models/aircraft.glb"),
        (SymbolKind::Helicopter, "models/helicopter.glb"),
        (SymbolKind::Ship, "models/ship.glb"),
        (SymbolKind::Vehicle, "models/vehicle.glb"),
        (SymbolKind::Missile, "models/missile.glb"),
    ] {
        models_state.set_source(kind, ModelSource::new(file));
    }

    // 3Dモデル設定ウインドウの開閉状態(表示メニューから開く。sim3dviewライブラリの型)。
    provide_context(ModelSettingsDialogState(RwSignal::new(false)));
    let origin_state = viewer.origin;
    // 接続はページの生存期間ずっと維持する(内部クロージャがselfを保持するため束縛は不要)。
    // WsConnectionはRc<RefCell<..>>を内部に持ちSend/Syncではないため、Copyのハンドル(WsHandle)に
    // 包んで、propとして子へ渡す。
    let conn = WsHandle::new(WsConnection::connect_new(default_ws_url(), signals));

    // 「設定」→「原点をクリックで指定」で有効になる、地図クリックによる原点指定モード
    // (sim3dviewライブラリの型)。クリックされた緯度経度を、原点設定パネルと同じ
    // ClientCommand::set_originでサーバーへ送る(シミュレーション停止中のみ受理される)。
    provide_context(OriginPickState::new(UnsyncCallback::new(
        move |(lat, lon)| conn.send_command(&ClientCommand::set_origin(lat, lon)),
    )));

    // 右クリックメニュー(sim3dviewライブラリの型)。本体(`<ContextMenu/>`)は下で1つだけ置き、
    // 地図の右クリックの項目(`provide_map_menu`)は、上で提供した各Stateを使うので、それらのあとに提供する。
    provide_context(ContextMenuState::new());
    provide_map_menu();

    // protocol::OriginState(サーバーから配信される生の値)→sim3dview::terrain::origin::OriginState
    // (ライブラリが読む値)への橋渡し。
    Effect::new(move |_| {
        if let Some(o) = signals.origin.get() {
            origin_state.0.set(Some(sim3dview::terrain::origin::Origin {
                lat_deg: o.lat_deg,
                lon_deg: o.lon_deg,
            }));
        }
    });

    let initial_fraction = 2.0 / 3.0;
    view! {
        <div class="app-root">
            <MenuBar show_left_panel=show_left_panel show_right_panel=show_right_panel/>
            // ブラウザがJITを止めていたら(Edgeのセキュリティ強化など)案内の帯を出す(7.11節)。
            <SlowWasmNotice/>
            <div class="app-shell">
                <div class="app-layout" class:left-panel-hidden=move || !show_left_panel.get()>
                    <div class="left-panel" style:display=move || if show_left_panel.get() { "grid" } else { "none" }>
                        <SimulationStatusPanel/>
                        // B1〜B4の中段ページ数。ページ送りは常に表示し、単一ページでは左右とも無効。
                        <VabPanel conn=conn mid_pages=[1usize, 2, 2, 2]/>
                    </div>

                    <SplitPane
                        min_first=MIN_CENTER_PX
                        min_second=MIN_RIGHT_PX
                        initial_fraction=initial_fraction
                        second_visible=Signal::derive(move || show_right_panel.get())
                        first=move || view! {
                            <div class="center-panel"><MainPanel/></div>
                        }
                        second=move || view! {
                            <div class="right-panel">
                                <TopStatusPanel/>
                                <BottomStatusPanel/>
                            </div>
                        }
                    />
                </div>
            </div>
            <LogPanel/>
            <OriginDialog
                on_submit=UnsyncCallback::new(move |(lat, lon)| {
                    conn.send_command(&ClientCommand::set_origin(lat, lon));
                })
            />
            <CoverageAltitudeDialog/>
            <DrawingWindow/>
            <UploadWindow/>
            <ModelSettingsDialog/>
            <ContextMenu/>
        </div>
    }
}
