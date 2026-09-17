//! 地形メッシュを描画する再利用可能なcanvasコンポーネント。
//! 中央の地図・右パネル下部の側面図の両方がこれを使う(DESIGN.md 5.2節: 同一メッシュに
//! 異なるカメラを適用する構成)。地形データ本体は`TerrainStore`で共有し、フェッチは1回だけ。
//!
//! フェーズ10: 自由視点カメラ(ドラッグで回転、ホイールでズーム、俯瞰/側面プリセット)。

use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::terrain::camera::{CameraPreset, OrbitCamera};
use crate::terrain::loader::TerrainData;
use crate::terrain::mesh::{self, Origin};
use crate::terrain::renderer::TerrainRenderer;
use crate::terrain::store::TerrainStore;
use crate::ws::WsSignals;

struct ViewState {
    renderer: Option<TerrainRenderer>,
    terrain: Option<Rc<TerrainData>>,
    /// 現在GPUにアップロードされているメッシュが基づいている原点。
    mesh_origin: Option<Origin>,
    camera: OrbitCamera,
    initializing: bool,
    dragging: bool,
    last_x: f64,
    last_y: f64,
}

/// 現在の状態でレンダラーを構築できるなら構築する。
/// canvasのサイズ確定(ResizeObserver)と地形データ取得(TerrainStore)は非同期かつ独立して
/// 完了するため、両方のイベントからこの関数を呼び、揃った時点で実際に初期化されるようにする。
fn try_init(
    state: Rc<RefCell<ViewState>>,
    canvas: web_sys::HtmlCanvasElement,
    data: Option<Rc<TerrainData>>,
    signals: WsSignals,
    status: RwSignal<String>,
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

    let origin = signals
        .origin
        .get_untracked()
        .map(|o| Origin {
            lat_deg: o.lat_deg,
            lon_deg: o.lon_deg,
        })
        .unwrap_or(Origin {
            lat_deg: data.metadata.default_origin.lat_deg,
            lon_deg: data.metadata.default_origin.lon_deg,
        });
    let terrain_mesh = mesh::build_mesh(&data, &origin);

    wasm_bindgen_futures::spawn_local(async move {
        match TerrainRenderer::new(canvas, &terrain_mesh).await {
            Ok(renderer) => {
                let camera = state.borrow().camera.to_camera(renderer.aspect_ratio());
                if let Err(e) = renderer.render(&camera) {
                    log::error!("[terrain] initial render failed: {e}");
                }
                let mut s = state.borrow_mut();
                s.renderer = Some(renderer);
                s.terrain = Some(data);
                s.mesh_origin = Some(origin);
                s.initializing = false;
                status.set(String::new());
            }
            Err(e) => {
                log::error!("[terrain] {e}");
                status.set(format!("地形描画エラー: {e}"));
                state.borrow_mut().initializing = false;
            }
        }
    });
}

fn render_now(state: &Rc<RefCell<ViewState>>) {
    let s = state.borrow();
    if let Some(renderer) = s.renderer.as_ref() {
        let camera = s.camera.to_camera(renderer.aspect_ratio());
        if let Err(e) = renderer.render(&camera) {
            log::error!("[terrain] render failed: {e}");
        }
    }
}

#[component]
pub fn TerrainView(preset: CameraPreset) -> impl IntoView {
    let canvas_ref: NodeRef<leptos::html::Canvas> = NodeRef::new();
    let status = RwSignal::new("地形データを読み込み中...".to_string());
    let signals = use_context::<WsSignals>().expect("WsSignals context not found");
    let terrain_store = use_context::<TerrainStore>().expect("TerrainStore context not found");

    terrain_store.ensure_loaded();

    let state = Rc::new(RefCell::new(ViewState {
        renderer: None,
        terrain: None,
        mesh_origin: None,
        camera: OrbitCamera::preset(preset),
        initializing: false,
        dragging: false,
        last_x: 0.0,
        last_y: 0.0,
    }));

    // --- Effect 1: canvasのマウント + ResizeObserver(初回サイズ確定・以後のリサイズ追従) ---
    {
        let state = state.clone();
        Effect::new(move |_| {
            let Some(canvas_el) = canvas_ref.get() else {
                return;
            };
            let canvas: web_sys::HtmlCanvasElement = (*canvas_el)
                .clone()
                .dyn_into()
                .expect("canvas node_ref should be an HtmlCanvasElement");

            let canvas_for_cb = canvas.clone();
            let state_for_cb = state.clone();
            let closure = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
                let Some(entry) = entries
                    .get(0)
                    .dyn_into::<web_sys::ResizeObserverEntry>()
                    .ok()
                else {
                    return;
                };
                let rect = entry.content_rect();
                let width = rect.width().round().max(0.0) as u32;
                let height = rect.height().round().max(0.0) as u32;
                if width == 0 || height == 0 {
                    return;
                }
                canvas_for_cb.set_width(width);
                canvas_for_cb.set_height(height);

                let has_renderer = state_for_cb.borrow().renderer.is_some();
                if has_renderer {
                    let mut s = state_for_cb.borrow_mut();
                    if let Some(renderer) = s.renderer.as_mut() {
                        renderer.resize(width, height);
                    }
                    drop(s);
                    render_now(&state_for_cb);
                } else {
                    try_init(
                        state_for_cb.clone(),
                        canvas_for_cb.clone(),
                        terrain_store.get_untracked(),
                        signals,
                        status,
                    );
                }
            });

            let observer = web_sys::ResizeObserver::new(closure.as_ref().unchecked_ref())
                .expect("ResizeObserver::new failed");
            observer.observe(&canvas);

            // クロージャ・observerともにこのパネルの生存期間ずっと必要なのでforgetする。
            closure.forget();
            std::mem::forget(observer);
        });
    }

    // --- Effect 2: 地形データ(TerrainStore)が届いたら初期化を試みる ---
    {
        let state = state.clone();
        Effect::new(move |_| {
            let Some(data) = terrain_store.get() else {
                return;
            };
            let Some(canvas_el) = canvas_ref.get_untracked() else {
                return;
            };
            let canvas: web_sys::HtmlCanvasElement = (*canvas_el)
                .clone()
                .dyn_into()
                .expect("canvas node_ref should be an HtmlCanvasElement");
            try_init(state.clone(), canvas, Some(data), signals, status);
        });
    }

    // --- Effect 3: OriginStateの変化に追従してメッシュを再計算する(DESIGN.md 3.3節) ---
    {
        let state = state.clone();
        Effect::new(move |_| {
            let Some(origin_state) = signals.origin.get() else {
                return;
            };
            let new_origin = Origin {
                lat_deg: origin_state.lat_deg,
                lon_deg: origin_state.lon_deg,
            };

            let mut s = state.borrow_mut();
            let (Some(terrain), Some(renderer)) = (s.terrain.as_ref(), s.renderer.as_ref()) else {
                return;
            };
            if s.mesh_origin == Some(new_origin) {
                return;
            }
            let new_mesh = mesh::build_mesh(terrain, &new_origin);
            renderer.update_vertices(&new_mesh);
            let camera = s.camera.to_camera(renderer.aspect_ratio());
            if let Err(e) = renderer.render(&camera) {
                log::error!("[terrain] re-render after origin change failed: {e}");
            }
            s.mesh_origin = Some(new_origin);
        });
    }

    // --- 自由視点カメラの操作(ドラッグ回転・ホイールズーム) ---
    const ORBIT_SENSITIVITY: f32 = 0.0075;

    let state_pd = state.clone();
    let on_pointer_down = move |ev: leptos::ev::PointerEvent| {
        {
            let mut s = state_pd.borrow_mut();
            s.dragging = true;
            s.last_x = ev.client_x() as f64;
            s.last_y = ev.client_y() as f64;
        }
        if let Some(target) = ev.target() {
            if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                let _ = el.set_pointer_capture(ev.pointer_id());
            }
        }
    };

    let state_pm = state.clone();
    let on_pointer_move = move |ev: leptos::ev::PointerEvent| {
        let should_render = {
            let mut s = state_pm.borrow_mut();
            if !s.dragging {
                false
            } else {
                let x = ev.client_x() as f64;
                let y = ev.client_y() as f64;
                let dx = (x - s.last_x) as f32;
                let dy = (y - s.last_y) as f32;
                s.last_x = x;
                s.last_y = y;
                s.camera.orbit(dx * ORBIT_SENSITIVITY, dy * ORBIT_SENSITIVITY);
                true
            }
        };
        if should_render {
            render_now(&state_pm);
        }
    };

    let state_pu = state.clone();
    let on_pointer_up = move |_ev: leptos::ev::PointerEvent| {
        state_pu.borrow_mut().dragging = false;
    };

    let state_wheel = state.clone();
    let on_wheel = move |ev: leptos::ev::WheelEvent| {
        ev.prevent_default();
        let factor = if ev.delta_y() > 0.0 { 1.12 } else { 1.0 / 1.12 };
        state_wheel.borrow_mut().camera.zoom(factor);
        render_now(&state_wheel);
    };

    let state_overview = state.clone();
    let on_preset_overview = move |_| {
        state_overview.borrow_mut().camera = OrbitCamera::preset(CameraPreset::Overview);
        render_now(&state_overview);
    };
    let state_side = state.clone();
    let on_preset_side = move |_| {
        state_side.borrow_mut().camera = OrbitCamera::preset(CameraPreset::Side);
        render_now(&state_side);
    };

    view! {
        <div class="terrain-view">
            <canvas
                node_ref=canvas_ref
                class="terrain-canvas"
                on:pointerdown=on_pointer_down
                on:pointermove=on_pointer_move
                on:pointerup=on_pointer_up.clone()
                on:pointercancel=on_pointer_up
                on:wheel=on_wheel
            ></canvas>
            <div class="terrain-view-controls">
                <button on:click=on_preset_overview title="俯瞰視点に切り替え">"俯瞰"</button>
                <button on:click=on_preset_side title="側面視点に切り替え">"側面"</button>
            </div>
            {move || {
                let s = status.get();
                (!s.is_empty()).then(|| view! { <p class="placeholder map-status">{s}</p> })
            }}
        </div>
    }
}
