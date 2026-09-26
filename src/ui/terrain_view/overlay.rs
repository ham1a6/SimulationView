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
use crate::terrain::markers;
use crate::terrain::models::placement::build_placements;
use crate::terrain::tracks::{self, TrackOptions};

/// レーダー観測点マーカー(ピン)・見通し範囲の覆域(3Dはドーム、2Dは塗り+輪郭線)のジオメトリを、現在の地形・原点・
/// マーカー一覧・選択状態から作り直してGPUバッファへ反映する(覆域は非同期の計算が終わってから反映する)。原点変更時
/// (メッシュ再構築後)・マーカー追加/削除/選択変更時に呼ぶ。描画自体は呼び出し側で
/// `render_now`すること。
pub(super) fn rebuild_markers(state: &Rc<RefCell<ViewState>>) {
    rebuild_marker_pins(state);
    // 覆域(計算が重い)は、計算済みならジオメトリの作り直しだけ、未計算なら小分けに非同期で計算する(`coverage`)。
    refresh_coverage(state, false);
}

/// `rebuild_markers`と同じだが、地形のレベルが切り替わったとき用。ピンは地表の高さに合わせて作り直し、覆域は
/// 観測点の範囲の地形が変わっていれば、少し待ってから計算し直す(切り替えは続けて何度も起きるため)。
pub(super) fn rebuild_markers_for_terrain(state: &Rc<RefCell<ViewState>>) {
    rebuild_marker_pins(state);
    refresh_coverage(state, true);
}

/// 観測点のマーカー(ピン)のジオメトリだけを作り直す(軽い)。
fn rebuild_marker_pins(state: &Rc<RefCell<ViewState>>) {
    let mut s = state.borrow_mut();
    let (Some(terrain), Some(mesh_origin)) = (s.terrain.clone(), s.mesh_origin) else {
        return;
    };
    // `renderer`を可変で借りる前に、必要なもの(Copyなcontext)を取り出しておく。
    let radar_markers = s.radar_markers;
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
    /// 地形データ(地表の高さを引く)。
    terrain: Rc<TerrainData>,
    /// いまGPUにあるメッシュの原点のENU変換。
    transform: EnuTransform,
    /// canvasの内部解像度(幅, 高さ。ピクセル)。画面座標の作図・線の太さに使う。
    viewport_px: (f32, f32),
}

impl GeometryInputs {
    /// `ViewState`の地形・メッシュの原点・canvasの大きさから作る。どれかがまだ無ければNone。
    fn of(s: &ViewState) -> Option<Self> {
        let (terrain, transform) = s.mesh_frame()?;
        let (width, height) = s.renderer.as_ref()?.canvas_size_px();
        Some(Self {
            terrain,
            transform,
            viewport_px: (width as f32, height as f32),
        })
    }

    /// `BuildContext`を作って`f`に渡し、その結果を返す。
    fn with_context<R>(&self, f: impl FnOnce(&drawing_geometry::BuildContext) -> R) -> R {
        // 描画中の三角形に高さを合わせる。範囲外・海は標高0m。
        let ground =
            |lat: f64, lon: f64| heightmap::sample_surface_height(&self.terrain, lat, lon) as f64;
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
    let Some(inputs) = GeometryInputs::of(&s) else {
        return;
    };
    let drawings = s.drawings;
    let Some(renderer) = s.renderer.as_mut() else {
        return;
    };
    let batches = inputs.with_context(|ctx| {
        drawings
            .items
            .with_untracked(|list| drawing_geometry::build(ctx, list))
    });
    renderer.update_drawings(&batches);
}

/// 地形のLOD切り替えが落ち着いてから、地表貼り付けの作図を1回だけ作り直す(300ms、覆域と同じ考え方)。
///
/// 地表貼り付けの図形は、表示中の地形の三角形を切り抜いて重ねる(`drawing_geometry::drape`)ため、
/// 地形のLOD更新(`lod_driver::update_lod`)の1ラウンドごとに同期で呼ぶと、時間で区切ってあるはずの
/// メッシュ生成の予算(`UPLOAD_TIME_BUDGET_MS`)を無視して固まる。覆域(`coverage.rs`)がすでに解決した
/// 「LODの小刻みな変化のたびに重い処理をやり直す」問題と同じなので、同じデバウンスで対処する。
const DRAWING_TERRAIN_DEBOUNCE_MS: u32 = 300;
pub(super) fn schedule_drawings_rebuild(state: &Rc<RefCell<ViewState>>) {
    {
        let mut s = state.borrow_mut();
        if s.lod.drawings_rebuild_pending {
            return;
        }
        s.lod.drawings_rebuild_pending = true;
    }
    let state = state.clone();
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(DRAWING_TERRAIN_DEBOUNCE_MS).await;
        state.borrow_mut().lod.drawings_rebuild_pending = false;
        rebuild_drawings(&state);
        super::frame::render_frame(&state);
    });
}

/// 航跡(`terrain::tracks`)の一覧・表示設定から、シンボル・航跡・高度線の頂点列とラベルを作り直して反映する。
/// トラックの受信・表示設定の変更・原点変更・地形のLOD切り替え・2D/3D切り替えのときに呼ぶ。
/// 描画自体は呼び出し側で`render_frame`(または`render_now`)すること。
pub(super) fn rebuild_tracks(state: &Rc<RefCell<ViewState>>) {
    let mut s = state.borrow_mut();
    let Some(inputs) = GeometryInputs::of(&s) else {
        return;
    };
    let mode = s.camera.mode;
    let tracks_state = s.tracks;
    let layer = label_layer(&s);
    // 3Dモデルで描いているトラックは、シンボルを描かない(`terrain::models`)。
    let symbols_hidden = s.models.shown.clone();
    // シンボル・航跡・高度線の頂点列と、3Dモデルの配置候補を同じ入力から作る。
    let (geometry, placements) = {
        let Some(renderer) = s.renderer.as_mut() else {
            return;
        };
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
    // 3Dモデルの配置・クリックの当たり判定・ラベルは、描画のたびに使うので状態に持たせる。
    s.models.placements = placements;
    s.pick_anchors = geometry.labels.iter().map(|l| (l.id, l.position)).collect();
    let labels = if tracks_state.show_labels.get_untracked() {
        geometry.labels
    } else {
        Vec::new()
    };
    set_labels(&mut s, layer.as_ref(), labels);
}
