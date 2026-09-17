//! 全体レイアウト(AppLayout)。DETAILED_DESIGN.md 7.1節: 3カラムの横並びは崩さない。
//! 左パネルは固定幅、メインパネル・右パネルはレスポンシブ対応 + ドラッグでサイズ変更できる。

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::main_panel::MainPanel;
use crate::components::menu_bar::MenuBar;
use crate::components::operation_panel::SimulationStatusPanel;
use crate::components::origin_dialog::OriginDialog;
use crate::components::right_panel::{BottomStatusPanel, TopStatusPanel};
use crate::components::vab::VabPanel;
use crate::terrain::store::TerrainStore;
use crate::ui_state::{OriginDialogState, RadarMarkersState};
use crate::ws::{default_ws_url, WsConnection, WsSignals};

// 地図・右パネルの最小幅(DETAILED_DESIGN.md 7.2節: これを下回ったら外側コンテナを横スクロールさせる)。
const MIN_CENTER_PX: f64 = 320.0;
const MIN_RIGHT_PX: f64 = 260.0;

#[component]
pub fn App() -> impl IntoView {
    let signals = WsSignals::new();
    provide_context(signals);
    // 原点設定フローティングパネルの開閉状態(メニューバーから開く)。
    provide_context(OriginDialogState(RwSignal::new(false)));
    // 地形データはメインパネル・ボトムステータスパネル(断面図タブ)で共有する(フェッチは
    // 1回だけ、BASIC_DESIGN.md 6節フェーズ10: 同一の地形メッシュに異なるカメラを適用する構成)。
    provide_context(TerrainStore::new());
    // 見通し範囲(レーダー観測点)の一覧・選択状態。メインパネル上の右クリックで追加し、
    // ボトムステータスパネルの「見通し範囲」タブで一覧表示・編集・削除する。
    provide_context(RadarMarkersState::new());
    // 接続はページの生存期間ずっと維持する(内部クロージャがselfを保持するため束縛は不要)。
    // WsConnectionはRc<RefCell<..>>を内部に持ちSend/Syncではないため、
    // provide_context(Leptos 0.8はSend+Sync境界を要求する)には乗せず、propとして子へ渡す。
    let conn: WsConnection = WsConnection::connect_new(default_ws_url(), signals);

    // 地図:右パネルの幅の比率。fr単位としてそのままCSSに渡すため絶対値に意味はなく、
    // 比率だけが意味を持つ。ただしドラッグ量(スクリーン座標のpx)をそのまま加減算するため、
    // 初期値もpxスケールの数値にしておく(例: 2.0/1.0のような小さい値だと、
    // 1回のドラッグ量(数百px)だけで下限に張り付いてしまい、ドラッグ操作にならない)。
    let center_fr = RwSignal::new(640.0_f64);
    let right_fr = RwSignal::new(320.0_f64);

    let dragging = RwSignal::new(false);
    let drag_start_x = RwSignal::new(0.0_f64);
    let drag_start_center = RwSignal::new(0.0_f64);
    let drag_start_right = RwSignal::new(0.0_f64);

    let on_pointer_down = move |ev: leptos::ev::PointerEvent| {
        dragging.set(true);
        drag_start_x.set(ev.client_x() as f64);
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
        if !dragging.get_untracked() {
            return;
        }
        let dx = ev.client_x() as f64 - drag_start_x.get_untracked();
        center_fr.set((drag_start_center.get_untracked() + dx).max(MIN_CENTER_PX));
        right_fr.set((drag_start_right.get_untracked() - dx).max(MIN_RIGHT_PX));
    };

    let on_pointer_up = move |_ev: leptos::ev::PointerEvent| {
        dragging.set(false);
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
                        <VabPanel conn=conn.clone()/>
                    </div>

                    <div class="center-panel">
                        <MainPanel/>
                    </div>

                    <div
                        class="resizer"
                        on:pointerdown=on_pointer_down
                        on:pointermove=on_pointer_move
                        on:pointerup=on_pointer_up
                        on:pointercancel=on_pointer_up
                    ></div>

                    <div class="right-panel">
                        <TopStatusPanel/>
                        <BottomStatusPanel/>
                    </div>
                </div>
            </div>
            <OriginDialog conn=conn.clone()/>
        </div>
    }
}
