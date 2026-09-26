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
    let s = state.borrow();
    let (terrain, mesh_origin, renderer) =
        (s.terrain.clone()?, s.mesh_origin?, s.renderer.as_ref()?);
    let size = renderer.canvas_size_px();
    let (x, y) = client_to_canvas_css(canvas, size, (client_x, client_y));
    let (width, height) = size;
    let camera = s.camera.to_camera(renderer.aspect_ratio());
    pick::pick_lat_lon(
        &terrain,
        &mesh_origin,
        &camera,
        x,
        y,
        width as f32,
        height as f32,
    )
}

/// canvas上の画面座標(client座標)にある航跡のシンボルのID(なければNone)。`terrain::tracks::pick_track`で、
/// シンボルの位置を画面へ射影して最も近いものを選ぶ。
pub(super) fn pick_track_at_client(
    state: &Rc<RefCell<ViewState>>,
    canvas: &web_sys::HtmlCanvasElement,
    (client_x, client_y): (f64, f64),
) -> Option<TrackId> {
    let s = state.borrow();
    let renderer = s.renderer.as_ref()?;
    let (x, y) = client_to_canvas_css(canvas, renderer.canvas_size_px(), (client_x, client_y));
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

/// client座標を、canvasの左上を原点とするレンダラーの画面座標(`TerrainRenderer::canvas_size_px`の
/// CSSピクセル)へ換算する。canvasの見た目の大きさ(`getBoundingClientRect`)とレンダラーが知っている
/// 大きさは、丸めの分だけずれうるので比で合わせる(ふつうは同じ)。
fn client_to_canvas_css(
    canvas: &web_sys::HtmlCanvasElement,
    (width, height): (u32, u32),
    (client_x, client_y): (f64, f64),
) -> (f32, f32) {
    let rect = canvas.get_bounding_client_rect();
    let x = (client_x - rect.left()) * f64::from(width) / rect.width().max(1.0);
    let y = (client_y - rect.top()) * f64::from(height) / rect.height().max(1.0);
    (x as f32, y as f32)
}
