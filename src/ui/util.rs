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
