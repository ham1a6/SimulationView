//! 航跡のラベル(canvasに重ねるHTML要素)の配置と更新。


use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::terrain::tracks::TrackLabel;
use super::state::*;

/// ラベルを覆う層(`TerrainView`が`canvas`の上に置くHTML要素)。
pub(super) fn label_layer(s: &ViewState) -> Option<web_sys::HtmlElement> {
    let element = s.labels_ref.get_untracked()?;
    (*element).clone().dyn_into::<web_sys::HtmlElement>().ok()
}

/// 画面に重ねるラベルを`labels`にそろえる。数が同じなら要素を使い回して文字だけ更新し、変わったら作り直す。
pub(super) fn set_labels(s: &mut ViewState, layer: Option<&web_sys::HtmlElement>, labels: Vec<TrackLabel>) {
    let Some(layer) = layer else {
        return;
    };
    if s.labels.len() != labels.len() {
        layer.set_text_content(None);
        s.labels.clear();
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let make = |class: &str| -> Option<web_sys::HtmlElement> {
            let element = document.create_element("div").ok()?.dyn_into::<web_sys::HtmlElement>().ok()?;
            element.set_class_name(class);
            Some(element)
        };
        for anchor in &labels {
            let (Some(root), Some(name), Some(detail)) =
                (make("track-label"), make("track-label-name"), make("track-label-detail"))
            else {
                continue;
            };
            let _ = root.append_child(&name);
            let _ = root.append_child(&detail);
            let _ = layer.append_child(&root);
            // 位置は毎フレーム`update_labels`が決める。名前・詳細・色は下で入れる(初回は必ず異なる扱いにする)。
            s.labels.push(LabelView {
                anchor: TrackLabel {
                    name: String::new(),
                    detail: String::new(),
                    color: [-1.0; 3],
                    selected: false,
                    ..anchor.clone()
                },
                root,
                name,
                detail,
            });
        }
    }
    for (view, new) in s.labels.iter_mut().zip(labels) {
        if view.anchor.name != new.name {
            view.name.set_text_content(Some(&new.name));
        }
        if view.anchor.detail != new.detail {
            view.detail.set_text_content(Some(&new.detail));
        }
        if view.anchor.selected != new.selected {
            view.root.set_class_name(if new.selected { "track-label selected" } else { "track-label" });
        }
        if view.anchor.color != new.color {
            let [r, g, b] = new.color;
            let _ = view.root.style().set_property(
                "color",
                &format!("rgb({},{},{})", (r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8),
            );
        }
        view.anchor = new;
    }
}

/// 航跡ラベルを、いまのカメラでの画面位置へ動かす(毎フレーム)。カメラの後ろ・画面の外は隠す。
/// トラックの位置(`TrackLabel::position`)は、シンボルと同じく地形メッシュの原点基準のENU座標。
pub(super) fn update_labels(s: &ViewState) {
    let (Some(renderer), false) = (s.renderer.as_ref(), s.labels.is_empty()) else {
        return;
    };
    let view_proj = s.camera.to_camera(renderer.aspect_ratio()).view_proj_matrix();
    let (width, height) = renderer.canvas_size_px();
    let (width, height) = (width as f32, height as f32);
    for label in &s.labels {
        let [x, y, z] = label.anchor.position;
        let clip = view_proj * glam::Vec4::new(x, y, z, 1.0);
        let (ndc_x, ndc_y) = (clip.x / clip.w, clip.y / clip.w);
        let visible = clip.w > 0.0 && ndc_x.abs() <= 1.1 && ndc_y.abs() <= 1.1;
        let style = label.root.style();
        if visible {
            // シンボル(約30px)の右上にずらして置く。
            let px = (ndc_x + 1.0) * 0.5 * width + LABEL_OFFSET_X_PX;
            let py = (1.0 - ndc_y) * 0.5 * height + LABEL_OFFSET_Y_PX;
            let _ = style.set_property("transform", &format!("translate({px:.1}px, {py:.1}px)"));
            let _ = style.set_property("display", "block");
        } else {
            let _ = style.set_property("display", "none");
        }
    }
}

/// 航跡ラベルを、シンボルの位置からずらす量(画面のpx)。
pub(super) const LABEL_OFFSET_X_PX: f32 = 18.0;
pub(super) const LABEL_OFFSET_Y_PX: f32 = -16.0;
