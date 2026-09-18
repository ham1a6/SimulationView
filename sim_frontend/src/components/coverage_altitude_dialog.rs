//! 覆域高度設定フローティングパネル。メニューバー(設定→覆域高度設定...)から開く。
//! メインパネルの2D表示モードで、選択中レーダーの探知可能領域を表示する対象の海抜高度
//! (`ui_state::RadarMarkersState::coverage_altitude_m`)を編集する。原点設定とは異なり
//! サーバーへは何も送らない、フロント側だけのローカル表示設定(`origin_dialog.rs`と同じ
//! 見た目のパネルだが、`WsConnection`は不要なためpropで受け取っていない)。

use leptos::prelude::*;

use crate::ui_state::{CoverageAltitudeDialogState, RadarMarkersState};

#[component]
pub fn CoverageAltitudeDialog() -> impl IntoView {
    let dialog = use_context::<CoverageAltitudeDialogState>()
        .expect("CoverageAltitudeDialogState context not found");
    let radar_markers =
        use_context::<RadarMarkersState>().expect("RadarMarkersState context not found");

    view! {
        {move || {
            dialog.0.get().then(|| view! {
                <div class="origin-dialog-backdrop" on:click=move |_| dialog.0.set(false)>
                    <div class="origin-dialog" on:click=|ev| ev.stop_propagation()>
                        <div class="origin-dialog-header">
                            <h3>"覆域高度設定"</h3>
                            <button
                                class="origin-dialog-close"
                                title="閉じる"
                                on:click=move |_| dialog.0.set(false)
                            >
                                "\u{2715}"
                            </button>
                        </div>
                        <p class="dialog-description">
                            "メインパネルの2D表示で、選択中のレーダーの探知可能領域を表示する際の海抜高度(m)。"
                        </p>
                        <div class="origin-form-row">
                            <label>
                                "高度(m)"
                                <input
                                    type="number"
                                    step="10"
                                    prop:value=move || radar_markers.coverage_altitude_m.get().to_string()
                                    on:input=move |ev| {
                                        if let Ok(v) = event_target_value(&ev).parse::<f64>() {
                                            radar_markers.coverage_altitude_m.set(v);
                                        }
                                    }
                                />
                            </label>
                        </div>
                    </div>
                </div>
            })
        }}
    }
}
