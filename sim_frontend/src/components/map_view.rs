//! 中央: 地図(3D地形、俯瞰プリセット既定)。実体は`terrain_view::TerrainView`。
//! 右パネル下: 側面図。マップ原点を起点に、スライダーで指定した方位角方向の地形断面を
//! 2Dで表示する(`cross_section_view::CrossSectionView`)。

use leptos::prelude::*;

use crate::components::cross_section_view::CrossSectionView;
use crate::components::terrain_view::TerrainView;
use crate::terrain::camera::CameraPreset;

#[component]
pub fn MapView() -> impl IntoView {
    view! {
        <div class="map-view">
            <TerrainView preset=CameraPreset::Overview/>
        </div>
    }
}

#[component]
pub fn SideView() -> impl IntoView {
    view! {
        <div class="panel-section side-view">
            <h2>"側面図"</h2>
            <CrossSectionView/>
        </div>
    }
}
