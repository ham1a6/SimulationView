//! canvas上の入力のうち、状態だけを変える部分(ドラッグ→カメラ、作図中のキー操作)。

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::state::ViewState;
use crate::terrain::camera::ViewMode;
use crate::terrain::draw_tool::DrawToolState;
use crate::terrain::heightmap;

/// 3Dモードの通常のドラッグで、1pxあたりに回す角度(ラジアン)。
const ORBIT_SENSITIVITY: f32 = 0.0075;

/// ホイール1ノッチ(`WHEEL_PX_PER_NOTCH`ぶん)でカメラの距離を何倍にするか。
const ZOOM_PER_NOTCH: f32 = 1.12;
/// ホイール1ノッチとみなすピクセル量。Windowsの多くのブラウザは、マウスホイール1ノッチを
/// `deltaMode=0`(ピクセル)の`deltaY=100`で通知する。トラックパッドは小さな値を多数通知するので、
/// 量に比例させることで、どちらでも同じ手応えになる。
const WHEEL_PX_PER_NOTCH: f64 = 100.0;
/// `deltaMode=1`(行単位)の1行をピクセルに換算する量。Firefoxはマウスホイール1ノッチを
/// (既定の設定では)3行で通知するので、3行で1ノッチになるようにしてある。
const WHEEL_PX_PER_LINE: f64 = WHEEL_PX_PER_NOTCH / 3.0;
/// `deltaMode=2`(ページ単位)の1ページをピクセルに換算する量(まれ。画面1枚ぶんほど)。
const WHEEL_PX_PER_PAGE: f64 = 800.0;
/// 1回のwheelイベントで進めるノッチ数の上限(ホイールを勢いよく回した・異常に大きな値で、
/// 一気に最大・最小まで飛ばないように)。
const MAX_NOTCHES_PER_WHEEL_EVENT: f64 = 3.0;

/// wheelイベント(`deltaY`・`deltaMode`)から、カメラの距離に掛ける倍率を求める。下スクロール
/// (`delta_y > 0`)で遠ざかる(1より大きい)。縦の量が無い(横スクロールだけ)・値が不正ならNone。
pub(super) fn wheel_zoom_factor(delta_y: f64, delta_mode: u32) -> Option<f32> {
    if delta_y == 0.0 || !delta_y.is_finite() {
        return None;
    }
    let px = match delta_mode {
        1 => delta_y * WHEEL_PX_PER_LINE,
        2 => delta_y * WHEEL_PX_PER_PAGE,
        _ => delta_y,
    };
    let notches =
        (px / WHEEL_PX_PER_NOTCH).clamp(-MAX_NOTCHES_PER_WHEEL_EVENT, MAX_NOTCHES_PER_WHEEL_EVENT);
    Some(ZOOM_PER_NOTCH.powf(notches as f32))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn one_mouse_notch_zooms_by_the_notch_factor() {
        // Chrome・Edge(ピクセル100)とFirefox(3行)の1ノッチは、同じ倍率になる。
        assert!(close(wheel_zoom_factor(100.0, 0).unwrap(), ZOOM_PER_NOTCH));
        assert!(close(
            wheel_zoom_factor(-100.0, 0).unwrap(),
            1.0 / ZOOM_PER_NOTCH
        ));
        assert!(close(wheel_zoom_factor(3.0, 1).unwrap(), ZOOM_PER_NOTCH));
    }

    #[test]
    fn horizontal_only_or_invalid_scroll_does_not_zoom() {
        assert_eq!(wheel_zoom_factor(0.0, 0), None);
        assert_eq!(wheel_zoom_factor(-0.0, 0), None);
        assert_eq!(wheel_zoom_factor(f64::NAN, 0), None);
        assert_eq!(wheel_zoom_factor(f64::INFINITY, 0), None);
    }

    #[test]
    fn small_trackpad_deltas_zoom_proportionally_and_large_ones_are_capped() {
        // トラックパッドの小さな値は、1ノッチの一部だけ進む(10回で1ノッチ)。
        let step = wheel_zoom_factor(10.0, 0).unwrap();
        assert!(step > 1.0 && step < ZOOM_PER_NOTCH);
        assert!(close(step.powi(10), ZOOM_PER_NOTCH));
        // 異常に大きな値でも、1回で3ノッチまで。
        let cap = ZOOM_PER_NOTCH.powf(MAX_NOTCHES_PER_WHEEL_EVENT as f32);
        assert!(close(wheel_zoom_factor(1.0e6, 0).unwrap(), cap));
        assert!(close(wheel_zoom_factor(-1.0e6, 2).unwrap(), 1.0 / cap));
    }
}
