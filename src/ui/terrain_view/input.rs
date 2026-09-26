//! canvas上の入力のうち、状態だけを変える部分(ドラッグ→カメラ、作図中のキー操作)。

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::state::ViewState;
use crate::terrain::camera::ViewMode;
use crate::terrain::draw_tool::DrawToolState;
use crate::terrain::heightmap;

/// 3Dモードの通常のドラッグで、1pxあたりに回す角度(ラジアン)。
const ORBIT_SENSITIVITY: f32 = 0.0075;

/// ドラッグ量(`dx`, `dy`、CSSピクセル)をカメラへ反映する。3Dは回転(`shift`なら注視点の平行移動)、
/// 2Dは平行移動。シミュレーション原点(`OriginState`)には触れない。
pub(super) fn apply_drag(s: &mut ViewState, dx: f32, dy: f32, shift: bool) {
    // 平行移動量をピクセル→メートルに換算するための、canvasの内部解像度の縦幅。
    let canvas_h = s.canvas_height_px() as f32;
    match s.camera.mode {
        ViewMode::ThreeD if shift => {
            // Shift+ドラッグ: 回転ではなく注視点(中心点)を平行移動する
            // (「原点は変えないでね」との要望通り、OriginStateには触れない)。
            s.camera.pan_orbit_target(dx, dy, canvas_h);
            // 移動先の実際の地表(ENU上座標)へtarget.zを更新する(古い高さの
            // ままだと、原点変更時と同様にズームインした際カメラが地面に
            // 埋まって真っ黒になりうる)。原点から遠いほど地球の丸みで地表が
            // 下がるため、標高ではなく丸みを含む上座標を使う。
            if let Some((terrain, transform)) = s.mesh_frame() {
                let (_, _, up) = heightmap::ground_at_enu(
                    &terrain,
                    &transform,
                    s.camera.target.x as f64,
                    s.camera.target.y as f64,
                );
                s.camera.target.z = up;
            }
        }
        ViewMode::ThreeD => s
            .camera
            .orbit(dx * ORBIT_SENSITIVITY, dy * ORBIT_SENSITIVITY),
        ViewMode::TwoD => {
            // 正射影の画面縦幅(distance)と実際のcanvas高さ(ピクセル)の比から、
            // 画面上のドラッグ量をワールド座標(メートル)の移動量へ変換する。
            let world_per_px = s.camera.distance / canvas_h;
            // 画面上は北=上(up=Vec3::Y)なので、上方向のドラッグ(dy<0)は北への移動。
            s.camera.pan(dx * world_per_px, -dy * world_per_px);
        }
    }
}

/// 図形の作成中のキー操作(Esc=終了、Enter=確定、Backspace=1つ戻す)。作成中でないとき・入力欄への入力は邪魔しない。
pub(super) fn handle_draw_key(tool: DrawToolState, ev: &web_sys::KeyboardEvent) {
    if tool.tool.get_untracked().is_none() {
        return;
    }
    // リスナーはwindowに付けているので、フォームの入力欄でのキー入力(文字の削除など)は横取りしない。
    let in_form = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT"));
    if in_form {
        return;
    }
    match ev.key().as_str() {
        "Escape" => tool.cancel(),
        "Enter" => tool.finish(),
        "Backspace" => {
            // ブラウザ既定の動作(ブラウザによっては「戻る」)を止める。
            ev.prevent_default();
            tool.undo();
        }
        _ => {}
    }
}
