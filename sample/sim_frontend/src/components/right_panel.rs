//! 右パネルの上段(トップステータスパネル)・下段(ボトムステータスパネル)。
//! どちらもsim3dviewライブラリの`TabbedPanel`(ui::tabbed_panel)で実装し、後から同じ枠に
//! 別内容のタブを追加できるようにしてある。現状のタブ構成:
//! トップ=「各種情報」(StatusPanelConfig駆動の数値一覧、このアプリ固有)+「航跡情報」(地図上でクリックして
//! 選択した航跡の詳細。`components/track_detail.rs`。航跡を選択すると自動でこのタブへ移る)、
//! ボトム=「断面図」(sim3dviewライブラリの`ui::cross_section_view::CrossSectionView`。
//! マップ原点を起点に方位角スライダーで指定した方向の地表断面と、レーダー覆域の
//! オーバーレイ)+「見通し範囲」(同ライブラリの`ui::los_view::LosView`。メインパネル上で
//! 右クリックして追加したレーダー観測点の一覧・選択・削除・パラメータ編集と、選択中レーダーの
//! 全方位角覆域図)。

use leptos::prelude::*;

use sim3dview::ui::cross_section_view::CrossSectionView;
use sim3dview::ui::los_view::LosView;
use sim3dview::ui::tabbed_panel::{tab, TabbedPanel};

use sim3dview::terrain::tracks::TracksState;

use crate::components::status_panel::StatusPanel;
use crate::components::track_detail::TrackDetail;

/// 「航跡情報」タブの番号(`tabs`の並びの2番目)。
const TRACK_DETAIL_TAB: usize = 1;

#[component]
pub fn TopStatusPanel() -> impl IntoView {
    let tracks = use_context::<TracksState>().expect("TracksState context not found");
    // 地図上で航跡が選択されたら、詳細のタブへ自動で移る(選択が外れてもタブはそのまま。手動で戻せる)。
    let active = RwSignal::new(0usize);
    Effect::new(move |_| {
        if tracks.selected.get().is_some() {
            active.set(TRACK_DETAIL_TAB);
        }
    });

    view! {
        <TabbedPanel
            active=active
            tabs=vec![
                tab("各種情報", view! { <StatusPanel/> }),
                tab("航跡情報", view! { <TrackDetail/> }),
            ]
        />
    }
}

#[component]
pub fn BottomStatusPanel() -> impl IntoView {
    view! {
        <TabbedPanel
            tabs=vec![
                tab("断面図", view! { <CrossSectionView/> }),
                tab("見通し範囲", view! { <LosView/> }),
            ]
        />
    }
}
