//! 汎用フローティングパネル(半透明バックドロップ+中央パネル、背景クリックか✕で閉じる)。
//! `ui::origin_dialog::OriginDialog`/`ui::coverage_altitude_dialog::CoverageAltitudeDialog`が
//! 内部でこれを使っている。呼び出し側(アプリ)が独自のフローティングウインドウ
//! (例: VAB設定パネルなど)を作る際にも、このコンポーネントで同じ見た目・挙動を再現できる。
//!
//! 開閉は`open: RwSignal<bool>`を外側から渡してもらう形(`TabbedPanel`の`active`のように
//! 内部で持たない)。中身は常時マウントしたまま`display`だけ切り替える(`TabbedPanel`の
//! タブ切り替えと同じ考え方。開くたびに作り直さない)。

use leptos::prelude::*;

#[component]
pub fn FloatingPanel(
    /// パネルの開閉状態。呼び出し側が`RwSignal<bool>`を持ち、メニュー項目のクリック等で
    /// `true`にすることでこのパネルを開く。
    open: RwSignal<bool>,
    #[prop(into)] title: String,
    children: Children,
) -> impl IntoView {
    view! {
        <div
            class="floating-panel-backdrop"
            style:display=move || if open.get() { "flex" } else { "none" }
            on:click=move |_| open.set(false)
        >
            <div class="floating-panel" on:click=|ev| ev.stop_propagation()>
                <div class="floating-panel-header">
                    <h3>{title}</h3>
                    <button class="floating-panel-close" title="閉じる" on:click=move |_| open.set(false)>
                        "\u{2715}"
                    </button>
                </div>
                <div class="floating-panel-body">{children()}</div>
            </div>
        </div>
    }
}
