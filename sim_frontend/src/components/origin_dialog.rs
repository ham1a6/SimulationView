//! 原点設定フローティングパネル。メニューバー(設定→原点設定...)から開く。
//! DETAILED_DESIGN.md 3.5節: 地形データ範囲外の値はそもそも送信できないようにする
//! (入力段階でブロック)。範囲(`geodetic_bounds`)はmetadata.jsonから取得する。
//! 中身は旧`operation_panel.rs::OriginForm`をそのままフローティングパネル化したもの。

use leptos::prelude::*;

use crate::protocol::ClientCommand;
use crate::terrain::loader::{self, GeodeticBounds};
use crate::ui_state::OriginDialogState;
use crate::ws::{WsConnection, WsSignals};

#[component]
pub fn OriginDialog(
    /// WsConnectionはRc<RefCell<..>>を含みSend/Syncでないため、propとして受け取る。
    conn: WsConnection,
) -> impl IntoView {
    let signals = use_context::<WsSignals>().expect("WsSignals context not found");
    let dialog = use_context::<OriginDialogState>().expect("OriginDialogState context not found");

    let bounds = RwSignal::new(None::<GeodeticBounds>);
    let lat_input = RwSignal::new(String::new());
    let lon_input = RwSignal::new(String::new());
    // ユーザーが手で編集を始めたら、OriginState受信による自動上書きを止める
    // (サーバーからの再配信で入力中の値が消えてしまうのを防ぐ)。
    let dirty = RwSignal::new(false);

    // 起動時に一度だけmetadata.jsonを取得してバリデーション用のgeodetic_boundsを得る。
    Effect::new(move |_| {
        if bounds.get_untracked().is_some() {
            return;
        }
        wasm_bindgen_futures::spawn_local(async move {
            match loader::fetch_metadata().await {
                Ok(meta) => bounds.set(Some(meta.geodetic_bounds)),
                Err(e) => log::error!("[origin_dialog] failed to fetch metadata.json: {e}"),
            }
        });
    });

    // サーバーからOriginStateが届くたびに、まだ編集していなければ入力欄へ反映する。
    Effect::new(move |_| {
        if let Some(origin) = signals.origin.get() {
            if !dirty.get_untracked() {
                lat_input.set(format!("{:.6}", origin.lat_deg));
                lon_input.set(format!("{:.6}", origin.lon_deg));
            }
        }
    });

    // 入力値を解析し、範囲チェックまで行う。Err内の文字列はそのままUIに表示するメッセージ。
    let parsed = move || -> Result<(f64, f64), String> {
        let lat: f64 = lat_input
            .get()
            .trim()
            .parse()
            .map_err(|_| "緯度は数値で入力してください".to_string())?;
        let lon: f64 = lon_input
            .get()
            .trim()
            .parse()
            .map_err(|_| "経度は数値で入力してください".to_string())?;
        let Some(b) = bounds.get() else {
            return Err("地形データ範囲を取得中です...".to_string());
        };
        if lat < b.min_lat || lat > b.max_lat {
            return Err(format!(
                "緯度は{:.1}〜{:.1}の範囲で入力してください",
                b.min_lat, b.max_lat
            ));
        }
        if lon < b.min_lon || lon > b.max_lon {
            return Err(format!(
                "経度は{:.1}〜{:.1}の範囲で入力してください",
                b.min_lon, b.max_lon
            ));
        }
        Ok((lat, lon))
    };

    view! {
        {move || {
            dialog.0.get().then({
                let conn = conn.clone();
                move || {
                    // このクロージャは開閉のたびに毎回呼ばれる(FnMut)必要があるため、
                    // conn(非Copy)をムーブするon_submitはこの中で毎回新しく作る
                    // (外側で1回だけ作ると、2回目以降の呼び出しでムーブ済みエラーになる)。
                    let on_submit = {
                        let conn = conn.clone();
                        move |_| {
                            if let Ok((lat, lon)) = parsed() {
                                conn.send_command(&ClientCommand::set_origin(lat, lon));
                                dirty.set(false); // 送信後は次に届くOriginStateで表示を更新させる
                            }
                        }
                    };
                    view! {
                        <div class="origin-dialog-backdrop" on:click=move |_| dialog.0.set(false)>
                            <div class="origin-dialog" on:click=|ev| ev.stop_propagation()>
                                <div class="origin-dialog-header">
                                    <h3>"原点設定"</h3>
                                    <button
                                        class="origin-dialog-close"
                                        title="閉じる"
                                        on:click=move |_| dialog.0.set(false)
                                    >
                                        "\u{2715}"
                                    </button>
                                </div>
                                <div class="origin-form-row">
                                    <label>
                                        "緯度"
                                        <input
                                            type="text"
                                            inputmode="decimal"
                                            prop:value=move || lat_input.get()
                                            on:input=move |ev| {
                                                dirty.set(true);
                                                lat_input.set(event_target_value(&ev));
                                            }
                                        />
                                    </label>
                                    <label>
                                        "経度"
                                        <input
                                            type="text"
                                            inputmode="decimal"
                                            prop:value=move || lon_input.get()
                                            on:input=move |ev| {
                                                dirty.set(true);
                                                lon_input.set(event_target_value(&ev));
                                            }
                                        />
                                    </label>
                                    <button on:click=on_submit disabled=move || parsed().is_err()>
                                        "設定"
                                    </button>
                                </div>
                                {move || {
                                    parsed()
                                        .err()
                                        .map(|msg| view! { <p class="field-error">{msg}</p> })
                                }}
                            </div>
                        </div>
                    }
                }
            })
        }}
    }
}
