//! 汎用タブパネル。パネル見出し(`title`)の下にタブバーを表示し、選択中タブの内容を表示する。
//! 右パネル上段(トップステータスパネル)・下段(ボトムステータスパネル)が共通で使う。
//! 現状はどちらも1タブのみだが、後から同じ枠に別内容のタブを追加できるようにするための
//! 共通コンポーネントとして切り出してある。

use leptos::prelude::*;

pub struct Tab {
    label: &'static str,
    view: AnyView,
}

/// タブ1個分を作る。`view!{...}`の戻り値等、`IntoView`を実装する値なら何でも渡せる。
pub fn tab(label: &'static str, view: impl IntoView + 'static) -> Tab {
    Tab { label, view: view.into_any() }
}

#[component]
pub fn TabbedPanel(#[prop(into)] title: String, tabs: Vec<Tab>) -> impl IntoView {
    let active = RwSignal::new(0usize);
    let labels: Vec<&'static str> = tabs.iter().map(|t| t.label).collect();

    view! {
        <div class="panel-section tabbed-panel">
            <div class="tabbed-panel-header">
                <h2>{title}</h2>
                <div class="tab-bar">
                    {labels
                        .into_iter()
                        .enumerate()
                        .map(|(i, label)| {
                            view! {
                                <button
                                    class="tab-button"
                                    class:active=move || active.get() == i
                                    on:click=move |_| active.set(i)
                                >
                                    {label}
                                </button>
                            }
                        })
                        .collect::<Vec<_>>()}
                </div>
            </div>
            <div class="tabbed-panel-body">
                {tabs
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| {
                        view! {
                            <div
                                class="tab-content"
                                style:display=move || if active.get() == i { "flex" } else { "none" }
                            >
                                {t.view}
                            </div>
                        }
                    })
                    .collect::<Vec<_>>()}
            </div>
        </div>
    }
}
