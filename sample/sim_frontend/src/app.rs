//! 全体レイアウト(AppLayout)。DETAILED_DESIGN.md 7.1節: 3カラムの横並びは崩さない。
//! 左パネルは固定幅、メインパネル・右パネルはレスポンシブ対応 + ドラッグでサイズ変更できる。

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use sim3dview::terrain::capture::CaptureState;
use sim3dview::terrain::draw_tool::DrawToolState;
use sim3dview::terrain::drawing::DrawingState;
use sim3dview::terrain::hillshade::HillshadeState;
use sim3dview::terrain::markers::RadarMarkersState;
use sim3dview::terrain::models::{ModelSource, ModelsState};
use sim3dview::terrain::origin::OriginState;
use sim3dview::terrain::origin_pick::OriginPickState;
use sim3dview::terrain::recenter::RecenterRequestState;
use sim3dview::terrain::store::TerrainStore;
use sim3dview::terrain::tracks::{SymbolKind, TracksState};
use sim3dview::ui::context_menu::{ContextMenu, ContextMenuState};
use sim3dview::ui::coverage_altitude_dialog::{
    CoverageAltitudeDialog, CoverageAltitudeDialogState,
};
use sim3dview::ui::model_settings_dialog::{ModelSettingsDialog, ModelSettingsDialogState};
use sim3dview::ui::origin_dialog::{OriginDialog, OriginDialogState};
use sim3dview::ui::pointer_drag::DragTracker;

use crate::components::drawing_window::{DrawingWindow, DrawingWindowState};
use crate::components::main_panel::MainPanel;
use crate::components::map_menu::provide_map_menu;
use crate::components::menu_bar::MenuBar;
use crate::components::operation_panel::SimulationStatusPanel;
use crate::components::right_panel::{BottomStatusPanel, TopStatusPanel};
use crate::components::vab::VabPanel;
use crate::protocol::ClientCommand;
use crate::track_bridge::bridge_tracks;
use crate::ws::{default_terrain_base_url, default_ws_url, WsConnection, WsHandle, WsSignals};

// 地図・右パネルの最小幅(DETAILED_DESIGN.md 7.2節: これを下回ったら外側コンテナを横スクロールさせる)。
const MIN_CENTER_PX: f64 = 320.0;
const MIN_RIGHT_PX: f64 = 260.0;

#[component]
pub fn App() -> impl IntoView {
    let signals = WsSignals::new();
    provide_context(signals);
    // 原点設定フローティングパネルの開閉状態(メニューバーから開く。sim3dviewライブラリの型)。
    provide_context(OriginDialogState(RwSignal::new(false)));
    // 覆域高度設定フローティングパネルの開閉状態(メニューバーから開く。sim3dviewライブラリの型)。
    provide_context(CoverageAltitudeDialogState(RwSignal::new(false)));
    // 作図ウインドウ(移動できる非モーダルのウインドウ)の開閉状態(表示メニューの「作図...」から開く)。
    provide_context(DrawingWindowState(RwSignal::new(false)));
    // 地形データはメインパネル・見通し範囲タブで共有する(フェッチは1回だけ、
    // DETAILED_DESIGN.md 6.0・6.5節: 同一の地形データに異なるカメラを適用する構成)。
    // sample/sim_server(ポート9001)の`/terrain/*`から取得する(sim3dviewライブラリ自体は
    // サーバーのホスト名・ポートを知らない。terrain::loader参照)。
    provide_context(TerrainStore::new(default_terrain_base_url()));
    // 見通し範囲(レーダー観測点)の一覧・選択状態(sim3dviewライブラリの型)。メインパネル上の
    // 右クリックメニュー「ここにレーダー観測点を追加」で追加し、ボトムステータスパネルの「見通し範囲」タブで一覧表示・編集・削除する。
    provide_context(RadarMarkersState::new());
    // 表示メニューの「中心点を原点に戻す」ボタンからTerrainViewへの通知チャンネル
    // (sim3dviewライブラリの型)。
    provide_context(RecenterRequestState::new());
    // 表示メニューの「陰影表示」のON/OFF(sim3dviewライブラリの型。既定はON)。
    provide_context(HillshadeState::default());
    // VABパネルの「スクリーンショット」「録画開始/停止」ボタンからTerrainViewへの要求
    // (sim3dviewライブラリの型)。サーバーへは何も送らない、完全にフロント側だけの機能。
    provide_context(CaptureState::new());
    // 作図(図形・線)の一覧(sim3dviewライブラリの型)。表示メニューの「作図デモ」が図形を出し入れする。
    let drawings = DrawingState::new();
    provide_context(drawings);
    // 図形の対話作成(右パネルの「作図」タブ+地図のクリック)。作った図形はブラウザ(localStorage)に保存して、
    // 次回の起動時に復元する(sim3dviewライブラリの型)。
    provide_context(DrawToolState::new(drawings).persist("sim3dview.user_drawings"));
    // 航跡(航空機・艦船・車両等)の一覧と表示設定(sim3dviewライブラリの型)。サーバーから届く`TrackList`を
    // 下の`bridge_tracks`が反映し、表示メニューの「ラベル/航跡/高度線」が表示設定を切り替える。
    let tracks_state = TracksState::new();
    provide_context(tracks_state);
    bridge_tracks(signals, tracks_state);
    // 3Dモデル(glTF)表示の設定(sim3dviewライブラリの型)。種別ごとに使うモデルを登録する(ファイルは`assets/models/`。
    // `index.html`のcopy-dirでtrunkが配信する。`scripts/gen_sample_models.py`が生成した簡易なモデル)。
    // 表示メニューの「3Dモデル...」で、表示方式(距離で切替/最小サイズを保証/シンボルのみ)を設定する。
    let models_state = ModelsState::new();
    for (kind, file) in [
        (SymbolKind::Aircraft, "models/aircraft.glb"),
        (SymbolKind::Helicopter, "models/helicopter.glb"),
        (SymbolKind::Ship, "models/ship.glb"),
        (SymbolKind::Vehicle, "models/vehicle.glb"),
        (SymbolKind::Missile, "models/missile.glb"),
    ] {
        models_state.set_source(kind, ModelSource::new(file));
    }
    provide_context(models_state);
    // 3Dモデル設定ウインドウの開閉状態(表示メニューから開く。sim3dviewライブラリの型)。
    provide_context(ModelSettingsDialogState(RwSignal::new(false)));
    // 現在の原点(sim3dviewライブラリの型)。TerrainViewはこれを読んでメッシュを再計算する。
    // サーバーからのOriginState(protocol)が届くたびに、下のEffectでこちらへミラーする
    // (sim3dviewライブラリは通信プロトコルを一切知らないため、この橋渡しはアプリ側の役目)。
    let origin_state = OriginState(RwSignal::new(None));
    provide_context(origin_state);

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

    // 地図:右パネルの幅の比率。fr単位としてそのままCSSに渡すため絶対値に意味はなく、
    // 比率だけが意味を持つ。ただしドラッグ量(スクリーン座標のpx)をそのまま加減算するため、
    // 初期値もpxスケールの数値にしておく(例: 2.0/1.0のような小さい値だと、
    // 1回のドラッグ量(数百px)だけで下限に張り付いてしまい、ドラッグ操作にならない)。
    let center_fr = RwSignal::new(640.0_f64);
    let right_fr = RwSignal::new(320.0_f64);

    let drag = RwSignal::new(DragTracker::default());
    let drag_start_center = RwSignal::new(0.0_f64);
    let drag_start_right = RwSignal::new(0.0_f64);

    let on_pointer_down = move |ev: leptos::ev::PointerEvent| {
        drag.update(|drag| {
            drag.begin(
                ev.pointer_id(),
                f64::from(ev.client_x()),
                f64::from(ev.client_y()),
            )
        });
        drag_start_center.set(center_fr.get_untracked());
        drag_start_right.set(right_fr.get_untracked());
        // ポインタキャプチャ: ドラッグ中にカーソルがリサイザー外に出てもイベントを受け取り続ける。
        if let Some(target) = ev.target() {
            if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                let _ = el.set_pointer_capture(ev.pointer_id());
            }
        }
    };

    let on_pointer_move = move |ev: leptos::ev::PointerEvent| {
        let Some(update) = drag
            .try_update(|drag| {
                drag.update(
                    ev.pointer_id(),
                    f64::from(ev.client_x()),
                    f64::from(ev.client_y()),
                )
            })
            .flatten()
        else {
            return;
        };
        let dx = update.total.0;
        center_fr.set((drag_start_center.get_untracked() + dx).max(MIN_CENTER_PX));
        right_fr.set((drag_start_right.get_untracked() - dx).max(MIN_RIGHT_PX));
    };

    let on_pointer_up = move |ev: leptos::ev::PointerEvent| {
        drag.update(|drag| {
            drag.end(
                ev.pointer_id(),
                f64::from(ev.client_x()),
                f64::from(ev.client_y()),
            );
        });
    };
    let on_pointer_cancel = move |ev: leptos::ev::PointerEvent| {
        drag.update(|drag| {
            drag.cancel(ev.pointer_id());
        });
    };

    // 4列: 左パネル(固定) / メインパネル / リサイザー / 右パネル。
    // minmaxの下限により、収まらない場合はグリッド自体が広がり、
    // 親(.app-shell)のoverflow-x:autoで横スクロールになる(縦積みへの再レイアウトはしない)。
    let grid_columns = move || {
        format!(
            "var(--panel-width) minmax({MIN_CENTER_PX}px, {}fr) 6px minmax({MIN_RIGHT_PX}px, {}fr)",
            center_fr.get(),
            right_fr.get()
        )
    };

    view! {
        <div class="app-root">
            <MenuBar/>
            <div class="app-shell">
                <div class="app-layout" style:grid-template-columns=grid_columns>
                    <div class="left-panel">
                        <SimulationStatusPanel/>
                        // 中段のページ数(1=単一ページ、2以上=ページ送りあり)。ここで自由に決められる。
                        <VabPanel conn=conn mid_pages=2usize/>
                    </div>

                    <div class="center-panel">
                        <MainPanel/>
                    </div>

                    <div
                        class="resizer"
                        on:pointerdown=on_pointer_down
                        on:pointermove=on_pointer_move
                        on:pointerup=on_pointer_up
                        on:pointercancel=on_pointer_cancel
                    ></div>

                    <div class="right-panel">
                        <TopStatusPanel/>
                        <BottomStatusPanel/>
                    </div>
                </div>
            </div>
            <OriginDialog
                on_submit=UnsyncCallback::new(move |(lat, lon)| {
                    conn.send_command(&ClientCommand::set_origin(lat, lon));
                })
            />
            <CoverageAltitudeDialog/>
            <DrawingWindow/>
            <ModelSettingsDialog/>
            <ContextMenu/>
        </div>
    }
}
