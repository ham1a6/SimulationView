//! canvasの大きさの追従(ResizeObserverと、タブが可視に戻ったときの取り直し)。

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

/// `canvas`のCSS上の大きさ(px、整数に丸める)が変わるたびに`apply_size(幅, 高さ)`を呼ぶ。
/// 呼び出し中のオーナー(Effect)が破棄されるときに、監視とリスナーを外す。
pub(super) fn observe_canvas_size(
    canvas: &web_sys::HtmlCanvasElement,
    apply_size: impl Fn(u32, u32) + Clone + 'static,
) {
    // `apply_size`はResizeObserverとvisibilitychangeの2つのコールバックで使うので複製しておく。
    let apply_size_for_resize = apply_size.clone();
    let closure = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
        let Some(entry) = entries
            .get(0)
            .dyn_into::<web_sys::ResizeObserverEntry>()
            .ok()
        else {
            return;
        };
        // 監視しているのはcanvas1つなので、先頭のエントリーだけ見ればよい。
        let rect = entry.content_rect();
        let width = rect.width().round().max(0.0) as u32;
        let height = rect.height().round().max(0.0) as u32;
        apply_size_for_resize(width, height);
    });

    let observer = web_sys::ResizeObserver::new(closure.as_ref().unchecked_ref())
        .expect("ResizeObserver::new failed");
    observer.observe(canvas);

    // ブラウザは非表示(バックグラウンド)タブに対してResizeObserverの通知自体を
    // スロットリング(完全停止)することがある(DEVELOPMENT_HISTORY.md「スプリッタードラッグ時の
    // リサイズ追従」で既知)。ページが非表示のまま初回マウントされると、canvasの
    // 内部解像度がHTML既定値(300×150)のまま一度も更新されず、その後CSSで
    // 実際の表示サイズへ引き伸ばされることでアスペクト比が崩れ、地形の一部
    // (特に画面端寄り・低標高の周辺部)が視野から欠けて見える不具合になっていた。
    // ws.rsのWebSocket再接続と同じPage Visibility APIのパターンで、タブが可視に
    // 戻った時点で実際のCSSサイズを取り直し、ズレていれば取り込み直す。
    let canvas_for_visibility = canvas.clone();
    let visibility_closure = Closure::<dyn FnMut()>::new(move || {
        let hidden = web_sys::window()
            .and_then(|w| w.document())
            .map(|d| d.hidden())
            .unwrap_or(false);
        if hidden {
            return;
        }
        let rect = canvas_for_visibility.get_bounding_client_rect();
        let width = rect.width().round().max(0.0) as u32;
        let height = rect.height().round().max(0.0) as u32;
        if width != canvas_for_visibility.width() || height != canvas_for_visibility.height() {
            apply_size(width, height);
        }
    });
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        let _ = document.add_event_listener_with_callback(
            "visibilitychange",
            visibility_closure.as_ref().unchecked_ref(),
        );
    }

    // クロージャ・observerはこのパネルの生存期間ずっと必要。パネルが破棄されるとき(このEffectの
    // オーナーの後始末)に、observerとdocumentのリスナーを外してからクロージャごと解放する
    // (`forget`すると、クロージャが握る`state`=GPUデバイスまでずっと解放されない)。
    // `on_cleanup`は`Send`を要求するので、JSオブジェクトはローカル専用の`StoredValue`に入れて渡す。
    let resources = StoredValue::new_local((closure, visibility_closure, observer));
    on_cleanup(move || {
        resources.with_value(|(_, visibility_closure, observer)| {
            observer.disconnect();
            if let Some(document) = web_sys::window().and_then(|w| w.document()) {
                let _ = document.remove_event_listener_with_callback(
                    "visibilitychange",
                    visibility_closure.as_ref().unchecked_ref(),
                );
            }
        });
    });
}
