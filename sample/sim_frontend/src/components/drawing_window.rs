//! 作図エディタ(`sim3dview::ui::drawing_editor::DrawingEditor`)を入れる、移動できるウインドウ。
//! 表示メニューの「作図...」で開く。地図をクリックして図形を置くので、背景を覆わない非モーダルの
//! `FloatingPanel`(`modal=false`)にして、タイトルバーのドラッグで動かせる(`draggable`)ようにしてある。

use leptos::prelude::*;

use sim3dview::ui::drawing_editor::DrawingEditor;
use sim3dview::ui::floating_panel::FloatingPanel;

/// 作図ウインドウの開閉状態。メニューバー(トリガー)と本体で共有する。
#[derive(Clone, Copy)]
pub struct DrawingWindowState(pub RwSignal<bool>);

#[component]
pub fn DrawingWindow() -> impl IntoView {
    let state = use_context::<DrawingWindowState>().expect("DrawingWindowState context not found");
    view! {
        <FloatingPanel open=state.0 title="作図" modal=false draggable=true initial_position=(360.0, 60.0)>
            <DrawingEditor/>
        </FloatingPanel>
    }
}
