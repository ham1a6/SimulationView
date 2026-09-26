//! 画面部品に共通の小さな道具。

use wasm_bindgen::JsCast;

/// テキストをクリップボードへコピーする(ブラウザの許可がない・非対応なら何もしない)。
/// `Clipboard`のAPIはWebアクセス(https・localhost)でだけ使える。
pub fn copy_to_clipboard(text: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let navigator = window.navigator();
    let clipboard = js_sys::Reflect::get(&navigator, &"clipboard".into())
        .unwrap_or(wasm_bindgen::JsValue::UNDEFINED);
    if clipboard.is_undefined() || clipboard.is_null() {
        log::warn!("[util] クリップボードが使えない(httpsかlocalhostで開く必要がある)");
        return;
    }
    let write = js_sys::Reflect::get(&clipboard, &"writeText".into())
        .ok()
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
    if let Some(write) = write {
        let _ = write.call1(&clipboard, &text.into());
    }
}

/// SVGのパス`d`へ点(x, y)を足す(`start`なら部分パスの始点`M`、そうでなければ直前の点からの直線`L`)。
pub(crate) fn push_path_point(d: &mut String, start: bool, (x, y): (f64, f64)) {
    use std::fmt::Write;
    let _ = if start {
        write!(d, "M{x:.1},{y:.1}")
    } else {
        write!(d, " L{x:.1},{y:.1}")
    };
}

/// 入力欄のイベントの値を数値として読む(数値でなければNone)。
pub(crate) fn event_f64<T: JsCast>(ev: &T) -> Option<f64> {
    leptos::prelude::event_target_value(ev).parse().ok()
}

/// マウス・ポインターのイベントの画面座標(client座標、CSSピクセル)。
pub(crate) fn client_xy(ev: &web_sys::MouseEvent) -> (f64, f64) {
    (f64::from(ev.client_x()), f64::from(ev.client_y()))
}

/// ブラウザのウインドウの内側の大きさ(CSSピクセル)。取れなければ1024x768。
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

/// 小分けの計算で、1回に続けて進める時間(ミリ秒)。これを超えたら、画面の描画・入力に処理を譲る。
const SLICE_BUDGET_MS: f64 = 8.0;

/// 重い計算を小分けにして進める(画面を固めないため)。`step`は計算を少しだけ進めて、終わったら`true`を返す関数。
/// 1回の持ち時間(`SLICE_BUDGET_MS`)まで`step`を続けて呼び、超えたらブラウザへ処理を譲って(その間に描画・入力が
/// 処理される)続きをやる。`cancelled`が`true`を返したら止めて`false`を返す(計算の結果が要らなくなったとき)。
/// 終わったら`true`を返す。
pub async fn run_in_slices(mut step: impl FnMut() -> bool, cancelled: impl Fn() -> bool) -> bool {
    loop {
        let start = js_sys::Date::now();
        loop {
            if cancelled() {
                return false;
            }
            if step() {
                return true;
            }
            if js_sys::Date::now() - start >= SLICE_BUDGET_MS {
                break;
            }
        }
        gloo_timers::future::TimeoutFuture::new(0).await;
    }
}
