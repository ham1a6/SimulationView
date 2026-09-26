//! 原点設定フローティングパネル。呼び出し側(アプリ)のメニュー等から
//! `OriginDialogState`を`true`にすることで開く。DETAILED_DESIGN.md 9.14節:
//! 地形データ範囲外の値はそもそも送信できないようにする(入力段階でブロック)。
//! 範囲(`geodetic_bounds`)は地形データ(`terrain::store::TerrainStore`。metadata.json)から読む。
//!
//! 実際に緯度経度をどう配信するか(通信プロトコル)はアプリごとに異なるため、
//! ライブラリはそれを知らない。「設定」ボタンが押されたら`on_submit`コールバックを
//! 呼ぶだけで、呼び出し側がその中で自分のプロトコルに応じた送信を行う。

use leptos::prelude::*;

use super::floating_panel::FloatingPanel;
use crate::terrain::origin::OriginState;
use crate::terrain::store::TerrainStore;

/// 原点設定フローティングパネルの開閉状態。トリガー(メニュー等)と本体で共有する。
#[derive(Clone, Copy)]
pub struct OriginDialogState(pub RwSignal<bool>);

/// 原点設定のフローティングパネル。`OriginState`・`OriginDialogState`・`TerrainStore`のcontextが必要。
/// 入力が数値で地形データの範囲内のときだけ「設定」ボタンを押せ、押すと`on_submit`を呼ぶ
/// (`OriginState`は書き換えない。アプリが送信し、配信されてきた原点で更新される想定)。
#[component]
pub fn OriginDialog(
    /// 「設定」ボタンで緯度経度が確定した際に呼ばれる。実際の送信方法は呼び出し側に委ねる。
    on_submit: UnsyncCallback<(f64, f64)>,
) -> impl IntoView {
    let origin_state = use_context::<OriginState>().expect("OriginState context not found");
    let dialog = use_context::<OriginDialogState>().expect("OriginDialogState context not found");

    let terrain_store = use_context::<TerrainStore>().expect("TerrainStore context not found");
    // バリデーション用の地形データの範囲。`TerrainView`が取得したデータ(`TerrainStore`)を共有する
    // (このパネルだけがmetadata.jsonを別に取得し直すことはしない)。
    let bounds = move || terrain_store.get().map(|d| d.metadata.geodetic_bounds);
    let lat_input = RwSignal::new(String::new());
    let lon_input = RwSignal::new(String::new());
    // ユーザーが手で編集を始めたら、OriginState受信による自動上書きを止める
    // (再配信で入力中の値が消えてしまうのを防ぐ)。
    let dirty = RwSignal::new(false);

    // 地形データがまだなら取得を始める(取得は`TerrainStore`が1回だけ行う)。
    terrain_store.ensure_loaded();

    // 原点(OriginState)が変わるたびに、まだ編集していなければ入力欄へ反映する。
    Effect::new(move |_| {
        if let Some(origin) = origin_state.0.get() {
            if !dirty.get_untracked() {
                lat_input.set(format!("{:.6}", origin.lat_deg));
                lon_input.set(format!("{:.6}", origin.lon_deg));
            }
        }
    });

    // 入力値を解析し、範囲チェックまで行う。Err内の文字列はそのままUIに表示するメッセージ。
    let parsed = Memo::new(move |_| -> Result<(f64, f64), String> {
        let number = |label: &str, input: RwSignal<String>| {
            input
                .get()
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("{label}は数値で入力してください"))
        };
        let in_range = |label: &str, v: f64, min: f64, max: f64| {
            if v < min || v > max {
                Err(format!(
                    "{label}は{min:.1}〜{max:.1}の範囲で入力してください"
                ))
            } else {
                Ok(v)
            }
        };
        let lat = number("緯度", lat_input)?;
        let lon = number("経度", lon_input)?;
        let Some(b) = bounds() else {
            return Err("地形データ範囲を取得中です...".to_string());
        };
        Ok((
            in_range("緯度", lat, b.min_lat, b.max_lat)?,
            in_range("経度", lon, b.min_lon, b.max_lon)?,
        ))
    });

    let on_click_submit = move |_| {
        if let Ok((lat, lon)) = parsed.get_untracked() {
            on_submit.run((lat, lon));
            dirty.set(false); // 送信後は次に届くOriginStateで表示を更新させる
        }
    };

    // 緯度・経度の入力欄。手で編集したら`dirty`にする。
    let coord_input = move |label: &'static str, input: RwSignal<String>| {
        view! {
            <label>
                {label}
                <input
                    type="text"
                    inputmode="decimal"
                    prop:value=move || input.get()
                    on:input=move |ev| {
                        dirty.set(true);
                        input.set(event_target_value(&ev));
                    }
                />
            </label>
        }
    };

    view! {
        <FloatingPanel open=dialog.0 title="原点設定">
            <div class="origin-form-row">
                {coord_input("緯度", lat_input)}
                {coord_input("経度", lon_input)}
                <button on:click=on_click_submit disabled=move || parsed.with(Result::is_err)>
                    "設定"
                </button>
            </div>
            {move || parsed.get().err().map(|msg| view! { <p class="field-error">{msg}</p> })}
        </FloatingPanel>
    }
}
