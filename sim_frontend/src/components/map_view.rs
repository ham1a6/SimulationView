//! 中央: 地図(3D地形、俯瞰プリセット既定)、右パネル下: 側面図(側面プリセット既定)。
//! 実体は共通コンポーネント`terrain_view::TerrainView`(同一の地形メッシュに異なるカメラを適用)。

use leptos::prelude::*;

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
            <TerrainView preset=CameraPreset::Side/>
        </div>
    }
}
