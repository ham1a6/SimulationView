//! 地図の初期化が終わるまで中央に出す状態表示(読み込み中のプログレスバー・エラー文言)。

use leptos::prelude::*;

use crate::terrain::store::TerrainStore;

/// レンダラーの初期化(`frame::try_init`)の状態。
#[derive(Clone, Debug, PartialEq)]
pub(super) enum InitStatus {
    /// 地形データの取得中、またはレンダラーの準備中。
    Pending,
    /// レンダラーの初期化が終わり、地形を描いている。
    Ready,
    /// レンダラー(wgpu)の初期化に失敗した(中身はエラーの文言)。
    Failed(String),
}

/// 初期化が終わるまでの状態表示。`Pending`の間は、地形データの取得中なら受信の割合、
/// 取得後(レンダラー・メッシュの準備中)なら動き続けるバーを出す。
#[component]
pub(super) fn MapStatus(init: RwSignal<InitStatus>, store: TerrainStore) -> impl IntoView {
    move || match init.get() {
        InitStatus::Ready => None,
        InitStatus::Failed(e) => Some(error_text(format!("地形描画エラー: {e}"))),
        InitStatus::Pending => Some(match store.error.get() {
            Some(e) => error_text(format!("地形データの取得に失敗しました: {e}")),
            None => view! { <LoadingProgress store=store/> }.into_any(),
        }),
    }
}

/// エラーの文言を、状態表示の位置に出す要素。
fn error_text(text: String) -> AnyView {
    view! { <p class="placeholder map-status">{text}</p> }.into_any()
}

/// 地形データの読み込みのプログレスバー(割合のラベル・受信量のMB表示つき)。
#[component]
fn LoadingProgress(store: TerrainStore) -> impl IntoView {
    let downloaded = move || store.get().is_some();
    // 取得後の準備中と、全体の大きさ(metadata・索引)がまだ分からない間は割合を出さない(動き続けるバー)。
    let percent = move || {
        if downloaded() {
            return None;
        }
        store.progress().fraction().map(|f| (f * 100.0).floor())
    };
    let label = move || match (downloaded(), percent()) {
        (true, _) => "地形を表示する準備中...".to_string(),
        (false, Some(p)) => format!("地形データを読み込み中... {p}%"),
        (false, None) => "地形データを読み込み中...".to_string(),
    };
    let detail = move || {
        let p = store.progress();
        let mib = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
        match p.total_bytes {
            Some(total) if !downloaded() => format!(
                "{:.1} / {:.1} MB",
                mib(p.received_bytes.min(total)),
                mib(total)
            ),
            _ => String::new(),
        }
    };
    // 割合が分からない間はCSSで左右に動き続けるバー(`indeterminate`)にする。
    let indeterminate = move || percent().is_none();
    let fill_width = move || percent().map(|p| format!("{p}%"));
    view! {
        <div class="map-status map-loading" role="status">
            <div class="map-loading-label">{label}</div>
            <div
                class="map-loading-bar"
                class:indeterminate=indeterminate
                role="progressbar"
                aria-label="地形の読み込み"
                aria-valuemin="0"
                aria-valuemax="100"
                aria-valuenow=percent
            >
                <div class="map-loading-fill" style:width=fill_width></div>
            </div>
            <div class="map-loading-detail">{detail}</div>
        </div>
    }
}
