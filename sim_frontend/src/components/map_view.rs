//! 中央: 地図(3D地形、俯瞰プリセット既定)。実体は`terrain_view::TerrainView`。
//! 右パネルの2つのタブパネル(トップ/ボトムステータスパネル)は`components/right_panel.rs`。

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
