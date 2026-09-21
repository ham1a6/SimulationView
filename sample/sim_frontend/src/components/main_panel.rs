//! 中央: メインパネル(3D地形、俯瞰プリセット既定)。実体はsim3dviewライブラリの
//! `ui::terrain_view::TerrainView`。右パネルの2つのタブパネル(トップ/ボトムステータス
//! パネル)は`components/right_panel.rs`。

use leptos::prelude::*;

use sim3dview::terrain::camera::CameraPreset;
use sim3dview::ui::terrain_view::TerrainView;

#[component]
pub fn MainPanel() -> impl IntoView {
    view! {
        <div class="map-view">
            <TerrainView preset=CameraPreset::Overview/>
        </div>
    }
}
