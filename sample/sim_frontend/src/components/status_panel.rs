//! トップステータスパネル(`right_panel.rs`)の「各種情報」タブの中身。
//! 表示項目はC++側が`StatusPanelConfig`で動的に決定する(DETAILED_DESIGN.md 4.3節・7.5節)。
//! フロント側はハードコードしない。

use leptos::prelude::*;

use crate::ws::WsSignals;

#[component]
pub fn StatusPanel() -> impl IntoView {
    let signals = use_context::<WsSignals>().expect("WsSignals context not found");

    view! {
        <ul class="status-values">
            {move || {
                let items = signals
                    .status_panel_config
                    .get()
                    .map(|c| c.items)
                    .unwrap_or_default();
                let values = signals
                    .last_sim_state
                    .get()
                    .map(|s| s.status_values)
                    .unwrap_or_default();

                if items.is_empty() {
                    return view! { <li class="placeholder">"(未受信)"</li> }.into_any();
                }

                items
                    .into_iter()
                    .enumerate()
                    .map(|(i, item)| {
                        let text = match values.get(i) {
                            Some(v) => format!("{}: {:.2} {}", item.label, v, item.unit),
                            None => format!("{}: -- {}", item.label, item.unit),
                        };
                        view! { <li>{text}</li> }
                    })
                    .collect::<Vec<_>>()
                    .into_any()
            }}
        </ul>
    }
}
