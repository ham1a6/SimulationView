//! 右パネルの上段(トップステータスパネル)・下段(ボトムステータスパネル)。
//! どちらもsim3dviewライブラリの`TabbedPanel`(ui::tabbed_panel)で実装し、後から同じ枠に
//! 別内容のタブを追加できるようにしてある。現状のタブ構成:
//! トップ=「各種情報」(StatusPanelConfig駆動の数値一覧、このアプリ固有)、
//! ボトム=「見通し範囲」(sim3dviewライブラリの`ui::los_view::LosView`。メインパネル上で
//! 右クリックして追加したレーダー観測点の一覧・選択・削除・パラメータ編集と、選択中レーダーの
//! 全方位角覆域図)。
//! 断面図(旧称「側面図」)タブは「ボトムステータスパネルの側面図もいらない」との
//! 要望により削除した(`components/cross_section_view.rs`ごと削除。git履歴参照)。

use leptos::prelude::*;

use sim3dview::ui::los_view::LosView;
use sim3dview::ui::tabbed_panel::{tab, TabbedPanel};

use crate::components::status_panel::StatusPanel;

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
            tabs=vec![tab("見通し範囲", view! { <LosView/> })]
        />
    }
}
