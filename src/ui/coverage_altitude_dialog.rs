//! 覆域高度設定フローティングパネル。呼び出し側(アプリ)のメニュー等から
//! `CoverageAltitudeDialogState`を`true`にすることで開く。`TerrainView`の2D表示モードで、
//! 選択中レーダーの探知可能領域を表示する対象の海抜高度(`terrain::markers::
//! RadarMarkersState::coverage_altitude_m`)を編集する。原点設定とは異なり通信を一切
//! 行わない、ライブラリ内で完結するローカル表示設定。

use leptos::prelude::*;

use super::floating_panel::FloatingPanel;
use crate::terrain::markers::RadarMarkersState;
use crate::ui::util::event_f64;

/// 覆域高度設定フローティングパネルの開閉状態。トリガー(メニュー等)と本体で共有する。
#[derive(Clone, Copy)]
pub struct CoverageAltitudeDialogState(pub RwSignal<bool>);

#[component]
pub fn CoverageAltitudeDialog() -> impl IntoView {
    let dialog = use_context::<CoverageAltitudeDialogState>()
        .expect("CoverageAltitudeDialogState context not found");
    let radar_markers =
        use_context::<RadarMarkersState>().expect("RadarMarkersState context not found");

    view! {
        <FloatingPanel open=dialog.0 title="覆域高度設定">
            <p class="dialog-description">
                "選択中のレーダーの探知可能領域を表示する際の海抜高度(m)(2D表示モードで使用)。"
            </p>
            <div class="origin-form-row">
                <label>
                    "高度(m)"
                    <input
                        type="number"
                        step="10"
                        prop:value=move || radar_markers.coverage_altitude_m.get().to_string()
                        on:input=move |ev| {
                            if let Some(v) = event_f64(&ev) {
                                radar_markers.coverage_altitude_m.set(v);
                            }
                        }
                    />
                </label>
            </div>
        </FloatingPanel>
    }
}
