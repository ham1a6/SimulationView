//! 画面最上部のメニューバー。ファイル/設定/表示/ヘルプの4項目。
//! 「設定」→「原点をクリックで指定」は地図クリックによる原点指定モード
//! (`sim3dview::terrain::origin_pick::OriginPickState`)を有効にする。
//! 「設定」→「原点設定...」/「覆域高度設定...」を選ぶと、sim3dviewライブラリが提供する
//! フローティングパネル(`ui::origin_dialog`/`ui::coverage_altitude_dialog`)を開く
//! (パネル本体・開閉状態の型ともライブラリ側にあり、このメニューは開閉のトリガーのみ)。
//! 「表示」→「陰影表示」は、地形の陰影(ヒルシェード)のON/OFF(`terrain::hillshade::HillshadeState`。
//! ONのときは項目の頭に✓が付く)。「表示」→「中心点を原点に戻す」は、
//! 3DモードのShift+ドラッグ・2Dモードの通常ドラッグで動かせるカメラの注視点(中心点)を
//! シミュレーション原点の位置へ戻すボタン(`terrain::recenter::RecenterRequestState`
//! 経由の通知、`TerrainView`側で実際にリセットする。シミュレーション原点自体は変更しない)。
//! 「表示」→「航跡ラベル/航跡(軌跡)/高度線」は、航跡(`terrain::tracks::TracksState`)の表示設定のON/OFF、
//! 「3Dモデル...」は、航跡を3Dモデル(glTF)で描く方式の設定ウインドウ(`ui::model_settings_dialog`)を開く、
//! 「作図...」は、図形を作る移動可能なウインドウ(`components/drawing_window.rs`)を開く。
//! 「作図デモ」は作図(`terrain::drawing`)のデモ図形の表示/消去。
//! ファイル/ヘルプは現時点では項目未定のプレースホルダ(クリックしても「準備中」の
//! ダミー項目が出るだけ)。メニューが開いている間は透明な全画面バックドロップを敷き、
//! そこをクリックすると閉じる(外側クリックで閉じる一般的なメニューの挙動)。

use leptos::prelude::*;

use sim3dview::terrain::drawing::{DrawingId, DrawingState};
use sim3dview::terrain::hillshade::HillshadeState;
use sim3dview::terrain::origin::OriginState;
use sim3dview::terrain::origin_pick::OriginPickState;
use sim3dview::terrain::recenter::RecenterRequestState;
use sim3dview::terrain::tracks::TracksState;
use sim3dview::ui::coverage_altitude_dialog::CoverageAltitudeDialogState;
use sim3dview::ui::model_settings_dialog::ModelSettingsDialogState;
use sim3dview::ui::origin_dialog::OriginDialogState;

use crate::components::drawing_demo;
use crate::components::drawing_window::DrawingWindowState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuId {
    File,
    Settings,
    View,
    Help,
}

#[component]
pub fn MenuBar(show_left_panel: RwSignal<bool>, show_right_panel: RwSignal<bool>) -> impl IntoView {
    let open_menu = RwSignal::new(None::<MenuId>);
    let origin_dialog =
        use_context::<OriginDialogState>().expect("OriginDialogState context not found");
    let origin_pick = use_context::<OriginPickState>().expect("OriginPickState context not found");
    let coverage_altitude_dialog = use_context::<CoverageAltitudeDialogState>()
        .expect("CoverageAltitudeDialogState context not found");
    let recenter_request =
        use_context::<RecenterRequestState>().expect("RecenterRequestState context not found");
    let hillshade = use_context::<HillshadeState>().expect("HillshadeState context not found");
    let drawings = use_context::<DrawingState>().expect("DrawingState context not found");
    let drawing_window =
        use_context::<DrawingWindowState>().expect("DrawingWindowState context not found");
    let origin = use_context::<OriginState>().expect("OriginState context not found");
    let tracks = use_context::<TracksState>().expect("TracksState context not found");
    let model_settings = use_context::<ModelSettingsDialogState>()
        .expect("ModelSettingsDialogState context not found");
    // 「作図デモ」で追加した図形のID(表示中なら空でない。ライブラリの作図一覧を出し入れするだけで、
    // ライブラリ側にこの状態はない。消すときはこのIDだけを消し、ユーザーが作った図形は残す)。
    let drawing_demo_ids = RwSignal::new(Vec::<DrawingId>::new());

    let menu_entry = move |id: MenuId, label: &'static str| {
        let dropdown = move || {
            (open_menu.get() == Some(id)).then(move || match id {
                MenuId::Settings => view! {
                    <div class="menu-dropdown">
                        <button
                            class="menu-dropdown-item"
                            on:click=move |_| {
                                open_menu.set(None);
                                origin_dialog.0.set(true);
                            }
                        >
                            "原点設定..."
                        </button>
                        <button
                            class="menu-dropdown-item"
                            on:click=move |_| {
                                open_menu.set(None);
                                origin_pick.active.set(true);
                            }
                        >
                            "原点をクリックで指定"
                        </button>
                        <button
                            class="menu-dropdown-item"
                            on:click=move |_| {
                                open_menu.set(None);
                                coverage_altitude_dialog.0.set(true);
                            }
                        >
                            "覆域高度設定..."
                        </button>
                    </div>
                }
                .into_any(),
                MenuId::View => view! {
                    <div class="menu-dropdown">
                        <button
                            class="menu-dropdown-item"
                            role="menuitemcheckbox"
                            aria-checked=move || show_left_panel.get().to_string()
                            on:click=move |_| {
                                open_menu.set(None);
                                show_left_panel.update(|on| *on = !*on);
                            }
                        >
                            {move || if show_left_panel.get() { "✓ 左ステータスパネル" } else { "　 左ステータスパネル" }}
                        </button>
                        <button
                            class="menu-dropdown-item"
                            role="menuitemcheckbox"
                            aria-checked=move || show_right_panel.get().to_string()
                            on:click=move |_| {
                                open_menu.set(None);
                                show_right_panel.update(|on| *on = !*on);
                            }
                        >
                            {move || if show_right_panel.get() { "✓ 右ステータスパネル" } else { "　 右ステータスパネル" }}
                        </button>
                        <button
                            class="menu-dropdown-item"
                            on:click=move |_| {
                                open_menu.set(None);
                                hillshade.enabled.update(|on| *on = !*on);
                            }
                        >
                            {move || if hillshade.enabled.get() { "✓ 陰影表示" } else { "　 陰影表示" }}
                        </button>
                        <button
                            class="menu-dropdown-item"
                            on:click=move |_| {
                                open_menu.set(None);
                                recenter_request.request();
                            }
                        >
                            "中心点を原点に戻す"
                        </button>
                        <button
                            class="menu-dropdown-item"
                            on:click=move |_| {
                                open_menu.set(None);
                                tracks.show_labels.update(|on| *on = !*on);
                            }
                        >
                            {move || if tracks.show_labels.get() { "✓ 航跡ラベル" } else { "　 航跡ラベル" }}
                        </button>
                        <button
                            class="menu-dropdown-item"
                            on:click=move |_| {
                                open_menu.set(None);
                                tracks.show_trails.update(|on| *on = !*on);
                            }
                        >
                            {move || if tracks.show_trails.get() { "✓ 航跡(軌跡)" } else { "　 航跡(軌跡)" }}
                        </button>
                        <button
                            class="menu-dropdown-item"
                            on:click=move |_| {
                                open_menu.set(None);
                                tracks.show_altitude_lines.update(|on| *on = !*on);
                            }
                        >
                            {move || if tracks.show_altitude_lines.get() { "✓ 高度線" } else { "　 高度線" }}
                        </button>
                        <button
                            class="menu-dropdown-item"
                            on:click=move |_| {
                                open_menu.set(None);
                                model_settings.0.set(true);
                            }
                        >
                            "3Dモデル..."
                        </button>
                        <button
                            class="menu-dropdown-item"
                            on:click=move |_| {
                                open_menu.set(None);
                                drawing_window.0.set(true);
                            }
                        >
                            "作図..."
                        </button>
                        <button
                            class="menu-dropdown-item"
                            on:click=move |_| {
                                open_menu.set(None);
                                let shown = drawing_demo_ids.get_untracked();
                                if !shown.is_empty() {
                                    for id in shown {
                                        drawings.remove(id);
                                    }
                                    drawing_demo_ids.set(Vec::new());
                                } else {
                                    let (lat, lon) = origin
                                        .0
                                        .get_untracked()
                                        .map(|o| (o.lat_deg, o.lon_deg))
                                        .unwrap_or((35.355556, 138.859722));
                                    drawing_demo_ids.set(drawing_demo::add_demo(drawings, lat, lon));
                                }
                            }
                        >
                            {move || if !drawing_demo_ids.with(|ids| ids.is_empty()) { "✓ 作図デモ" } else { "　 作図デモ" }}
                        </button>
                    </div>
                }
                .into_any(),
                _ => view! {
                    <div class="menu-dropdown">
                        <span class="menu-dropdown-item menu-dropdown-item--disabled">
                            "(準備中)"
                        </span>
                    </div>
                }
                .into_any(),
            })
        };

        view! {
            <div class="menu-item">
                <button
                    class="menu-button"
                    class:active=move || open_menu.get() == Some(id)
                    on:click=move |_| {
                        open_menu.update(|m| *m = if *m == Some(id) { None } else { Some(id) });
                    }
                >
                    {label}
                </button>
                {dropdown}
            </div>
        }
    };

    view! {
        <nav class="menu-bar">
            {menu_entry(MenuId::File, "ファイル")}
            {menu_entry(MenuId::Settings, "設定")}
            {menu_entry(MenuId::View, "表示")}
            {menu_entry(MenuId::Help, "ヘルプ")}
            {move || {
                open_menu
                    .get()
                    .is_some()
                    .then(|| view! { <div class="menu-backdrop" on:click=move |_| open_menu.set(None)></div> })
            }}
        </nav>
    }
}
