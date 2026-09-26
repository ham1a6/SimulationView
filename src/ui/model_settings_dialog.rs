//! 3Dモデル表示の設定ウインドウ。呼び出し側(アプリ)のメニュー等から`ModelSettingsDialogState`を`true`にすることで開く。
//! `terrain::models::ModelsState`の表示方式・切替距離・最小画面サイズを編集する。通信を一切行わない、
//! ライブラリ内で完結するローカル表示設定。背後の地図を見ながら調整できるよう、モーダルではなく移動できるウインドウにしてある。

use leptos::prelude::*;

use super::floating_panel::FloatingPanel;
use crate::terrain::models::{ModelDisplayMode, ModelsState};
use crate::ui::util::event_f64;

/// 3Dモデル設定ウインドウの開閉状態。トリガー(メニュー等)と本体で共有する。
#[derive(Clone, Copy)]
pub struct ModelSettingsDialogState(pub RwSignal<bool>);

/// 選択肢に並べる表示方式(この順で出す)。
const MODES: [ModelDisplayMode; 3] = [
    ModelDisplayMode::SwitchToSymbol,
    ModelDisplayMode::MinScreenSize,
    ModelDisplayMode::Off,
];

/// `<select>`の`value`に使う、表示方式ごとの識別子。
fn mode_value(mode: ModelDisplayMode) -> &'static str {
    match mode {
        ModelDisplayMode::Off => "off",
        ModelDisplayMode::SwitchToSymbol => "switch",
        ModelDisplayMode::MinScreenSize => "min_size",
    }
}

/// 3Dモデル表示の設定ウインドウ(モーダルではない、ドラッグで動かせるウインドウ)。
/// `ModelSettingsDialogState`・`ModelsState`のcontextが必要。
#[component]
pub fn ModelSettingsDialog() -> impl IntoView {
    let dialog = use_context::<ModelSettingsDialogState>()
        .expect("ModelSettingsDialogState context not found");
    let models = use_context::<ModelsState>().expect("ModelsState context not found");

    // 選ばれた`value`を表示方式に戻して反映する。
    let on_mode = move |ev| {
        let value = event_target_value(&ev);
        if let Some(mode) = MODES.into_iter().find(|m| mode_value(*m) == value) {
            models.mode.set(mode);
        }
    };
    // 数値の入力欄。パースできない・範囲外の入力は反映しない(入力の途中の状態で表示が乱れないように)。
    let number_input = move |signal: RwSignal<f64>, min: f64, max: f64| {
        move |ev| {
            if let Some(v) = event_f64(&ev) {
                if (min..=max).contains(&v) {
                    signal.set(v);
                }
            }
        }
    };

    view! {
        <FloatingPanel open=dialog.0 title="3Dモデル表示" modal=false draggable=true initial_position=(360.0, 120.0)>
            <p class="dialog-description">
                "航空機・艦船・車両などを3Dモデルで表示する方式。モデルが登録されていない種別・読み込み中のトラックは、シンボルのままです。"
            </p>
            <div class="origin-form-row">
                <label>
                    "表示方式"
                    <select
                        prop:value=move || mode_value(models.mode.get())
                        on:change=on_mode
                    >
                        {MODES
                            .into_iter()
                            .map(|mode| view! { <option value=mode_value(mode)>{mode.label()}</option> })
                            .collect_view()}
                    </select>
                </label>
            </div>
            <div class="origin-form-row">
                <label>
                    "切替距離(m)"
                    <input
                        type="number"
                        min="100"
                        max="200000"
                        step="500"
                        disabled=move || models.mode.get() != ModelDisplayMode::SwitchToSymbol
                        prop:value=move || models.switch_distance_m.get().to_string()
                        on:input=number_input(models.switch_distance_m, 100.0, 200_000.0)
                    />
                </label>
                <label>
                    "最小サイズ(px)"
                    <input
                        type="number"
                        min="4"
                        max="512"
                        step="4"
                        disabled=move || models.mode.get() != ModelDisplayMode::MinScreenSize
                        prop:value=move || models.min_screen_px.get().to_string()
                        on:input=number_input(models.min_screen_px, 4.0, 512.0)
                    />
                </label>
            </div>
            <p class="dialog-description">
                "切替距離: カメラからこの距離までのトラックをモデルで描き、それより遠いものはシンボルにします。最小サイズ: 常にモデルで描き、画面での大きさがこれに満たないモデルは実寸より大きくして、この大きさを保ちます。"
            </p>
        </FloatingPanel>
    }
}
