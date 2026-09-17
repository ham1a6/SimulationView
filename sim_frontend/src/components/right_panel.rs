//! 右パネルの上段(トップステータスパネル)・下段(ボトムステータスパネル)。
//! どちらも`TabbedPanel`(components/tabbed_panel.rs)で実装し、後から同じ枠に
//! 別内容のタブを追加できるようにしてある。現状のタブ構成:
//! トップ=「各種情報」(StatusPanelConfig駆動の数値一覧)、
//! ボトム=「断面図」(原点起点・方位角スライダーの2D地形断面)+
//! 「見通し範囲」(原点を観測点とした全方位角のレーダー覆域図)。

use leptos::prelude::*;

use crate::components::cross_section_view::CrossSectionView;
use crate::components::los_view::LosView;
use crate::components::status_panel::StatusPanel;
use crate::components::tabbed_panel::{tab, TabbedPanel};

#[component]
pub fn TopStatusPanel() -> impl IntoView {
    view! {
        <TabbedPanel
            title="トップステータスパネル"
            tabs=vec![tab("各種情報", view! { <StatusPanel/> })]
        />
    }
}

#[component]
pub fn BottomStatusPanel() -> impl IntoView {
    view! {
        <TabbedPanel
            title="ボトムステータスパネル"
            tabs=vec![
                tab("断面図", view! { <CrossSectionView/> }),
                tab("見通し範囲", view! { <LosView/> }),
            ]
        />
    }
}
