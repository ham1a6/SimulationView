//! 画面部品に共通の小さな道具。

use wasm_bindgen::JsCast;

/// テキストをクリップボードへコピーする(ブラウザの許可がない・非対応なら何もしない)。
/// `Clipboard`のAPIはWebアクセス(https・localhost)でだけ使える。
pub fn copy_to_clipboard(text: &str) {
    let Some(window) = web_sys::window() else { return };
    let navigator = window.navigator();
    let clipboard = js_sys::Reflect::get(&navigator, &"clipboard".into()).unwrap_or(wasm_bindgen::JsValue::UNDEFINED);
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
