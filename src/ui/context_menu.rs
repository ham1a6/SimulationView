//! 汎用の右クリックメニュー(コンテキストメニュー)。DETAILED_DESIGN.md 7.7節。
//!
//! アプリは`ContextMenuState`を`provide_context`し、`ContextMenu`をどこかに1つ置く(画面全体に重なるので位置はどこでもよい)。
//! メニューを出したいところ(右クリックのハンドラ)から`ContextMenuState::show(x, y, items)`を呼ぶと、
//! 画面座標(client座標)`(x, y)`の位置にメニューが出る。項目(`MenuItem`)は出す側が渡す。
//! `FloatingPanel`と同じく、本体(見た目・開閉・位置補正)だけをライブラリが持ち、中身は使う側が決める。
//!
//! - 項目の種類: 操作(`action`。無効にもできる)・サブメニュー(`submenu`。入れ子にできる)・見出し(`label`。押せない)・区切り線(`separator`)
//! - 項目を選ぶと、メニューを閉じてから`on_select`を呼ぶ。メニューの外(背景)のクリック・右クリック・Escでも閉じる
//! - 画面の右端・下端にはみ出すときは、収まるようにずらす。サブメニューは右へ、収まらなければ左へ開く
//! - 地図(`TerrainView`)の右クリックにつなぐには、`ui::terrain_view::MapMenuState`を使う

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::floating_panel::viewport_size;
use crate::terrain::tracks::TrackId;

/// メニューを画面の端からこれだけ(px)離す。
const EDGE_MARGIN_PX: f64 = 4.0;
/// サブメニューの幅の見積もり(px)。右に収まるかの判定に使う。
const SUBMENU_WIDTH_ESTIMATE_PX: f64 = 240.0;

/// メニューの項目1つ。
#[derive(Clone)]
pub enum MenuItem {
    /// 押せる項目。`enabled`がfalseなら灰色で押せない。
    Action {
        label: String,
        enabled: bool,
        on_select: UnsyncCallback<()>,
    },
    /// 子のメニューを右に開く項目。
    Submenu {
        label: String,
        items: Vec<MenuItem>,
    },
    /// 押せない見出し・情報の行。
    Label(String),
    Separator,
}

impl MenuItem {
    /// 押せる項目。選ばれたら`on_select`を呼ぶ(メニューは先に閉じる)。
    pub fn action(label: impl Into<String>, on_select: impl Fn() + 'static) -> Self {
        Self::Action {
            label: label.into(),
            enabled: true,
            on_select: UnsyncCallback::new(move |()| on_select()),
        }
    }

    /// 灰色で押せない項目(機能はあるが、いまは使えないことを見せたいとき)。
    pub fn disabled(label: impl Into<String>) -> Self {
        Self::Action {
            label: label.into(),
            enabled: false,
            on_select: UnsyncCallback::new(|()| {}),
        }
    }

    /// `enabled`がfalseなら押せなくする(操作の項目だけに効く)。
    pub fn enabled(mut self, enabled: bool) -> Self {
        if let Self::Action { enabled: e, .. } = &mut self {
            *e = enabled;
        }
        self
    }

    pub fn submenu(label: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self::Submenu {
            label: label.into(),
            items,
        }
    }

    pub fn label(text: impl Into<String>) -> Self {
        Self::Label(text.into())
    }

    pub fn separator() -> Self {
        Self::Separator
    }
}

/// いま開いているメニュー(位置と項目)。
#[derive(Clone)]
struct OpenMenu {
    x: f64,
    y: f64,
    items: Vec<MenuItem>,
}

/// 右クリックメニューの状態。`Copy`なので、そのままクロージャへ持ち込める。
#[derive(Clone, Copy)]
pub struct ContextMenuState {
    open: RwSignal<Option<OpenMenu>>,
}

impl ContextMenuState {
    pub fn new() -> Self {
        Self {
            open: RwSignal::new(None),
        }
    }

    /// 画面座標(client座標)`(x, y)`にメニューを出す。すでに開いていれば置き換える。項目が空なら何もしない。
    pub fn show(&self, x: f64, y: f64, items: Vec<MenuItem>) {
        if !items.is_empty() {
            self.open.set(Some(OpenMenu { x, y, items }));
        }
    }

    pub fn close(&self) {
        if self.open.get_untracked().is_some() {
            self.open.set(None);
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.with(|m| m.is_some())
    }
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self::new()
    }
}

/// メニュー1段分(と、開いているサブメニュー)を作る。`open_sub`は、この段で開いているサブメニューの番号。
fn menu_view(items: Vec<MenuItem>, state: ContextMenuState) -> AnyView {
    let open_sub = RwSignal::new(None::<usize>);
    items
        .into_iter()
        .enumerate()
        .map(|(i, item)| match item {
            MenuItem::Separator => view! { <div class="context-menu-separator"></div> }.into_any(),
            MenuItem::Label(text) => view! { <div class="context-menu-label">{text}</div> }.into_any(),
            MenuItem::Action { label, enabled, on_select } => {
                let disabled = !enabled;
                view! {
                    <button
                        class="context-menu-item"
                        disabled=disabled
                        on:mouseenter=move |_| open_sub.set(None)
                        on:click=move |_| {
                            // 先に閉じてから実行する(実行した先で別のメニューを出せるように)。
                            state.close();
                            on_select.run(());
                        }
                    >
                        {label}
                    </button>
                }
                .into_any()
            }
            MenuItem::Submenu { label, items } => {
                // 右に収まらなければ左へ開く(項目の上へカーソルが来たときに判定する)。
                let flip = RwSignal::new(false);
                let on_enter = move |ev: leptos::ev::MouseEvent| {
                    if let Some(el) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                        let (vw, _) = viewport_size();
                        flip.set(el.get_bounding_client_rect().right() + SUBMENU_WIDTH_ESTIMATE_PX > vw);
                    }
                    open_sub.set(Some(i));
                };
                view! {
                    <div class="context-menu-parent" on:mouseenter=on_enter>
                        <button class="context-menu-item context-menu-item--submenu" on:click=move |_| open_sub.set(Some(i))>
                            <span>{label}</span>
                            <span class="context-menu-arrow">"\u{25B6}"</span>
                        </button>
                        {move || {
                            (open_sub.get() == Some(i)).then(|| {
                                view! {
                                    <div class="context-menu context-menu-sub" class:flip=move || flip.get()>
                                        {menu_view(items.clone(), state)}
                                    </div>
                                }
                            })
                        }}
                    </div>
                }
                .into_any()
            }
        })
        .collect_view()
        .into_any()
}

/// 右クリックメニューの本体。`ContextMenuState`(context)を見て、開いていれば描く。アプリは1つだけ置く。
#[component]
pub fn ContextMenu() -> impl IntoView {
    let state = use_context::<ContextMenuState>().expect("ContextMenuState context not found");
    let menu_ref: NodeRef<leptos::html::Div> = NodeRef::new();
    // 表示位置。開いた直後は指定された位置にして、描いたあとで画面内へ収める。
    let pos = RwSignal::new((0.0_f64, 0.0_f64));
    // 位置を収め終わるまでは見せない(指定位置から一瞬ずれて見えるのを防ぐ)。
    let placed = RwSignal::new(false);

    Effect::new(move |_| {
        let Some(menu) = state.open.get() else { return };
        pos.set((menu.x, menu.y));
        placed.set(false);
        request_animation_frame(move || {
            let Some(el) = menu_ref.get_untracked() else {
                placed.set(true);
                return;
            };
            let rect = el.get_bounding_client_rect();
            let (vw, vh) = viewport_size();
            let x = if rect.right() > vw - EDGE_MARGIN_PX {
                (vw - rect.width() - EDGE_MARGIN_PX).max(0.0)
            } else {
                rect.left()
            };
            let y = if rect.bottom() > vh - EDGE_MARGIN_PX {
                (vh - rect.height() - EDGE_MARGIN_PX).max(0.0)
            } else {
                rect.top()
            };
            pos.set((x, y));
            placed.set(true);
        });
    });

    // Escで閉じる。コンポーネントが破棄されたらリスナーを外す。
    let keydown_handle =
        window_event_listener(leptos::ev::keydown, move |ev: web_sys::KeyboardEvent| {
            if ev.key() == "Escape" && state.is_open() {
                state.close();
            }
        });
    on_cleanup(move || keydown_handle.remove());

    view! {
        {move || {
            state.open.get().map(|menu| {
                view! {
                    // 背景(画面全体)。メニューの外を左/右クリックしたら閉じる(そのクリックは背後へ通さない)。
                    <div
                        class="context-menu-backdrop"
                        on:pointerdown=move |_| state.close()
                        on:contextmenu=move |ev| {
                            ev.prevent_default();
                            state.close();
                        }
                    >
                        <div
                            class="context-menu"
                            node_ref=menu_ref
                            style:left=move || format!("{}px", pos.get().0)
                            style:top=move || format!("{}px", pos.get().1)
                            style:visibility=move || if placed.get() { "visible" } else { "hidden" }
                            on:pointerdown=|ev| ev.stop_propagation()
                            on:contextmenu=|ev| {
                                ev.prevent_default();
                                ev.stop_propagation();
                            }
                        >
                            {menu_view(menu.items, state)}
                        </div>
                    </div>
                }
            })
        }}
    }
}

// ---------------------------------------------------------------------------------------------
// 地図の右クリック(`TerrainView`が使う)
// ---------------------------------------------------------------------------------------------

/// 地図を右クリックした場所にあるもの(右クリックメニューの項目を決める材料)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapMenuTarget {
    /// 地表の(緯度, 経度)。地形データの範囲外・空ならNone。
    pub position: Option<(f64, f64)>,
    /// 右クリックした航跡のシンボル。あればTerrainViewが先にそのトラックを選択する。
    pub track: Option<TrackId>,
}

/// 地図の右クリックメニューの項目を作るコールバック(`provide_context`する。`ContextMenuState`も必要)。
/// 右クリックのたびに、その場所(`MapMenuTarget`)から項目を作って返す。空を返せばメニューは出ない。
/// レーダー観測点の追加・原点の指定・作図の開始など、何を並べるかはアプリが決める。
#[derive(Clone, Copy)]
pub struct MapMenuState(pub UnsyncCallback<MapMenuTarget, Vec<MenuItem>>);

impl MapMenuState {
    pub fn new(build: impl Fn(MapMenuTarget) -> Vec<MenuItem> + 'static) -> Self {
        Self(UnsyncCallback::new(build))
    }
}
