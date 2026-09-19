//! 表示オプション(海レイヤーの表示/非表示)。使う側(サンプルアプリの表示メニュー等)が
//! `provide_context`し、`ui::terrain_view::TerrainView`がこれを読んでレンダラーに反映する。
//! `terrain::origin::OriginState`と同じ、RwSignalを1本だけ持つ薄いラッパーのパターン。

use leptos::prelude::*;

/// 海レイヤー(NaNセル・背景スカートとも`terrain::mesh`のWATER_COLOR)を表示するかどうか。
/// 既定は表示(true)。
#[derive(Clone, Copy)]
pub struct WaterVisibilityState(pub RwSignal<bool>);

impl WaterVisibilityState {
    pub fn new() -> Self {
        Self(RwSignal::new(true))
    }
}

impl Default for WaterVisibilityState {
    fn default() -> Self {
        Self::new()
    }
}
