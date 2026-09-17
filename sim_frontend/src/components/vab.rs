//! 左パネル下: VAB(操作ボタングリッド)。DETAILED_DESIGN.md 7.4節。
//! - 配置・ラベル・有効/無効はサーバーの`VabConfig`が決める(ハードコードしない)
//! - 操作は単純クリックのみ
//! - ラベルが空文字のボタンは「未使用の穴」としてDOM要素自体を描画しない
//!   (グリッド位置がずれないよう、各ボタンにrow/columnを明示的に指定する)

use leptos::prelude::*;

use crate::protocol::ClientCommand;
use crate::ws::WsConnection;
use crate::ws::WsSignals;

#[component]
pub fn Vab(
    /// WsConnectionはRc<RefCell<..>>を含みSend/Syncでないため、
    /// (Leptos 0.8のprovide_contextが要求する境界を満たせない)、propとして受け取る。
    conn: WsConnection,
) -> impl IntoView {
    let signals = use_context::<WsSignals>().expect("WsSignals context not found");

    view! {
        <div class="panel-section vab-panel">
            <h2>"VAB"</h2>
            {move || {
                let Some(cfg) = signals.vab_config.get() else {
                    return view! { <p class="placeholder">"(未受信)"</p> }.into_any();
                };
                let cols = cfg.cols.max(1) as usize;
                let grid_style = format!(
                    "grid-template-columns: repeat({}, 1fr); grid-template-rows: repeat({}, auto);",
                    cfg.cols, cfg.rows
                );
                let conn = conn.clone();

                view! {
                    <div class="vab-grid" style=grid_style>
                        {cfg.buttons
                            .into_iter()
                            .enumerate()
                            // ラベルが空文字のボタンは「未使用の穴」: DOM要素を一切生成しない。
                            .filter(|(_, btn)| !btn.label.is_empty())
                            .map(|(i, btn)| {
                                let row = i / cols + 1;
                                let col = i % cols + 1;
                                let pos_style = format!("grid-row: {row}; grid-column: {col};");
                                let button_id = btn.id.clone();
                                let enabled = btn.enabled;
                                let conn = conn.clone();
                                let on_click = move |_| {
                                    if enabled {
                                        conn.send_command(&ClientCommand::vab_press(button_id.clone()));
                                    }
                                };
                                view! {
                                    <button
                                        class="vab-button"
                                        style=pos_style
                                        disabled=!enabled
                                        on:click=on_click
                                    >
                                        {btn.label}
                                    </button>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </div>
                }
                    .into_any()
            }}
        </div>
    }
}
