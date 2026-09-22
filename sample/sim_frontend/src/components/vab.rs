//! 左パネル下: VABパネル(操作ボタングリッド)。DETAILED_DESIGN.md 7.4節。
//! - 先頭行(B1〜B4相当)の配置・ラベル・有効/無効はサーバーの`VabConfig`が決める
//!   (ハードコードしない)。押すと通常のvab_pressコマンドを送るのに加え、
//!   「選択中カテゴリ」としてローカルに記憶する(カテゴリ選択タブとして扱う)
//! - 中段のページ数は、このコンポーネントを置く側が`mid_pages`で決める(1=単一ページ、任意のページ数)
//! - 中段・下段(サーバーVabConfigでのB5〜B24相当)は、**選択中カテゴリに応じて内容が
//!   動的に切り替わるフロント側だけのダミーコンテンツ**に置き換える(サーバーからの
//!   実際のボタン定義は使わない)。VAB自体がまだ実ハードウェア非連動の開発用ダミーの
//!   段階のため、「B1〜B4の押下でB5〜B24の表示・機能が変わる」「中段は追加ボタンを
//!   ページ送りボタンで操作できる」というUXをまずフロント側だけで試作したもの。
//!   サーバー側をカテゴリ対応させる(カテゴリごとに本物のVabConfigを配信する)のは
//!   将来の課題(CLAUDE.md参照)
//! - ラベルが空文字のボタン(先頭行側)は「未使用の穴」としてDOM要素自体を描画しない
//!   (グリッド位置がずれないよう、各ボタンにrow/columnを明示的に指定する)
//!
//! **有効化状態(`.vab-button-active`、通常状態の色を反転させた見た目)**: 「B1〜B4は
//! フロント側で現在の選択状況を有効化表示する」との要望により、先頭行は選択中カテゴリを
//! 有効化表示する(既存の`selected_category`をそのまま使う)。中段・下段は元々C++側の
//! ステータスで有効化を切り替える想定だったが、この2区画は現状C++側に存在自体を
//! 知らせていないダミーボタンのため紐づけようがなく、「フロントエンド側で制御する方針に
//! 変更する」との回答を受け、区画ごとに直近クリックしたボタンをローカルに記憶して
//! 有効化表示する(`selected_mid`/`selected_bottom`。カテゴリ切り替え時はどちらもリセット)。
//!
//! **スクリーンショット・画面録画ボタン**: マップパネル(地形canvas)をPNG保存/WebM録画する
//! ボタンを、中段の先頭2枠(既定のカテゴリ・1ページ目なら「B1-1」「B1-2」の位置。要望により
//! この位置に実装した)に固定で置く。通常の中段ダミーボタン(`mid_button`)を、この2枠だけ
//! 差し替える形(`vab_dummy_mid_0`/`_1`は送らない)。カテゴリやページを変えてもこの2枠だけは
//! 動かない(`i == 0`/`i == 1`、中段グリッドの絶対インデックスの先頭2つは、必ず1ページ目の
//! 先頭2列になるため)。サーバーへは何も送らない完全にフロント側だけの機能で、実処理は
//! sim3dviewライブラリ側(`terrain::capture::CaptureState`・`ui::terrain_view::TerrainView`)に
//! ある。このパネルはcontextへ要求を出すだけ。
//!
//! **シミュレーション開始/一時停止ボタン**: 上と同じ固定配置の考え方で、続く2枠
//! (`i == 2`/`i == 3`。既定なら「B1-3」「B1-4」の位置)を「開始」「一時停止」ボタンにする
//! (`SimulationStatusPanel`の同名ボタンと同じ`ClientCommand::resume`/`pause`を送るだけで、
//! 状態の押し分け表示は持たない。左パネル上の`SimulationStatusPanel`側のボタンは残したまま、
//! VABにも同じ操作口を増やす)。

use leptos::prelude::*;

use sim3dview::terrain::capture::CaptureState;

use crate::protocol::ClientCommand;
use crate::ws::WsHandle;
use crate::ws::WsSignals;

/// 中段(ページ送りする領域)の行数。先頭行(カテゴリ選択)・下段(固定4個)を除いた
/// 残りをこの行数として扱う(元のVabConfigのrowsから引くのではなく、ダミー表示専用の
/// 固定値。サーバーのVabConfigとは独立している)。
const MID_ROWS: usize = 4;
/// 中段のページ数の既定値(`VabPanel`の`mid_pages`を省略したとき)。
const DEFAULT_MID_PAGES: usize = 2;
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
    /// WebSocket接続。`WsConnection`はRc<RefCell<..>>を含みSend/Syncでないため、
    /// Copyのハンドル(`WsHandle`)にしてpropとして受け取る。
    conn: WsHandle,
    /// 中段(ダミーボタン)のページ数。1なら単一ページ(ページ送りボタンを出さない)、2以上ならその
    /// ページ数だけ「◀ 1/N ▶」で切り替える。1ページの列数は先頭行と同じ(サーバーのVabConfigの`cols`)で、
    /// 中段の総列数は`mid_pages * cols`になる。`Signal`を渡せば実行中に変えられる(0は1として扱う)。
    /// 省略時は2ページ。
    #[prop(into, default = DEFAULT_MID_PAGES.into())]
    mid_pages: Signal<usize>,
) -> impl IntoView {
    let signals = use_context::<WsSignals>().expect("WsSignals context not found");
    let capture = use_context::<CaptureState>().expect("CaptureState context not found");
    // 選択中カテゴリ(先頭行のうち何番目のボタンが押されたか、0始まり)。既定は先頭。
    let selected_category = RwSignal::new(0usize);
    // 中段の現在表示中ページ(0始まり)。カテゴリを切り替えたら先頭ページに戻す。
    let current_page = RwSignal::new(0usize);
    // 中段・下段それぞれの区画で直近にクリックされたダミーボタン(絶対インデックス、
    // mid_button/bottom_buttonの`index`引数と同じ体系)。有効化表示(色反転)に使う。
    // カテゴリを切り替えたらどちらもリセットする(前カテゴリでの押下状態を引き継がない)。
    let selected_mid = RwSignal::new(None::<usize>);
    let selected_bottom = RwSignal::new(None::<usize>);

    view! {
        <div class="panel-section vab-panel">
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
                                let on_click = move |_| {
                                    if enabled {
                                        conn.send_command(&ClientCommand::vab_press(button_id.clone()));
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
                                        {btn.label}
                                    </button>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </div>
                };

                // --- 中段: 選択中カテゴリのダミーボタン(ページ送りボタンで追加ボタンを見せる) ---
                // 1ページの列数は先頭行と同じ`cols`。横スクロールではなく、◀/▶ボタンで
                // ページを切り替える方式にした(要望により、横スクロールバー方式から変更)。
                let total_pages = mid_pages.get().max(1);
                // 中段の総列数(ページ数×1ページの列数)。ダミーボタンはこの列数ぶん並べる。
                let mid_total_cols = total_pages * cols;
                let page = current_page.get().min(total_pages - 1);
                let mid_grid_style =
                    format!("grid-template-columns: repeat({cols}, 1fr); grid-template-rows: repeat({MID_ROWS}, auto);");
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
                                            "スクショ"
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
                                            {move || if capture.is_recording.get() { "停止" } else { "録画" }}
                                        </button>
                                    }
                                        .into_any();
                                }
                                // 続く2枠(B1-3・B1-4の位置)は、シミュレーションの開始/一時停止
                                // (SimulationStatusPanelと同じコマンドを送るだけの操作口)。
                                if i == 2 {
                                    return view! {
                                        <button class="vab-button" on:click=move |_| conn.send_command(&ClientCommand::resume())>
                                            "開始"
                                        </button>
                                    }
                                        .into_any();
                                }
                                if i == 3 {
                                    return view! {
                                        <button class="vab-button" on:click=move |_| conn.send_command(&ClientCommand::pause())>
                                            "一時停止"
                                        </button>
                                    }
                                        .into_any();
                                }
                                let (label, id) = mid_button(&cat_for_mid, i);
                                let on_click = move |_| {
                                    conn.send_command(&ClientCommand::vab_press(id.clone()));
                                    selected_mid.set(Some(i));
                                };
                                let is_active = move || selected_mid.get() == Some(i);
                                view! {
                                    <button
                                        class="vab-button vab-button-dummy"
                                        class:vab-button-active=is_active
                                        on:click=on_click
                                    >
                                        {label}
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
                // 単一ページならページ送りは不要なので出さない。
                let pager = (total_pages > 1).then(|| view! {
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
                });
                let mid_block = view! {
                    <div class="vab-mid-block">
                        {mid_grid}
                        {pager}
                    </div>
                };

                // --- 下段: 選択中カテゴリのダミーボタン(固定4個、横スクロールなし) ---
                let bottom_grid_style = format!("grid-template-columns: repeat({BOTTOM_COLS}, 1fr);");
                let cat_for_bottom = category_label;
                let bottom_row = view! {
                    <div class="vab-grid vab-bottom-row" style=bottom_grid_style>
                        {(0..BOTTOM_COLS)
                            .map(|i| {
                                let (label, id) = bottom_button(&cat_for_bottom, i);
                                let on_click = move |_| {
                                    conn.send_command(&ClientCommand::vab_press(id.clone()));
                                    selected_bottom.set(Some(i));
                                };
                                let is_active = move || selected_bottom.get() == Some(i);
                                view! {
                                    <button
                                        class="vab-button vab-button-dummy"
                                        class:vab-button-active=is_active
                                        on:click=on_click
                                    >
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
