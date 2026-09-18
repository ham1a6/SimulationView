//! 原点設定フローティングパネル。呼び出し側(アプリ)のメニュー等から
//! `OriginDialogState`を`true`にすることで開く。DETAILED_DESIGN.md 3.5節:
//! 地形データ範囲外の値はそもそも送信できないようにする(入力段階でブロック)。
//! 範囲(`geodetic_bounds`)はmetadata.jsonから取得する。
//!
//! 実際に緯度経度をどう配信するか(通信プロトコル)はアプリごとに異なるため、
//! ライブラリはそれを知らない。「設定」ボタンが押されたら`on_submit`コールバックを
//! 呼ぶだけで、呼び出し側がその中で自分のプロトコルに応じた送信を行う。

use leptos::prelude::*;

use super::floating_panel::FloatingPanel;
use crate::terrain::loader::{self, GeodeticBounds};
use crate::terrain::origin::OriginState;

/// 原点設定フローティングパネルの開閉状態。トリガー(メニュー等)と本体で共有する。
#[derive(Clone, Copy)]
pub struct OriginDialogState(pub RwSignal<bool>);

#[component]
pub fn OriginDialog(
    /// 地形データ配信のベースURL(`terrain::loader`参照)。`geodetic_bounds`の取得に使う。
    #[prop(into)] base_url: String,
    /// 「設定」ボタンで緯度経度が確定した際に呼ばれる。実際の送信方法は呼び出し側に委ねる。
    on_submit: Callback<(f64, f64)>,
) -> impl IntoView {
    let origin_state = use_context::<OriginState>().expect("OriginState context not found");
    let dialog = use_context::<OriginDialogState>().expect("OriginDialogState context not found");

    let bounds = RwSignal::new(None::<GeodeticBounds>);
    let lat_input = RwSignal::new(String::new());
    let lon_input = RwSignal::new(String::new());
    // ユーザーが手で編集を始めたら、OriginState受信による自動上書きを止める
    // (再配信で入力中の値が消えてしまうのを防ぐ)。
    let dirty = RwSignal::new(false);

    // 起動時に一度だけmetadata.jsonを取得してバリデーション用のgeodetic_boundsを得る。
    Effect::new(move |_| {
        if bounds.get_untracked().is_some() {
            return;
        }
        let base_url = base_url.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match loader::fetch_metadata(&base_url).await {
                Ok(meta) => bounds.set(Some(meta.geodetic_bounds)),
                Err(e) => log::error!("[origin_dialog] failed to fetch metadata.json: {e}"),
            }
        });
    });

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

    let on_click_submit = move |_| {
        if let Ok((lat, lon)) = parsed() {
            on_submit.run((lat, lon));
            dirty.set(false); // 送信後は次に届くOriginStateで表示を更新させる
        }
    };

    view! {
        <FloatingPanel open=dialog.0 title="原点設定">
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
                <button on:click=on_click_submit disabled=move || parsed().is_err()>
                    "設定"
                </button>
            </div>
            {move || parsed().err().map(|msg| view! { <p class="field-error">{msg}</p> })}
        </FloatingPanel>
    }
}
