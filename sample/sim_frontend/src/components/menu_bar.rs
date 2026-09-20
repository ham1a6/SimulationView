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
//! ファイル/ヘルプは現時点では項目未定のプレースホルダ(クリックしても「準備中」の
//! ダミー項目が出るだけ)。メニューが開いている間は透明な全画面バックドロップを敷き、
//! そこをクリックすると閉じる(外側クリックで閉じる一般的なメニューの挙動)。

use leptos::prelude::*;

use sim3dview::terrain::drawing::DrawingState;
use sim3dview::terrain::hillshade::HillshadeState;
use sim3dview::terrain::origin::OriginState;
use sim3dview::terrain::origin_pick::OriginPickState;
use sim3dview::terrain::recenter::RecenterRequestState;
use sim3dview::ui::coverage_altitude_dialog::CoverageAltitudeDialogState;
use sim3dview::ui::origin_dialog::OriginDialogState;

use crate::components::drawing_demo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuId {
    File,
    Settings,
    View,
    Help,
}

#[component]
pub fn MenuBar() -> impl IntoView {
    let open_menu = RwSignal::new(None::<MenuId>);
    let origin_dialog =
        use_context::<OriginDialogState>().expect("OriginDialogState context not found");
    let origin_pick =
        use_context::<OriginPickState>().expect("OriginPickState context not found");
    let coverage_altitude_dialog = use_context::<CoverageAltitudeDialogState>()
        .expect("CoverageAltitudeDialogState context not found");
    let recenter_request =
        use_context::<RecenterRequestState>().expect("RecenterRequestState context not found");
    let hillshade = use_context::<HillshadeState>().expect("HillshadeState context not found");
    let drawings = use_context::<DrawingState>().expect("DrawingState context not found");
    let origin = use_context::<OriginState>().expect("OriginState context not found");
    // 「作図デモ」の表示中か(ライブラリの作図一覧を出し入れするだけ。ライブラリ側にこの状態はない)。
    let drawing_demo_on = RwSignal::new(false);

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
                                if drawing_demo_on.get_untracked() {
                                    drawings.clear();
                                    drawing_demo_on.set(false);
                                } else {
                                    let (lat, lon) = origin
                                        .0
                                        .get_untracked()
                                        .map(|o| (o.lat_deg, o.lon_deg))
                                        .unwrap_or((35.355556, 138.859722));
                                    drawing_demo::add_demo(drawings, lat, lon);
                                    drawing_demo_on.set(true);
                                }
                            }
                        >
                            {move || if drawing_demo_on.get() { "✓ 作図デモ" } else { "　 作図デモ" }}
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
