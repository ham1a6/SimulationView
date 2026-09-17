//! 左パネル上: 操作パネル(ステータス表示 + 原点入力フォーム)。DETAILED_DESIGN.md 7.1節参照。
//! VAB(左パネル下)は`vab.rs`。

use leptos::prelude::*;

use crate::protocol::ClientCommand;
use crate::terrain::loader::{self, GeodeticBounds};
use crate::ws::{ConnectionStatus, WsConnection, WsSignals};

#[component]
pub fn OperationPanel(
    /// WsConnectionはRc<RefCell<..>>を含みSend/Syncでないため(vab.rsと同じ理由)、
    /// contextではなくpropとして受け取る。
    conn: WsConnection,
) -> impl IntoView {
    let signals = use_context::<WsSignals>().expect("WsSignals context not found");

    let status_text = move || signals.status.get().to_string();
    let status_class = move || match signals.status.get() {
        ConnectionStatus::Connected => "status status--connected",
        ConnectionStatus::Connecting => "status status--connecting",
        ConnectionStatus::Reconnecting { .. } => "status status--reconnecting",
        ConnectionStatus::PausedHidden => "status status--paused",
    };

    view! {
        <div class="panel-section operation-panel">
            <h2>"ステータス"</h2>
            <p class=status_class>{status_text}</p>

            <dl class="kv-list">
                <dt>"原点"</dt>
                <dd>
                    {move || match signals.origin.get() {
                        Some(o) => format!("{:.6}, {:.6}", o.lat_deg, o.lon_deg),
                        None => "--".to_string(),
                    }}
                </dd>

                <dt>"フレーム"</dt>
                <dd>
                    {move || match signals.last_sim_state.get() {
                        Some(s) => format!("#{} (t={:.1}s)", s.frame_id, s.t),
                        None => "--".to_string(),
                    }}
                </dd>
            </dl>

            <OriginForm conn=conn signals=signals/>

            {move || {
                signals
                    .last_command_error
                    .get()
                    .map(|e| view! {
                        <p class="command-error">
                            "[" {e.command_type} "] " {e.message}
                        </p>
                    })
            }}
        </div>
    }
}

/// 原点入力フォーム。DETAILED_DESIGN.md 3.5節: 地形データ範囲外の値はそもそも送信できないようにする
/// (入力段階でブロック)。範囲(`geodetic_bounds`)はmetadata.jsonから取得する。
#[component]
fn OriginForm(conn: WsConnection, signals: WsSignals) -> impl IntoView {
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
                Err(e) => log::error!("[origin_form] failed to fetch metadata.json: {e}"),
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

    let on_submit = move |_| {
        if let Ok((lat, lon)) = parsed() {
            conn.send_command(&ClientCommand::set_origin(lat, lon));
            dirty.set(false); // 送信後は次に届くOriginStateで表示を更新させる
        }
    };

    view! {
        <div class="origin-form">
            <h3>"原点設定"</h3>
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
    }
}
