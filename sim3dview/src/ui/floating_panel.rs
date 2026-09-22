//! 汎用フローティングパネル(タイトルバー+✕で閉じる)。
//! `ui::origin_dialog::OriginDialog`/`ui::coverage_altitude_dialog::CoverageAltitudeDialog`が
//! 内部でこれを使っている。呼び出し側(アプリ)が独自のフローティングウインドウ
//! (例: VAB設定パネルなど)を作る際にも、このコンポーネントで同じ見た目・挙動を再現できる。
//!
//! # 種類
//! - **モーダル**(既定、`modal=true`): 半透明のバックドロップが画面を覆い、中央にパネルを出す。背景クリックか✕で閉じる。
//! - **ウインドウ**(`modal=false`): バックドロップなしで、背後の画面(地図など)を操作したまま出しておける。
//!   ✕でだけ閉じる。位置は`initial_position`(画面左上からのpx)で決める。
//!
//! どちらも`draggable=true`にすると、タイトルバーをドラッグして動かせる(画面の外へ出しきれないよう、
//! タイトルバーの一部が必ず画面内に残る範囲に制限する。動かした位置は、閉じて開き直しても保つ)。
//!
//! 開閉は`open: RwSignal<bool>`を外側から渡してもらう形(`TabbedPanel`の`active`のように
//! 内部で持たない)。中身は常時マウントしたまま`display`だけ切り替える(`TabbedPanel`の
//! タブ切り替えと同じ考え方。開くたびに作り直さない)。

use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// ドラッグ中でも、パネルのこれだけ(px)は必ず画面内に残す(タイトルバーをつかみ直せるように)。
const KEEP_VISIBLE_X_PX: f64 = 80.0;
const KEEP_VISIBLE_Y_PX: f64 = 40.0;
/// `initial_position`を省略したときの、ウインドウの左上の位置(px)。
const DEFAULT_WINDOW_POSITION: (f64, f64) = (80.0, 60.0);

/// ドラッグの開始時点の状態(移動量の許容範囲は、開始時のパネルの位置から決めておく)。
#[derive(Debug, Clone, Copy)]
struct Drag {
    start_x: f64,
    start_y: f64,
    base: (f64, f64),
    dx_range: (f64, f64),
    dy_range: (f64, f64),
}

pub(crate) fn viewport_size() -> (f64, f64) {
    let Some(window) = web_sys::window() else {
        return (1024.0, 768.0);
    };
    let px = |v: Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>, fallback: f64| {
        v.ok().and_then(|v| v.as_f64()).unwrap_or(fallback)
    };
    (
        px(window.inner_width(), 1024.0),
        px(window.inner_height(), 768.0),
    )
}

#[component]
pub fn FloatingPanel(
    /// パネルの開閉状態。呼び出し側が`RwSignal<bool>`を持ち、メニュー項目のクリック等で
    /// `true`にすることでこのパネルを開く。
    open: RwSignal<bool>,
    #[prop(into)] title: String,
    /// trueなら背景を覆うモーダル(既定)、falseなら背後を操作できるウインドウ。
    #[prop(default = true)]
    modal: bool,
    /// タイトルバーのドラッグで動かせるか。
    #[prop(optional)]
    draggable: bool,
    /// ウインドウ(`modal=false`)の初期位置(画面左上からの(x, y)、px)。モーダルでは使わない(中央に出す)。
    #[prop(optional)]
    initial_position: Option<(f64, f64)>,
    children: Children,
) -> impl IntoView {
    let panel_ref: NodeRef<leptos::html::Div> = NodeRef::new();
    // ドラッグで動かした量(モーダルなら中央、ウインドウなら初期位置からの相対、px)。
    let offset = RwSignal::new((0.0_f64, 0.0_f64));
    let drag = RwSignal::new(None::<Drag>);
    let (left, top) = if modal {
        (0.0, 0.0)
    } else {
        initial_position.unwrap_or(DEFAULT_WINDOW_POSITION)
    };

    let on_pointer_down = move |ev: leptos::ev::PointerEvent| {
        // ✕ボタン等の操作はドラッグにしない。
        let on_button = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .is_some_and(|el| el.closest("button").ok().flatten().is_some());
        if !draggable || ev.button() != 0 || on_button {
            return;
        }
        let Some(panel) = panel_ref.get_untracked() else {
            return;
        };
        let rect = panel.get_bounding_client_rect();
        let (vw, vh) = viewport_size();
        drag.set(Some(Drag {
            start_x: f64::from(ev.client_x()),
            start_y: f64::from(ev.client_y()),
            base: offset.get_untracked(),
            dx_range: (
                KEEP_VISIBLE_X_PX - rect.right(),
                vw - KEEP_VISIBLE_X_PX - rect.left(),
            ),
            dy_range: (-rect.top(), vh - KEEP_VISIBLE_Y_PX - rect.top()),
        }));
        if let Some(header) = ev
            .current_target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        {
            let _ = header.set_pointer_capture(ev.pointer_id());
        }
    };
    let on_pointer_move = move |ev: leptos::ev::PointerEvent| {
        let Some(d) = drag.get_untracked() else {
            return;
        };
        let dx = (f64::from(ev.client_x()) - d.start_x)
            .clamp(d.dx_range.0, d.dx_range.1.max(d.dx_range.0));
        let dy = (f64::from(ev.client_y()) - d.start_y)
            .clamp(d.dy_range.0, d.dy_range.1.max(d.dy_range.0));
        offset.set((d.base.0 + dx, d.base.1 + dy));
    };
    let on_pointer_end = move |_ev: leptos::ev::PointerEvent| drag.set(None);

    let panel = view! {
        <div
            node_ref=panel_ref
            class=if modal { "floating-panel" } else { "floating-panel floating-panel--window" }
            style:left=move || if modal { None } else { Some(format!("{left}px")) }
            style:top=move || if modal { None } else { Some(format!("{top}px")) }
            style:transform=move || {
                let (x, y) = offset.get();
                (x != 0.0 || y != 0.0).then(|| format!("translate({x}px, {y}px)"))
            }
            on:click=|ev| ev.stop_propagation()
        >
            <div
                class="floating-panel-header"
                class:draggable=draggable
                on:pointerdown=on_pointer_down
                on:pointermove=on_pointer_move
                on:pointerup=on_pointer_end
                on:pointercancel=on_pointer_end
            >
                <h3>{title}</h3>
                <button class="floating-panel-close" title="閉じる" on:click=move |_| open.set(false)>
                    "\u{2715}"
                </button>
            </div>
            <div class="floating-panel-body">{children()}</div>
        </div>
    };

    if modal {
        view! {
            <div
                class="floating-panel-backdrop"
                style:display=move || if open.get() { "flex" } else { "none" }
                on:click=move |_| open.set(false)
            >
                {panel}
            </div>
        }
        .into_any()
    } else {
        // 画面全体を覆う透明な層(クリックは通す)の上に、ウインドウだけがクリックを受ける。
        view! {
            <div class="floating-window-layer" style:display=move || if open.get() { "block" } else { "none" }>
                {panel}
            </div>
        }
        .into_any()
    }
}
