//! 画面座標から、地表の緯度経度・航跡のシンボルを求める(クリック・右クリック・ホバー)。

use std::cell::RefCell;
use std::rc::Rc;

use super::state::*;
use crate::terrain::pick;
use crate::terrain::tracks::{self, TrackId};

/// この距離(CSSピクセル)未満の移動なら、ドラッグではなく単発クリックとして扱う。
pub(super) const CLICK_MAX_MOVE_PX: f64 = 5.0;

/// canvas上の画面座標(client座標)が指す地表の緯度経度。地形データ範囲外・未初期化ならNone。
pub(super) fn pick_at_client(
    state: &Rc<RefCell<ViewState>>,
    canvas: &web_sys::HtmlCanvasElement,
    (client_x, client_y): (f64, f64),
) -> Option<(f64, f64)> {
    // client座標をcanvasの左上基準のCSSピクセルへ(canvasの大きさはCSSピクセルに合わせてある:
    // `resize::observe_canvas_size`)。
    let rect = canvas.get_bounding_client_rect();
    let x = client_x as f32 - rect.left() as f32;
    let y = client_y as f32 - rect.top() as f32;
    let s = state.borrow();
    let (terrain, mesh_origin, renderer) =
        (s.terrain.clone()?, s.mesh_origin?, s.renderer.as_ref()?);
    let camera = s.camera.to_camera(renderer.aspect_ratio());
    pick::pick_lat_lon(
        &terrain,
        &mesh_origin,
        &camera,
        x,
        y,
        canvas.width() as f32,
        canvas.height() as f32,
    )
}

/// canvas上の画面座標(client座標)にある航跡のシンボルのID(なければNone)。`terrain::tracks::pick_track`で、
/// シンボルの位置を画面へ射影して最も近いものを選ぶ。
pub(super) fn pick_track_at_client(
    state: &Rc<RefCell<ViewState>>,
    canvas: &web_sys::HtmlCanvasElement,
    (client_x, client_y): (f64, f64),
) -> Option<TrackId> {
    let rect = canvas.get_bounding_client_rect();
    // CSSのpxからcanvasの内部解像度のpxへ(通常は同じ)。
    let x = (client_x - rect.left()) as f32 * canvas.width() as f32 / rect.width().max(1.0) as f32;
    let y = (client_y - rect.top()) as f32 * canvas.height() as f32 / rect.height().max(1.0) as f32;
    let s = state.borrow();
    let renderer = s.renderer.as_ref()?;
    let view_proj = s
        .camera
        .to_camera(renderer.aspect_ratio())
        .view_proj_matrix();
    let (width, height) = renderer.canvas_size_px();
    tracks::pick_track(
        &s.pick_anchors,
        &view_proj,
        (width as f32, height as f32),
        (x, y),
        tracks::PICK_RADIUS_PX,
    )
}
