//! 画面全体で共有するUI状態(メニュー操作で開閉するフローティングパネルなど)。

use leptos::prelude::*;

use crate::terrain::markers::RadarMarker;

/// 原点設定フローティングパネルの開閉状態。
/// `components/menu_bar.rs`(トリガー)と`components/origin_dialog.rs`(表示)で共有する。
#[derive(Clone, Copy)]
pub struct OriginDialogState(pub RwSignal<bool>);

/// メインパネル(3D地形)上への右クリックで追加するレーダー観測点(見通し範囲)の一覧・選択状態。
/// `components/terrain_view.rs`(追加・3D描画)・`components/los_view.rs`(一覧・編集・削除)・
/// `components/cross_section_view.rs`(断面図での覆域表示)で共有する。
#[derive(Clone, Copy)]
pub struct RadarMarkersState {
    pub markers: RwSignal<Vec<RadarMarker>>,
    pub selected: RwSignal<Option<u64>>,
    next_id: RwSignal<u64>,
}

impl RadarMarkersState {
    pub fn new() -> Self {
        Self {
            markers: RwSignal::new(Vec::new()),
            selected: RwSignal::new(None),
            next_id: RwSignal::new(1),
        }
    }

    /// 指定した緯度経度に既定パラメータ(アンテナ高10m・最大観測範囲50km)のレーダーを
    /// 追加し、選択状態にする。
    pub fn add(&self, lat_deg: f64, lon_deg: f64) -> u64 {
        let id = self.next_id.get_untracked();
        self.next_id.set(id + 1);
        self.markers.update(|list| {
            list.push(RadarMarker { id, lat_deg, lon_deg, height_m: 10.0, max_range_m: 50_000.0 });
        });
        self.selected.set(Some(id));
        id
    }

    pub fn remove(&self, id: u64) {
        self.markers.update(|list| list.retain(|m| m.id != id));
        if self.selected.get_untracked() == Some(id) {
            self.selected.set(None);
        }
    }
}

impl Default for RadarMarkersState {
    fn default() -> Self {
        Self::new()
    }
}
