//! 左パネル下: VABパネル(操作ボタングリッド)。DETAILED_DESIGN.md 7.4節。
//! - 先頭行(B1〜B4相当)の配置・ラベル・有効/無効はサーバーの`VabConfig`が決める
//!   (ハードコードしない)。押すと通常のvab_pressコマンドを送るのに加え、
//!   「選択中カテゴリ」としてローカルに記憶する(カテゴリ選択タブとして扱う)
//! - 中段・下段(サーバーVabConfigでのB5〜B24相当)は、**選択中カテゴリに応じて内容が
//!   動的に切り替わるフロント側だけのダミーコンテンツ**に置き換える(サーバーからの
//!   実際のボタン定義は使わない)。VAB自体がまだ実ハードウェア非連動の開発用ダミーの
//!   段階のため、「B1〜B4の押下でB5〜B24の表示・機能が変わる」「中段は横スクロールで
//!   追加ボタンを操作できる」というUXをまずフロント側だけで試作したもの。
//!   サーバー側をカテゴリ対応させる(カテゴリごとに本物のVabConfigを配信する)のは
//!   将来の課題(CLAUDE.md参照)
//! - ラベルが空文字のボタン(先頭行側)は「未使用の穴」としてDOM要素自体を描画しない
//!   (グリッド位置がずれないよう、各ボタンにrow/columnを明示的に指定する)

use leptos::prelude::*;

use crate::protocol::ClientCommand;
use crate::ws::WsConnection;
use crate::ws::WsSignals;

/// 中段(横スクロールする領域)の行数。先頭行(カテゴリ選択)・下段(固定4個)を除いた
/// 残りをこの行数として扱う(元のVabConfigのrowsから引くのではなく、ダミー表示専用の
/// 固定値。サーバーのVabConfigとは独立している)。
const MID_ROWS: usize = 4;
/// 中段の実際の列数(横スクロールで見せる分、可視列数より広くしてスクロールを試作できるようにする)。
const MID_TOTAL_COLS: usize = 8;
/// 下段(固定・横スクロールなし)の列数。
const BOTTOM_COLS: usize = 4;

/// 選択中カテゴリの中段ボタン1個ぶんのダミーラベル・id。
fn mid_button(category_label: &str, index: usize) -> (String, String) {
    (format!("{category_label}-{}", index + 1), format!("vab_dummy_mid_{index}"))
}

/// 選択中カテゴリの下段ボタン1個ぶんのダミーラベル・id。
fn bottom_button(category_label: &str, index: usize) -> (String, String) {
    (format!("{category_label}A{}", index + 1), format!("vab_dummy_bottom_{index}"))
}

#[component]
pub fn VabPanel(
    /// WsConnectionはRc<RefCell<..>>を含みSend/Syncでないため、
    /// (Leptos 0.8のprovide_contextが要求する境界を満たせない)、propとして受け取る。
    conn: WsConnection,
) -> impl IntoView {
    let signals = use_context::<WsSignals>().expect("WsSignals context not found");
    // 選択中カテゴリ(先頭行のうち何番目のボタンが押されたか、0始まり)。既定は先頭。
    let selected_category = RwSignal::new(0usize);

    view! {
        <div class="panel-section vab-panel">
            <h2>"VABパネル"</h2>
            {move || {
                let Some(cfg) = signals.vab_config.get() else {
                    return view! { <p class="placeholder">"(未受信)"</p> }.into_any();
                };
                let cols = cfg.cols.max(1) as usize;
                let top_row_buttons: Vec<_> = cfg.buttons.iter().take(cols).cloned().collect();
                let category_label = top_row_buttons
                    .get(selected_category.get().min(top_row_buttons.len().saturating_sub(1)))
                    .map(|b| b.label.clone())
                    .filter(|l| !l.is_empty())
                    .unwrap_or_else(|| "?".to_string());

                let top_grid_style =
                    format!("grid-template-columns: repeat({cols}, 1fr); grid-template-rows: repeat(1, auto);");
                let conn_top = conn.clone();

                // --- 先頭行: カテゴリ選択タブ(サーバーVabConfig駆動、従来通りvab_pressも送る) ---
                let top_row = view! {
                    <div class="vab-grid vab-top-row" style=top_grid_style>
                        {top_row_buttons
                            .into_iter()
                            .enumerate()
                            .filter(|(_, btn)| !btn.label.is_empty())
                            .map(|(i, btn)| {
                                let col = i + 1;
                                let pos_style = format!("grid-row: 1; grid-column: {col};");
                                let button_id = btn.id.clone();
                                let enabled = btn.enabled;
                                let conn = conn_top.clone();
                                let on_click = move |_| {
                                    if enabled {
                                        conn.send_command(&ClientCommand::vab_press(button_id.clone()));
                                        selected_category.set(i);
                                    }
                                };
                                let is_selected = move || selected_category.get() == i;
                                view! {
                                    <button
                                        class="vab-button"
                                        class:vab-button-selected=is_selected
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
                };

                // --- 中段: 選択中カテゴリのダミーボタン(横スクロールで追加ボタンを見せる) ---
                let mid_grid_style = format!(
                    "grid-template-columns: repeat({MID_TOTAL_COLS}, 1fr); \
                     grid-template-rows: repeat({MID_ROWS}, auto); \
                     min-width: calc(100% * {MID_TOTAL_COLS} / {cols});"
                );
                let conn_mid = conn.clone();
                let cat_for_mid = category_label.clone();
                let mid_block = view! {
                    <div class="vab-scroll-outer">
                        <div class="vab-grid vab-mid-grid" style=mid_grid_style>
                            {(0..MID_ROWS * MID_TOTAL_COLS)
                                .map(|i| {
                                    let (label, id) = mid_button(&cat_for_mid, i);
                                    let conn = conn_mid.clone();
                                    let on_click = move |_| {
                                        conn.send_command(&ClientCommand::vab_press(id.clone()));
                                    };
                                    view! {
                                        <button class="vab-button vab-button-dummy" on:click=on_click>
                                            {label}
                                        </button>
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </div>
                    </div>
                };

                // --- 下段: 選択中カテゴリのダミーボタン(固定4個、横スクロールなし) ---
                let bottom_grid_style = format!("grid-template-columns: repeat({BOTTOM_COLS}, 1fr);");
                let conn_bottom = conn.clone();
                let cat_for_bottom = category_label;
                let bottom_row = view! {
                    <div class="vab-grid vab-bottom-row" style=bottom_grid_style>
                        {(0..BOTTOM_COLS)
                            .map(|i| {
                                let (label, id) = bottom_button(&cat_for_bottom, i);
                                let conn = conn_bottom.clone();
                                let on_click = move |_| {
                                    conn.send_command(&ClientCommand::vab_press(id.clone()));
                                };
                                view! {
                                    <button class="vab-button vab-button-dummy" on:click=on_click>
                                        {label}
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
