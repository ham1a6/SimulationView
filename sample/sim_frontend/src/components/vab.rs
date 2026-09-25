//! VABの配置・表示・操作はフロントで定義する。DETAILED_DESIGN.md 7.4節。
//! ローカル操作は通信せず、シミュレーション操作だけをコマンド送信する。

use leptos::prelude::*;

use sim3dview::terrain::capture::CaptureState;

use crate::protocol::ClientCommand;
use crate::ws::WsHandle;

/// カテゴリ定義。空ラベルは未使用の穴として表示しない。
const CATEGORIES: [(&str, bool); 4] = [("B1", true), ("B2", true), ("B3", true), ("B4", true)];
/// 中段の行数。
const MID_ROWS: usize = 4;
/// 中段のページ数の既定値(`VabPanel`の`mid_pages`を省略したとき)。
const DEFAULT_MID_PAGES: [usize; 4] = [1, 2, 2, 2];
/// 下段(固定・横スクロールなし)の列数。
const BOTTOM_COLS: usize = 4;

/// 文字の自然寸法を測り、ボタンの内側に等倍以下で収める。
/// 枠と文字の両方を監視するため、リサイズ・ラベル変更・フォント変更にも追従する。
#[component]
fn VabLabel(#[prop(into)] text: Signal<String>) -> impl IntoView {
    use wasm_bindgen::{closure::Closure, JsCast};

    let frame_ref = NodeRef::<leptos::html::Span>::new();
    let text_ref = NodeRef::<leptos::html::Span>::new();
    Effect::new(move |_| {
        let (Some(frame), Some(label)) = (frame_ref.get(), text_ref.get()) else {
            return;
        };
        let frame_for_resize = frame.clone();
        let label_for_resize = label.clone();
        let callback = Closure::<dyn FnMut(js_sys::Array)>::new(move |_| {
            // offset寸法の整数丸めで端が欠けないよう、文字側に1pxの余裕を含める。
            let width = label_for_resize.offset_width() as f64 + 1.0;
            let height = label_for_resize.offset_height() as f64 + 1.0;
            let frame_rect = frame_for_resize.get_bounding_client_rect();
            let scale = (frame_rect.width() / width)
                .min(frame_rect.height() / height)
                .min(1.0);
            let _ = label_for_resize.set_attribute(
                "style",
                &format!("transform: scale({scale}); visibility: visible;"),
            );
        });
        let observer = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref())
            .expect("VABラベルのサイズ監視を開始できません");
        observer.observe(&frame);
        observer.observe(&label);
        let resources = StoredValue::new_local((observer, callback));
        on_cleanup(move || resources.with_value(|(observer, _)| observer.disconnect()));
    });
    view! {
        <span class="vab-label-frame" node_ref=frame_ref title=move || text.get()>
            <span class="vab-label-text" node_ref=text_ref>{move || text.get()}</span>
        </span>
    }
}

#[component]
pub fn VabPanel(
    /// WebTransport接続。`WsConnection`はRc<RefCell<..>>を含みSend/Syncでないため、
    /// Copyのハンドル(`WsHandle`)にしてpropとして受け取る。
    conn: WsHandle,
    /// B1〜B4の順に指定する中段のページ数。「◀ 1/N ▶」は常に表示し、
    /// 1ページなら左右とも無効にする。1ページの列数は先頭行と同じ(`CATEGORIES`の要素数)で、
    /// 各カテゴリの総列数はページ数×列数。`Signal`で実行中にも変更できる(0は1扱い)。
    /// 省略時はB1が1ページ、B2〜B4が2ページ。
    #[prop(into, default = DEFAULT_MID_PAGES.into())]
    mid_pages: Signal<[usize; 4]>,
) -> impl IntoView {
    let capture = use_context::<CaptureState>().expect("CaptureState context not found");
    // 選択中カテゴリ(先頭行のうち何番目のボタンが押されたか、0始まり)。既定は先頭。
    let selected_category = RwSignal::new(0usize);
    // 中段の現在表示中ページ(0始まり)。カテゴリを切り替えたら先頭ページに戻す。
    let current_page = RwSignal::new(0usize);
    // 中段・下段それぞれの区画で直近にクリックされたダミーボタン(絶対インデックス、
    // グリッド内の位置)。有効化表示(色反転)に使う。
    // カテゴリを切り替えたらどちらもリセットする(前カテゴリでの押下状態を引き継がない)。
    let selected_mid = RwSignal::new(None::<usize>);
    let selected_bottom = RwSignal::new(None::<usize>);

    view! {
        <div class="panel-section vab-panel">
            {move || {
                let cols = CATEGORIES.len();
                let category_label = CATEGORIES[selected_category.get()].0.to_string();

                let top_grid_style =
                    format!("grid-template-columns: repeat({cols}, minmax(0, 1fr)); grid-template-rows: repeat(1, auto);");

                // --- 先頭行: ローカルのカテゴリ選択タブ ---
                let top_row = view! {
                    <div class="vab-grid vab-top-row" style=top_grid_style>
                        {CATEGORIES
                            .into_iter()
                            .enumerate()
                            .filter(|(_, (label, _))| !label.is_empty())
                            .map(|(i, (label, enabled))| {
                                let col = i + 1;
                                let pos_style = format!("grid-row: 1; grid-column: {col};");
                                let on_click = move |_| {
                                    if enabled {
                                        selected_category.set(i);
                                        current_page.set(0);
                                        // 前カテゴリでの中段・下段の有効化状態は引き継がない。
                                        selected_mid.set(None);
                                        selected_bottom.set(None);
                                    }
                                };
                                let is_selected = move || selected_category.get() == i;
                                view! {
                                    <button
                                        class="vab-button"
                                        class:vab-button-active=is_selected
                                        style=pos_style
                                        disabled=!enabled
                                        on:click=on_click
                                    >
                                        <VabLabel text=Signal::derive(move || label.to_string())/>
                                    </button>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </div>
                };

                // --- 中段: 選択中カテゴリのダミーボタン(ページ送りボタンで追加ボタンを見せる) ---
                // 1ページの列数は先頭行と同じ`cols`。横スクロールではなく、◀/▶ボタンで
                // ページを切り替える方式にした(要望により、横スクロールバー方式から変更)。
                let total_pages = mid_pages.get()[selected_category.get()].max(1);
                // 中段の総列数(ページ数×1ページの列数)。ダミーボタンはこの列数ぶん並べる。
                let mid_total_cols = total_pages * cols;
                let page = current_page.get().min(total_pages - 1);
                let mid_grid_style =
                    format!("grid-template-columns: repeat({cols}, minmax(0, 1fr)); grid-template-rows: repeat({MID_ROWS}, auto);");
                let cat_for_mid = category_label.clone();
                let mid_grid = view! {
                    <div class="vab-grid vab-mid-grid" style=mid_grid_style>
                        {(0..MID_ROWS)
                            .flat_map(|row| {
                                (0..cols).filter_map(move |col_in_page| {
                                    let abs_col = page * cols + col_in_page;
                                    (abs_col < mid_total_cols).then_some(row * mid_total_cols + abs_col)
                                })
                            })
                            .map(|i| {
                                // 中段の先頭2枠(B1-1・B1-2の位置)は、カテゴリ・ページによらず
                                // 常にスクリーンショット・画面録画ボタン(要望による固定配置)。
                                if i == 0 {
                                    return view! {
                                        <button class="vab-button" on:click=move |_| capture.request_screenshot()>
                                            <VabLabel text=Signal::derive(|| "スクショ".to_string())/>
                                        </button>
                                    }
                                        .into_any();
                                }
                                if i == 1 {
                                    return view! {
                                        <button
                                            class="vab-button"
                                            class:vab-button-active=move || capture.is_recording.get()
                                            on:click=move |_| capture.toggle_recording()
                                        >
                                            <VabLabel text=Signal::derive(move || if capture.is_recording.get() { "停止" } else { "録画" }.to_string())/>
                                        </button>
                                    }
                                        .into_any();
                                }
                                // 続く2枠(B1-3・B1-4の位置)は、シミュレーションの開始/一時停止
                                // サーバーへ送るのは画面のボタンIDではなく業務コマンド。
                                if i == 2 {
                                    return view! {
                                        <button
                                            class="vab-button"
                                            on:click=move |_| conn.send_command(&ClientCommand::resume())
                                            on:dblclick=move |_| conn.send_command(&ClientCommand::pause())
                                        >
                                            <VabLabel text=Signal::derive(|| "開始".to_string())/>
                                        </button>
                                    }
                                        .into_any();
                                }
                                if i == 3 {
                                    return view! {
                                        <button class="vab-button" on:click=move |_| conn.send_command(&ClientCommand::pause())>
                                            <VabLabel text=Signal::derive(|| "一時停止".to_string())/>
                                        </button>
                                    }
                                        .into_any();
                                }
                                let label = format!("{cat_for_mid}-{}", i + 1);
                                let on_click = move |_| {
                                    selected_mid.set(Some(i));
                                };
                                let is_active = move || selected_mid.get() == Some(i);
                                view! {
                                    <button
                                        class="vab-button vab-button-dummy"
                                        class:vab-button-active=is_active
                                        on:click=on_click
                                    >
                                        <VabLabel text=Signal::derive(move || label.to_string())/>
                                    </button>
                                }
                                    .into_any()
                            })
                            .collect::<Vec<_>>()}
                    </div>
                };

                // --- 中段のページ送りコントロール(◀ 1/2 ▶) ---
                let is_first_page = page == 0;
                let is_last_page = page + 1 >= total_pages;
                let on_prev_page = move |_: leptos::ev::MouseEvent| {
                    current_page.update(|p| *p = p.saturating_sub(1));
                };
                let on_next_page = move |_: leptos::ev::MouseEvent| {
                    current_page.update(|p| *p = (*p + 1).min(total_pages - 1));
                };
                // ページ数によらず表示し、移動できない方向のボタンを無効にする。
                let pager = view! {
                    <div class="vab-pager">
                        <button
                            class="vab-pager-btn"
                            disabled=is_first_page
                            on:click=on_prev_page
                        >
                            "◀"
                        </button>
                        <span class="vab-pager-label">{format!("{}/{}", page + 1, total_pages)}</span>
                        <button
                            class="vab-pager-btn"
                            disabled=is_last_page
                            on:click=on_next_page
                        >
                            "▶"
                        </button>
                    </div>
                };
                let mid_block = view! {
                    <div class="vab-mid-block">
                        {mid_grid}
                        {pager}
                    </div>
                };

                // --- 下段: 選択中カテゴリのダミーボタン(固定4個、横スクロールなし) ---
                let bottom_grid_style = format!("grid-template-columns: repeat({BOTTOM_COLS}, minmax(0, 1fr));");
                let cat_for_bottom = category_label;
                let bottom_row = view! {
                    <div class="vab-grid vab-bottom-row" style=bottom_grid_style>
                        {(0..BOTTOM_COLS)
                            .map(|i| {
                                let label = format!("{cat_for_bottom}A{}", i + 1);
                                let on_click = move |_| {
                                    selected_bottom.set(Some(i));
                                };
                                let is_active = move || selected_bottom.get() == Some(i);
                                view! {
                                    <button
                                        class="vab-button vab-button-dummy"
                                        class:vab-button-active=is_active
                                        on:click=on_click
                                    >
                                        <VabLabel text=Signal::derive(move || label.to_string())/>
                                    </button>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </div>
                };

                view! {
                    <div class="vab-panel-body">
                        {top_row}
                        {mid_block}
                        {bottom_row}
                    </div>
                }
                    .into_any()
            }}
        </div>
    }
}
