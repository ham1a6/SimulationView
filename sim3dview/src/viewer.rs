//! ビューア状態をまとめて初期化する任意の補助API。
//! LeptosのOwner内で生成し、子ビューを作る前に`provide`を1回呼ぶ。
//! 通信・メニュー構成・ダイアログの開閉はアプリ側で設定する。

use crate::terrain::{
    capture::CaptureState, draw_tool::DrawToolState, drawing::DrawingState,
    hillshade::HillshadeState, markers::RadarMarkersState, models::ModelsState,
    origin::OriginState, recenter::RecenterRequestState, store::TerrainStore, tracks::TracksState,
};
use leptos::prelude::*;

/// 全機能で共有する状態。従来の個別context登録も引き続き使用できる。
#[derive(Clone, Copy)]
pub struct ViewerState {
    pub terrain: TerrainStore,
    pub origin: OriginState,
    pub radar_markers: RadarMarkersState,
    pub recenter: RecenterRequestState,
    pub hillshade: HillshadeState,
    pub capture: CaptureState,
    pub drawings: DrawingState,
    pub draw_tool: DrawToolState,
    pub tracks: TracksState,
    pub models: ModelsState,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::{draw_tool::ToolKind, origin::Origin};

    #[test]
    fn provided_states_share_updates_and_drawing_storage() {
        let owner = Owner::new();
        owner.with(|| {
            let viewer = ViewerState::new("/terrain").provide();
            let origin = use_context::<OriginState>().unwrap();
            viewer.origin.0.set(Some(Origin {
                lat_deg: 35.0,
                lon_deg: 139.0,
            }));
            assert_eq!(origin.0.get_untracked().unwrap().lat_deg, 35.0);
            let tool = use_context::<DrawToolState>().unwrap();
            tool.start_at(ToolKind::Circle, 35.0, 139.0);
            tool.click(35.01, 139.0);
            assert!(!viewer.drawings.items.get_untracked().is_empty());
            assert_eq!(
                viewer.drawings.items.get_untracked().len(),
                use_context::<DrawingState>()
                    .unwrap()
                    .items
                    .get_untracked()
                    .len()
            );
            viewer.tracks.show_labels.set(false);
            assert!(!use_context::<TracksState>()
                .unwrap()
                .show_labels
                .get_untracked());
            assert!(use_context::<TerrainStore>().is_some());
            assert!(use_context::<RadarMarkersState>().is_some());
            assert!(use_context::<RecenterRequestState>().is_some());
            assert!(use_context::<HillshadeState>().is_some());
            assert!(use_context::<CaptureState>().is_some());
            assert!(use_context::<ModelsState>().is_some());
        });
        owner.cleanup();
    }
}

impl ViewerState {
    /// URLは呼び出し側が指定する。生成だけでは通信や永続化を開始しない。
    pub fn new(terrain_base_url: impl Into<String>) -> Self {
        let drawings = DrawingState::new();
        Self {
            terrain: TerrainStore::new(terrain_base_url),
            origin: OriginState(RwSignal::new(None)),
            radar_markers: RadarMarkersState::new(),
            recenter: RecenterRequestState::new(),
            hillshade: HillshadeState::default(),
            capture: CaptureState::new(),
            drawings,
            draw_tool: DrawToolState::new(drawings),
            tracks: TracksState::new(),
            models: ModelsState::new(),
        }
    }

    /// 作図の保存・復元を有効にする。キーはアプリごとに指定し、登録前に1回だけ呼ぶ。
    pub fn persist_drawings(mut self, key: &'static str) -> Self {
        self.draw_tool = self.draw_tool.persist(key);
        self
    }

    /// 現在のOwnerへ共有状態を登録する。同じOwnerでの二重登録は避けること。
    /// 戻り値の状態へ受信データやモデル設定を反映できる。
    pub fn provide(self) -> Self {
        provide_context(self.terrain);
        provide_context(self.origin);
        provide_context(self.radar_markers);
        provide_context(self.recenter);
        provide_context(self.hillshade);
        provide_context(self.capture);
        provide_context(self.drawings);
        provide_context(self.draw_tool);
        provide_context(self.tracks);
        provide_context(self.models);
        self
    }
}
