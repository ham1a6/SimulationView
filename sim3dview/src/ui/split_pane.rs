//! 最小幅を保ちながら左右の区画をリサイズする汎用部品。
use super::pointer_drag::DragTracker;
use leptos::prelude::*;

fn fraction_after_drag(first: f64, total: f64, dx: f64, min_first: f64, min_second: f64) -> f64 {
    (first + dx).clamp(min_first, total - min_second) / total
}

/// 横分割。狭い画面では最小幅の合計を維持するため、親で横スクロールを許可する。
/// 幅はCSS px。初期比率は左区画の割合(0〜1)。すべて有限値を指定する。
#[component]
pub fn SplitPane(
    #[prop(into)] first: ViewFn,
    #[prop(into)] second: ViewFn,
    #[prop(default = 0.5)] initial_fraction: f64,
    #[prop(default = 160.0)] min_first: f64,
    #[prop(default = 160.0)] min_second: f64,
) -> impl IntoView {
    assert!(min_first.is_finite() && min_first > 0.0);
    assert!(min_second.is_finite() && min_second > 0.0);
    assert!(initial_fraction.is_finite());
    let fraction = RwSignal::new(initial_fraction.clamp(0.001, 0.999));
    let left = NodeRef::<leptos::html::Div>::new();
    let right = NodeRef::<leptos::html::Div>::new();
    let drag = RwSignal::new(DragTracker::default());
    let start = RwSignal::new(None::<(f64, f64)>);
    let columns = move || {
        let f = fraction.get();
        format!(
            "minmax({min_first}px, {f}fr) 6px minmax({min_second}px, {}fr)",
            1.0 - f
        )
    };
    let minimum = format!("{}px", min_first + min_second + 6.0);
    view! {
        <div class="sim3d-split-pane" style:grid-template-columns=columns style:min-width=minimum>
            <div class="sim3d-split-content" node_ref=left>{first.run()}</div>
            <div class="sim3d-split-handle"
                on:pointerdown=move |ev: leptos::ev::PointerEvent| {
                    if ev.button() != 0 || start.get_untracked().is_some() { return; }
                    let (Some(l), Some(r)) = (left.get(), right.get()) else { return; };
                    let width = l.get_bounding_client_rect().width();
                    let total = width + r.get_bounding_client_rect().width();
                    if total < min_first + min_second { return; }
                    use wasm_bindgen::JsCast;
                    let Some(el) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return; };
                    if el.set_pointer_capture(ev.pointer_id()).is_err() { return; }
                    ev.prevent_default();
                    start.set(Some((width, total)));
                    drag.update(|d| d.begin(ev.pointer_id(), ev.client_x() as f64, ev.client_y() as f64));
                }
                on:pointermove=move |ev: leptos::ev::PointerEvent| {
                    let Some((width, total)) = start.get_untracked() else { return; };
                    let update = drag.try_update(|d| d.update(ev.pointer_id(), ev.client_x() as f64, ev.client_y() as f64)).flatten();
                    if let Some(update) = update {
                        fraction.set(fraction_after_drag(width, total, update.total.0, min_first, min_second));
                    }
                }
                on:pointerup=move |ev: leptos::ev::PointerEvent| {
                    if drag.try_update(|d| d.end(ev.pointer_id(), ev.client_x() as f64, ev.client_y() as f64)).flatten().is_some() {
                        start.set(None);
                    }
                }
                on:pointercancel=move |ev: leptos::ev::PointerEvent| {
                    if drag.try_update(|d| d.cancel(ev.pointer_id())).unwrap_or(false) { start.set(None); }
                }
                on:lostpointercapture=move |ev: leptos::ev::PointerEvent| {
                    if drag.try_update(|d| d.cancel(ev.pointer_id())).unwrap_or(false) { start.set(None); }
                }
            ></div>
            <div class="sim3d-split-content" node_ref=right>{second.run()}</div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drag_uses_actual_width_and_preserves_both_minima() {
        assert_eq!(
            fraction_after_drag(800.0, 1200.0, 100.0, 320.0, 260.0),
            0.75
        );
        assert_eq!(
            fraction_after_drag(800.0, 1200.0, -2000.0, 320.0, 260.0),
            320.0 / 1200.0
        );
        assert_eq!(
            fraction_after_drag(800.0, 1200.0, 2000.0, 320.0, 260.0),
            940.0 / 1200.0
        );
        assert_eq!(
            fraction_after_drag(320.0, 580.0, 100.0, 320.0, 260.0),
            320.0 / 580.0
        );
    }
}
