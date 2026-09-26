//! ライブラリの汎用送信APIを使うファイル転送のサンプル。
use leptos::prelude::*;
use sim3dview::ui::floating_panel::FloatingPanel;
use sim3dview::upload::upload_blob;

#[derive(Clone, Copy)]
pub struct UploadWindowState(pub RwSignal<bool>);

const MAX_UPLOAD_BYTES: f64 = 64.0 * 1024.0 * 1024.0;

#[component]
pub fn UploadWindow() -> impl IntoView {
    let state = use_context::<UploadWindowState>().expect("UploadWindowState context not found");
    let log = use_context::<crate::components::log_panel::LogState>().expect("LogState context not found");
    let selected = RwSignal::new_local(None::<web_sys::File>);
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    view! {
        <FloatingPanel open=state.0 title="サーバーへファイル転送">
            <p>"1ファイル64 MiBまで。サーバーが発行する保存IDで保存します。"</p>
            <input type="file" aria-label="送信するファイル" disabled=move || busy.get()
                on:change=move |event| {
                    let input = event_target::<web_sys::HtmlInputElement>(&event);
                    selected.set(input.files().and_then(|files| files.get(0)));
                    message.set(String::new());
                }
            />
            <button disabled=move || busy.get() || selected.with(|file| file.is_none())
                on:click=move |_| {
                    if busy.get_untracked() { return; }
                    let Some(file) = selected.get_untracked() else { return; };
                    if file.size() > MAX_UPLOAD_BYTES {
                        message.set("ファイルが64 MiBを超えています。".into());
                        return;
                    }
                    busy.set(true);
                    message.set("送信中…".into());
                    wasm_bindgen_futures::spawn_local(async move {
                        let result = upload_blob(&crate::ws::default_upload_url(), &file, &[]).await;
                        match result {
                            Ok(id) => {
                                let text = format!("{} の送信が完了しました。保存先: {id}", file.name());
                                log.info(text.clone());
                                message.set(text);
                            }
                            Err(error) => {
                                log.error(format!("{} の送信に失敗: {error}", file.name()));
                                message.set(error);
                            }
                        }
                        busy.set(false);
                    });
                }
            >{move || if busy.get() { "送信中…" } else { "送信" }}</button>
            <p role="status" aria-live="polite" style="overflow-wrap: anywhere">{move || message.get()}</p>
        </FloatingPanel>
    }
}
