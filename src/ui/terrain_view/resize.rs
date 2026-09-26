//! canvasの大きさの追従(ResizeObserverと、タブが可視に戻ったときの取り直し)と、高DPIの画面向けの
//! canvasの内部解像度(表示上の大きさ×devicePixelRatio)の決定。

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

/// canvasの内部解像度に掛けるdevicePixelRatioの上限。描画の内部解像度(スーパーサンプリング)は
/// 表示上の大きさの2倍なので(`terrain::renderer`)、2までなら描画の負荷は増えず、縮小の比率が変わる
/// だけ。3倍の画面(スマートフォンなど)でも2倍で描き、残りはブラウザの引き伸ばしに任せる。
const MAX_PIXEL_RATIO: f64 = 2.0;
/// canvasの内部解像度の一辺の上限(物理ピクセル)。スーパーサンプリングの内部テクスチャの上限
/// (`SUPERSAMPLE_MAX_DIMENSION`)と同じ値で、WebGPUが保証する`maxTextureDimension2D`(8192)より十分小さい。
const MAX_CANVAS_PIXELS: u32 = 4096;

/// 表示上の大きさ`css_size`(CSSピクセル)のcanvasに持たせる内部解像度(物理ピクセル)。
/// devicePixelRatioを1〜`MAX_PIXEL_RATIO`に収めて掛け(不正な値なら1)、各辺を`MAX_CANVAS_PIXELS`で頭打ちにする。
pub(super) fn canvas_pixel_size(css_size: (u32, u32), device_pixel_ratio: f64) -> (u32, u32) {
    let ratio = if device_pixel_ratio.is_finite() {
        device_pixel_ratio.clamp(1.0, MAX_PIXEL_RATIO)
    } else {
        1.0
    };
    let side = |v: u32| ((f64::from(v) * ratio).round() as u32).min(MAX_CANVAS_PIXELS);
    (side(css_size.0), side(css_size.1))
}

/// ブラウザのdevicePixelRatio(CSSピクセル1つあたりの物理ピクセル数)。取れなければ1。
pub(super) fn device_pixel_ratio() -> f64 {
    web_sys::window().map_or(1.0, |w| w.device_pixel_ratio())
}

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
    // 戻った時点で実際のCSSサイズを取り直して渡す(大きさもdevicePixelRatioも変わっていなければ
    // `apply_size`側で何もしない。非表示の間に別の画面へ移ってdevicePixelRatioが変わった場合もここで拾う)。
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
        apply_size(width, height);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_size_follows_the_device_pixel_ratio_up_to_two() {
        assert_eq!(canvas_pixel_size((800, 600), 1.0), (800, 600));
        assert_eq!(canvas_pixel_size((800, 600), 1.5), (1200, 900));
        assert_eq!(canvas_pixel_size((800, 600), 2.0), (1600, 1200));
        // 3倍の画面でも2倍まで。
        assert_eq!(canvas_pixel_size((800, 600), 3.0), (1600, 1200));
        // 端数は四捨五入(1.25倍で333.75→334)。
        assert_eq!(canvas_pixel_size((267, 100), 1.25), (334, 125));
    }

    #[test]
    fn pixel_size_never_goes_below_css_size_or_above_the_limit() {
        // ブラウザの縮小表示(devicePixelRatio<1)・不正な値では、表示上の大きさのまま。
        assert_eq!(canvas_pixel_size((800, 600), 0.5), (800, 600));
        assert_eq!(canvas_pixel_size((800, 600), f64::NAN), (800, 600));
        assert_eq!(canvas_pixel_size((800, 600), f64::INFINITY), (800, 600));
        // 各辺を独立に上限で頭打ちにする。
        assert_eq!(
            canvas_pixel_size((3000, 500), 2.0),
            (MAX_CANVAS_PIXELS, 1000)
        );
    }
}
