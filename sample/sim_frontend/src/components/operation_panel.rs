//! 左パネル上: シミュレーションステータスパネル(接続状態・原点・フレームの読み取り専用表示)。
//! DETAILED_DESIGN.md 7.1節参照。VABパネル(左パネル下)は`vab.rs`。
//! 原点を変更するフォームは、メニューバー(設定→原点設定...)から開く
//! フローティングパネル(`origin_dialog.rs`)へ移動済み。

use leptos::prelude::*;

use crate::ws::{ConnectionStatus, WsSignals};

#[component]
pub fn SimulationStatusPanel() -> impl IntoView {
    let signals = use_context::<WsSignals>().expect("WsSignals context not found");

    // 接続済みの間は、シミュレータアプリケーション自身が送ってきた状態文字列を
    // そのまま表示する。それ以外の状態(接続中・再接続試行中・一時停止中)は、
    // 詳細を出し分けず一律「接続中」と表示する(バッジの色分けは維持する)。
    let status_text = move || match signals.status.get() {
        ConnectionStatus::Connected => signals
            .app_status
            .get()
            .map(|s| s.text)
            .unwrap_or_else(|| "接続済み".to_string()),
        _ => "接続中".to_string(),
    };
    let status_class = move || match signals.status.get() {
        ConnectionStatus::Connected => "status status--connected",
        ConnectionStatus::Connecting => "status status--connecting",
        ConnectionStatus::Reconnecting { .. } => "status status--reconnecting",
        ConnectionStatus::PausedHidden => "status status--paused",
    };

    view! {
        <div class="panel-section operation-panel">
            <h2>"シミュレーションステータスパネル"</h2>
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
