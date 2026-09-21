//! レンダラーの初期化と、1フレームの描画。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use leptos::prelude::*;

use crate::terrain::loader::{TerrainData, WHOLE_TILE};
use crate::terrain::lod::TileLayout;
use crate::terrain::markers::RadarMarkersState;
use crate::terrain::geodesy::EnuTransform;
use crate::terrain::heightmap;
use crate::terrain::mesh;
use crate::terrain::origin::{Origin, OriginState};
use crate::terrain::renderer::TerrainRenderer;
use super::{labels::*, lod_driver::*, models::update_models, overlay::*, state::*};

/// 現在の状態でレンダラーを構築できるなら構築する。
/// canvasのサイズ確定(ResizeObserver)と地形データ取得(TerrainStore)は非同期かつ独立して
/// 完了するため、両方のイベントからこの関数を呼び、揃った時点で実際に初期化されるようにする。
pub(super) fn try_init(
    state: Rc<RefCell<ViewState>>,
    canvas: web_sys::HtmlCanvasElement,
    data: Option<Rc<TerrainData>>,
    origin_state: OriginState,
    status: RwSignal<String>,
    radar_markers: RadarMarkersState,
) {
    let width = canvas.width();
    let height = canvas.height();
    if width == 0 || height == 0 {
        return;
    }
    let Some(data) = data else {
        return;
    };
    {
        let s = state.borrow();
        if s.renderer.is_some() || s.initializing {
            return;
        }
    }
    state.borrow_mut().initializing = true;

    let origin = origin_state.0.get_untracked().unwrap_or(Origin {
        lat_deg: data.metadata.default_origin.lat_deg,
        lon_deg: data.metadata.default_origin.lon_deg,
    });
    // 注視点は原点の実際の地表標高に置く(Vec3::ZEROのままだと、原点が高山の
    // 斜面にある場合にズームインした際カメラが地面に埋まって真っ黒になる)。
    let target_up = heightmap::sample_heightmap(&data, origin.lat_deg, origin.lon_deg).unwrap_or(0.0);

    wasm_bindgen_futures::spawn_local(async move {
        match TerrainRenderer::new(canvas).await {
            Ok(mut renderer) => {
                // 全タイルを最粗のレベル0(タイル全体で1枚)で載せる(細かいレベルはカメラに近い
                // チャンクだけ、あとから`update_lod`が差し替える)。
                let transform = EnuTransform::new(&origin, &data.metadata.ellipsoid);
                let mut resident = HashMap::new();
                for tile in data.tiles() {
                    let tile_mesh = mesh::build_whole_tile_mesh(&data, tile, &transform);
                    renderer.set_mesh((tile.key.0, tile.key.1, WHOLE_TILE), &tile_mesh);
                    resident.insert(tile.key, TileLayout::Whole);
                }
                {
                    let mut s = state.borrow_mut();
                    s.target_up = target_up;
                    s.camera.target.z = target_up;
                    s.resident = resident;
                }
                renderer.set_hillshade(state.borrow().hillshade.enabled.get_untracked());
                renderer.set_ellipsoid_origin(&transform);
                let camera = state.borrow().camera.to_camera(renderer.aspect_ratio());
                if let Err(e) = renderer.render(&camera) {
                    log::error!("[terrain] initial render failed: {e}");
                }
                let mut s = state.borrow_mut();
                s.renderer = Some(renderer);
                s.terrain = Some(data);
                s.mesh_origin = Some(origin);
                s.initializing = false;
                // signalの更新で購読しているEffectが同期的に走っても`state`を借用し直せるよう、
                // 借用を手放してから更新する。
                drop(s);
                status.set(String::new());
                rebuild_markers(&state, radar_markers);
                rebuild_drawings(&state);
                rebuild_tracks(&state);
                render_now(&state);
            }
            Err(e) => {
                log::error!("[terrain] {e}");
                status.set(format!("地形描画エラー: {e}"));
                state.borrow_mut().initializing = false;
            }
        }
    });
}

/// 3Dモードのカメラ(視点)が地面の下にもぐらないようにする(`OrbitCamera::keep_above_ground`)。
/// 視点の真下の地面の高さは、いま画面に出している地形(`heightmap::ground_at_enu`)から引く。
/// カメラの操作(回転・ズーム・移動)・原点変更・LODの切り替えのあとの描画の前に必ず通る。
pub(super) fn keep_camera_above_ground(state: &Rc<RefCell<ViewState>>) {
    let mut s = state.borrow_mut();
    let (Some(terrain), Some(origin)) = (s.terrain.clone(), s.mesh_origin) else {
        return;
    };
    let transform = EnuTransform::new(&origin, &terrain.metadata.ellipsoid);
    s.camera.keep_above_ground(|east, north| {
        heightmap::ground_at_enu(&terrain, &transform, east as f64, north as f64).2
    });
}

/// 現在の状態で1フレーム描くだけ(LODの更新は予約しない)。
pub(super) fn render_frame(state: &Rc<RefCell<ViewState>>) {
    keep_camera_above_ground(state);
    // どのトラックを3Dモデルで描くか(カメラからの距離・大きさで決まる)。描画の前に決める。
    update_models(state);
    let mut s = state.borrow_mut();
    let s = &mut *s;
    let mut fading = false;
    if let Some(renderer) = s.renderer.as_mut() {
        let camera = s.camera.to_camera(renderer.aspect_ratio());
        if let Err(e) = renderer.render(&camera) {
            log::error!("[terrain] render failed: {e}");
        }
        fading = renderer.is_fading();
    }
    update_labels(s);
    // 地形のレベル切り替えのクロスフェード中は、時間が進むので次のフレームも描く(1つだけ予約する)。
    if fading && !s.fade_frame_pending {
        s.fade_frame_pending = true;
        let state = state.clone();
        request_animation_frame(move || {
            state.borrow_mut().fade_frame_pending = false;
            render_frame(&state);
        });
    }
}

/// 1フレーム描き、カメラなどが変わった可能性があるのでLODの更新を予約する。
pub(super) fn render_now(state: &Rc<RefCell<ViewState>>) {
    render_frame(state);
    schedule_lod(state);
}
