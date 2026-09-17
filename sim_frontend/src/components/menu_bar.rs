//! 画面最上部のメニューバー。ファイル/設定/表示/ヘルプの4項目。
//! 「設定」→「原点設定...」を選ぶと、原点設定用フローティングパネル(`origin_dialog.rs`)を開く。
//! ファイル/表示/ヘルプは現時点では項目未定のプレースホルダ(クリックしても「準備中」の
//! ダミー項目が出るだけ)。メニューが開いている間は透明な全画面バックドロップを敷き、
//! そこをクリックすると閉じる(外側クリックで閉じる一般的なメニューの挙動)。

use leptos::prelude::*;

use crate::ui_state::OriginDialogState;

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
