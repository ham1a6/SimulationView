//! 地形の上に重ねるもの(レーダー観測点・作図・航跡)のジオメトリを作り直してレンダラーへ渡す。

use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;

use super::{coverage::refresh_coverage, labels::*, state::*};
use crate::terrain::camera::ViewMode;
use crate::terrain::drawing_geometry;
use crate::terrain::geodesy::EnuTransform;
use crate::terrain::heightmap;
use crate::terrain::loader::TerrainData;
use crate::terrain::markers::{self, RadarMarkersState};
use crate::terrain::models::placement::build_placements;
use crate::terrain::origin::Origin;
use crate::terrain::tracks::{self, TrackOptions};

/// レーダー観測点マーカー(ピン)・見通し範囲の覆域(3Dはドーム、2Dは塗り+輪郭線)のジオメトリを、現在の地形・原点・
/// マーカー一覧・選択状態から作り直してGPUバッファへ反映する(覆域は非同期の計算が終わってから反映する)。原点変更時
/// (メッシュ再構築後)・マーカー追加/削除/選択変更時に呼ぶ。描画自体は呼び出し側で
/// `render_now`すること。
pub(super) fn rebuild_markers(state: &Rc<RefCell<ViewState>>, radar_markers: RadarMarkersState) {
    rebuild_marker_pins(state, radar_markers);
    // 覆域(計算が重い)は、計算済みならジオメトリの作り直しだけ、未計算なら小分けに非同期で計算する(`coverage`)。
    refresh_coverage(state, radar_markers, false);
}

/// `rebuild_markers`と同じだが、地形のレベルが切り替わったとき用。ピンは地表の高さに合わせて作り直し、覆域は
/// 観測点の範囲の地形が変わっていれば、少し待ってから計算し直す(切り替えは続けて何度も起きるため)。
pub(super) fn rebuild_markers_for_terrain(
    state: &Rc<RefCell<ViewState>>,
    radar_markers: RadarMarkersState,
) {
    rebuild_marker_pins(state, radar_markers);
    refresh_coverage(state, radar_markers, true);
}

/// 観測点のマーカー(ピン)のジオメトリだけを作り直す(軽い)。
fn rebuild_marker_pins(state: &Rc<RefCell<ViewState>>, radar_markers: RadarMarkersState) {
    let mut s = state.borrow_mut();
    let (Some(terrain), Some(mesh_origin)) = (s.terrain.clone(), s.mesh_origin) else {
        return;
    };
    let Some(renderer) = s.renderer.as_mut() else {
        return;
    };
    let marker_list = radar_markers.markers.get_untracked();
    let selected = radar_markers.selected.get_untracked();
    renderer.update_markers(&markers::build_marker_geometry(
        &terrain,
        &mesh_origin,
        &marker_list,
        selected,
    ));
}

/// 作図・航跡のジオメトリ生成(`drawing_geometry::BuildContext`)の入力。`BuildContext`は変換と地表の高さの
/// 関数を借りるので、それらの持ち主をここに置き、`with_context`の間だけ`BuildContext`を作る。
struct GeometryInputs {
    terrain: Rc<TerrainData>,
    transform: EnuTransform,
    viewport_px: (f32, f32),
}

impl GeometryInputs {
    fn new(terrain: &Rc<TerrainData>, mesh_origin: &Origin, (width, height): (u32, u32)) -> Self {
        Self {
            terrain: terrain.clone(),
            transform: EnuTransform::new(mesh_origin, &terrain.metadata.ellipsoid),
            viewport_px: (width as f32, height as f32),
        }
    }

    fn with_context<R>(&self, f: impl FnOnce(&drawing_geometry::BuildContext) -> R) -> R {
        // 描画中の三角形に高さを合わせる。範囲外・海は標高0m。
        let ground = |lat: f64, lon: f64| {
            heightmap::sample_surface_height(&self.terrain, lat, lon).unwrap_or(0.0) as f64
        };
        f(&drawing_geometry::BuildContext {
            terrain: Some(&self.terrain),
            mesh_transform: &self.transform,
            ellipsoid: &self.terrain.metadata.ellipsoid,
            ground: &ground,
            viewport_px: self.viewport_px,
        })
    }
}

/// 作図(`terrain::drawing`)の一覧から頂点列を作り直してGPUバッファへ反映する。一覧の変更・原点変更
/// (メッシュ再構築後)・地表に貼り付けた図形があるときの地形LOD切り替え・canvasのリサイズ
/// (画面座標の角の位置が変わる)のときに呼ぶ。描画自体は呼び出し側で`render_now`すること。
pub(super) fn rebuild_drawings(state: &Rc<RefCell<ViewState>>) {
    let mut s = state.borrow_mut();
    let (Some(terrain), Some(mesh_origin)) = (s.terrain.clone(), s.mesh_origin) else {
        return;
    };
    let drawings = s.drawings;
    let Some(renderer) = s.renderer.as_mut() else {
        return;
    };
    let inputs = GeometryInputs::new(&terrain, &mesh_origin, renderer.canvas_size_px());
    let batches = inputs.with_context(|ctx| {
        drawings
            .items
            .with_untracked(|list| drawing_geometry::build(ctx, list))
    });
    renderer.update_drawings(&batches);
}

/// 航跡(`terrain::tracks`)の一覧・表示設定から、シンボル・航跡・高度線の頂点列とラベルを作り直して反映する。
/// トラックの受信・表示設定の変更・原点変更・地形のLOD切り替え・2D/3D切り替えのときに呼ぶ。
/// 描画自体は呼び出し側で`render_frame`(または`render_now`)すること。
pub(super) fn rebuild_tracks(state: &Rc<RefCell<ViewState>>) {
    let mut s = state.borrow_mut();
    let (Some(terrain), Some(mesh_origin), mode) =
        (s.terrain.clone(), s.mesh_origin, s.camera.mode)
    else {
        return;
    };
    let tracks_state = s.tracks;
    let layer = label_layer(&s);
    // 3Dモデルで描いているトラックは、シンボルを描かない(`terrain::models`)。
    let symbols_hidden = s.models.shown.clone();
    let (geometry, placements) = {
        let Some(renderer) = s.renderer.as_mut() else {
            return;
        };
        let inputs = GeometryInputs::new(&terrain, &mesh_origin, renderer.canvas_size_px());
        let options = TrackOptions {
            selected: tracks_state.selected.get_untracked(),
            trails: tracks_state.show_trails.get_untracked(),
            // 真上から見る2D地図では、縦の線は点になるので出さない。
            altitude_lines: tracks_state.show_altitude_lines.get_untracked()
                && mode == ViewMode::ThreeD,
            symbols_hidden: Some(&symbols_hidden),
        };
        let (geometry, placements) = inputs.with_context(|ctx| {
            tracks_state.entries.with_untracked(|entries| {
                (
                    tracks::build_track_geometry(ctx, entries, options),
                    build_placements(ctx, entries),
                )
            })
        });
        renderer.update_tracks(&geometry.vertices);
        (geometry, placements)
    };
    s.models.placements = placements;
    s.pick_anchors = geometry.labels.iter().map(|l| (l.id, l.position)).collect();
    let labels = if tracks_state.show_labels.get_untracked() {
        geometry.labels
    } else {
        Vec::new()
    };
    set_labels(&mut s, layer.as_ref(), labels);
}
