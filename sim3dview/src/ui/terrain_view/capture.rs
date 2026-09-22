//! マップパネル(canvas)のスクリーンショット(PNG)保存・画面録画(WebM)の実処理。
//! 要求そのものは`terrain::capture::CaptureState`(context)が運び、`mod.rs`のEffectが
//! この中の関数を呼ぶ(このファイルはweb_sys APIを直接叩くだけの下請け)。
//!
//! - スクリーンショット: `HtmlCanvasElement::to_blob_with_type`でPNG化し、その場で
//!   `<a download>`要素を作ってクリックすることでブラウザのダウンロードとして保存する。
//! - 画面録画: `HtmlCanvasElement::capture_stream`(引数なし=canvasが実際に描画されるたびに
//!   フレームが入る。地形は常時アニメーションせず操作時だけ再描画するため、固定fps指定で
//!   毎フレーム取りに行くより効率が良い)を`MediaRecorder`に渡し、`stop()`時にWebMとして
//!   まとめてダウンロードする。
//! - `MediaRecorder.start()`に`timeslice`を渡さないため、`ondataavailable`は`stop()`の
//!   タイミングで録画全体を1つの`Blob`として1回だけ発火する(`onstop`は使わずこれだけで足りる)。
//!   `Closure::once`はJS側から1回呼ばれた時点でRust側のメモリも自動解放されるので、
//!   (`mod.rs`のResizeObserver用クロージャと違い)`forget()`してもリークしない。

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

/// 現在のcanvasの内容をPNGとしてダウンロードする("sim3dview_20260922_153012.png")。
pub(super) fn save_screenshot(canvas: &web_sys::HtmlCanvasElement) {
    let filename = format!("sim3dview_{}.png", timestamp());
    let callback = Closure::once(move |blob: JsValue| match blob.dyn_into::<web_sys::Blob>() {
        Ok(blob) => trigger_download(&blob, &filename),
        Err(_) => log::warn!("[capture] スクリーンショットの生成に失敗しました(toBlobがBlobを返しませんでした)"),
    });
    if canvas.to_blob_with_type(callback.as_ref().unchecked_ref(), "image/png").is_err() {
        log::warn!("[capture] HTMLCanvasElement.toBlob の呼び出しに失敗しました");
    }
    callback.forget();
}

/// 進行中の画面録画のハンドル。`stop()`を呼ぶと、ブラウザ側で録画データがまとまり次第
/// 非同期にWebMとしてダウンロードが始まる(このハンドル自体をdropしても録画は止まらない。
/// 必ず`stop()`を呼ぶこと)。
pub(super) struct Recording {
    recorder: web_sys::MediaRecorder,
}

impl Recording {
    pub(super) fn stop(&self) {
        let _ = self.recorder.stop();
    }
}

/// 画面録画を開始する。ブラウザがMediaRecorder/captureStream未対応の場合はErr。
pub(super) fn start_recording(canvas: &web_sys::HtmlCanvasElement) -> Result<Recording, JsValue> {
    let stream = canvas.capture_stream()?;

    // コーデックの対応状況はブラウザにより異なるため、対応しているものを順に試す
    // (どれも非対応ならオプション省略=ブラウザ既定のコーデックに任せる)。
    const MIME_CANDIDATES: [&str; 3] = ["video/webm;codecs=vp9", "video/webm;codecs=vp8", "video/webm"];
    let mime_type = MIME_CANDIDATES.into_iter().find(|mime| web_sys::MediaRecorder::is_type_supported(mime));

    let recorder = match mime_type {
        Some(mime) => {
            let opts = web_sys::MediaRecorderOptions::new();
            opts.set_mime_type(mime);
            web_sys::MediaRecorder::new_with_media_stream_and_media_recorder_options(&stream, &opts)?
        }
        None => web_sys::MediaRecorder::new_with_media_stream(&stream)?,
    };

    let filename = format!("sim3dview_{}.webm", timestamp());
    let ondataavailable = Closure::once(move |event: web_sys::BlobEvent| {
        if let Some(blob) = event.data() {
            if blob.size() > 0.0 {
                trigger_download(&blob, &filename);
            }
        }
    });
    recorder.set_ondataavailable(Some(ondataavailable.as_ref().unchecked_ref()));
    ondataavailable.forget();

    recorder.start()?;
    Ok(Recording { recorder })
}

/// Blobを`<a download>`要素で即ダウンロードさせる。
fn trigger_download(blob: &web_sys::Blob, filename: &str) {
    let Ok(url) = web_sys::Url::create_object_url_with_blob(blob) else {
        log::warn!("[capture] ダウンロード用URLの生成に失敗しました");
        return;
    };
    (|| -> Option<()> {
        let document = web_sys::window()?.document()?;
        let body = document.body()?;
        let anchor: web_sys::HtmlAnchorElement = document.create_element("a").ok()?.dyn_into().ok()?;
        anchor.set_href(&url);
        anchor.set_download(filename);
        body.append_child(&anchor).ok()?;
        anchor.click();
        body.remove_child(&anchor).ok()?;
        Some(())
    })();
    let _ = web_sys::Url::revoke_object_url(&url);
}

/// ファイル名に使うタイムスタンプ("20260922_153012")。
fn timestamp() -> String {
    let now = js_sys::Date::new_0();
    format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}",
        now.get_full_year(),
        now.get_month() + 1,
        now.get_date(),
        now.get_hours(),
        now.get_minutes(),
        now.get_seconds(),
    )
}
