//! 地形メッシュを描画する再利用可能なcanvasコンポーネント。地形データ本体は`TerrainStore`
//! context(1回だけフェッチ)、原点は`terrain::origin::OriginState` context、レーダー観測点は
//! `terrain::markers::RadarMarkersState` contextから読む(呼び出し側が`provide_context`する。
//! `sim3dview/README.md`参照)。自由視点カメラはドラッグで回転、ホイールでズーム。

use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::terrain::camera::{CameraPreset, OrbitCamera, ViewMode};
use crate::terrain::display::WaterVisibilityState;
use crate::terrain::loader::TerrainData;
use crate::terrain::markers::{self, RadarMarkersState};
use crate::terrain::mesh::{self, Origin};
use crate::terrain::origin::OriginState;
use crate::terrain::pick;
use crate::terrain::renderer::TerrainRenderer;
use crate::terrain::store::TerrainStore;

struct ViewState {
    renderer: Option<TerrainRenderer>,
    terrain: Option<Rc<TerrainData>>,
    /// 現在GPUにアップロードされているメッシュが基づいている原点。
    mesh_origin: Option<Origin>,
    camera: OrbitCamera,
    /// 現在の注視点の地表標高(ENU上座標、メートル)。原点の緯度経度における
    /// heightmapの値。カメラの`target`はここではなく`Vec3::new(0,0,target_up)`に
    /// 置くことで、ズームインしても地表に埋まらないようにする(camera.rs参照)。
    target_up: f32,
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
    origin_state: OriginState,
    status: RwSignal<String>,
    radar_markers: RadarMarkersState,
    water_visibility: WaterVisibilityState,
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
    let terrain_mesh = mesh::build_mesh(&data, &origin);
    // 注視点は原点の実際の地表標高に置く(Vec3::ZEROのままだと、原点が高山の
    // 斜面にある場合にズームインした際カメラが地面に埋まって真っ黒になる)。
    let target_up = mesh::sample_heightmap(&data, origin.lat_deg, origin.lon_deg).unwrap_or(0.0);

    wasm_bindgen_futures::spawn_local(async move {
        match TerrainRenderer::new(canvas, &terrain_mesh).await {
            Ok(renderer) => {
                {
                    let mut s = state.borrow_mut();
                    s.target_up = target_up;
                    s.camera.target.z = target_up;
                }
                renderer.set_show_water(water_visibility.0.get_untracked());
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
                drop(s);
                rebuild_markers(&state, radar_markers);
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

fn render_now(state: &Rc<RefCell<ViewState>>) {
    let s = state.borrow();
    if let Some(renderer) = s.renderer.as_ref() {
        let camera = s.camera.to_camera(renderer.aspect_ratio());
        if let Err(e) = renderer.render(&camera) {
            log::error!("[terrain] render failed: {e}");
        }
    }
}

/// レーダー観測点マーカー・見通し範囲の覆域ドームのジオメトリを、現在の地形・原点・
/// マーカー一覧・選択状態から作り直してGPUバッファへ反映する。原点変更時
/// (メッシュ再構築後)・マーカー追加/削除/選択変更時に呼ぶ。描画自体は呼び出し側で
/// `render_now`すること。
fn rebuild_markers(state: &Rc<RefCell<ViewState>>, radar_markers: RadarMarkersState) {
    let mut s = state.borrow_mut();
    let (Some(terrain), Some(mesh_origin), mode) =
        (s.terrain.clone(), s.mesh_origin, s.camera.mode)
    else {
        return;
    };
    let Some(renderer) = s.renderer.as_mut() else {
        return;
    };
    let marker_list = radar_markers.markers.get_untracked();
    let selected = radar_markers.selected.get_untracked();
    let mut marker_vertices = markers::build_marker_geometry(&terrain, &mesh_origin, &marker_list, selected);
    // 覆域表示は3D(半球ドーム)と2D(指定高度での探知可能領域)で見せ方自体が別物なので、
    // 同じ描画パイプライン(update_dome)に対してモードに応じて別のジオメトリを渡す。
    let coverage_vertices = match mode {
        ViewMode::ThreeD => markers::build_dome_surface_geometry(&terrain, &mesh_origin, &marker_list, selected),
        ViewMode::TwoD => {
            let altitude_m = radar_markers.coverage_altitude_m.get_untracked();
            // 塗り(半透明、アルファ0.22)だけでは地図上で見えにくいため、マーカーと同じ
            // 不透明LineListのバッファへ輪郭線も追加する(`push_coverage_outline`参照)。
            marker_vertices.extend(markers::build_coverage_outline_geometry(
                &terrain,
                &mesh_origin,
                &marker_list,
                selected,
                altitude_m,
            ));
            markers::build_coverage_area_geometry(&terrain, &mesh_origin, &marker_list, selected, altitude_m)
        }
    };
    renderer.update_markers(&marker_vertices);
    renderer.update_dome(&coverage_vertices);
}

#[component]
pub fn TerrainView(preset: CameraPreset) -> impl IntoView {
    let canvas_ref: NodeRef<leptos::html::Canvas> = NodeRef::new();
    let status = RwSignal::new("地形データを読み込み中...".to_string());
    let origin_state = use_context::<OriginState>().expect("OriginState context not found");
    let terrain_store = use_context::<TerrainStore>().expect("TerrainStore context not found");
    let radar_markers = use_context::<RadarMarkersState>().expect("RadarMarkersState context not found");
    // 未提供でもデフォルト(表示)で動作するよう、他のcontextと違いunwrap_or_defaultにしてある
    // (既存の利用側コードに影響を与えない、後から追加したオプション機能のため)。
    let water_visibility = use_context::<WaterVisibilityState>().unwrap_or_default();

    terrain_store.ensure_loaded();

    let state = Rc::new(RefCell::new(ViewState {
        renderer: None,
        terrain: None,
        mesh_origin: None,
        // target_upは地形データ取得後(try_init)に実際の標高で上書きする。
        camera: OrbitCamera::preset(preset, 0.0),
        target_up: 0.0,
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

            // canvasの内部解像度(width/height)を実際のCSSサイズへ合わせ、必要なら
            // レンダラーを初期化/リサイズする。ResizeObserverのコールバックと、
            // 下の`visibilitychange`ハンドラの両方から呼べるよう共通化してある。
            let apply_size = {
                let canvas = canvas.clone();
                let state = state.clone();
                move |width: u32, height: u32| {
                    if width == 0 || height == 0 {
                        return;
                    }
                    canvas.set_width(width);
                    canvas.set_height(height);

                    let has_renderer = state.borrow().renderer.is_some();
                    if has_renderer {
                        let mut s = state.borrow_mut();
                        if let Some(renderer) = s.renderer.as_mut() {
                            renderer.resize(width, height);
                        }
                        drop(s);
                        render_now(&state);
                    } else {
                        try_init(
                            state.clone(),
                            canvas.clone(),
                            terrain_store.get_untracked(),
                            origin_state,
                            status,
                            radar_markers,
                            water_visibility,
                        );
                    }
                }
            };

            let apply_size_for_resize = apply_size.clone();
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
                apply_size_for_resize(width, height);
            });

            let observer = web_sys::ResizeObserver::new(closure.as_ref().unchecked_ref())
                .expect("ResizeObserver::new failed");
            observer.observe(&canvas);

            // ブラウザは非表示(バックグラウンド)タブに対してResizeObserverの通知自体を
            // スロットリング(完全停止)することがある(CLAUDE.md「スプリッタードラッグ時の
            // リサイズ追従」で既知)。ページが非表示のまま初回マウントされると、canvasの
            // 内部解像度がHTML既定値(300×150)のまま一度も更新されず、その後CSSで
            // 実際の表示サイズへ引き伸ばされることでアスペクト比が崩れ、地形の一部
            // (特に画面端寄り・低標高の周辺部)が視野から欠けて見える不具合になっていた。
            // ws.rsのWebSocket再接続と同じPage Visibility APIのパターンで、タブが可視に
            // 戻った時点で実際のCSSサイズを取り直し、ズレていれば取り込み直す。
            let canvas_for_visibility = canvas.clone();
            let visibility_closure = Closure::<dyn FnMut()>::new(move || {
                let hidden = web_sys::window()
                    .and_then(|w| w.document())
                    .map(|d| d.hidden())
                    .unwrap_or(false);
                if hidden {
                    return;
                }
                let rect = canvas_for_visibility.get_bounding_client_rect();
                let width = rect.width().round().max(0.0) as u32;
                let height = rect.height().round().max(0.0) as u32;
                if width != canvas_for_visibility.width() || height != canvas_for_visibility.height() {
                    apply_size(width, height);
                }
            });
            if let Some(document) = web_sys::window().and_then(|w| w.document()) {
                let _ = document.add_event_listener_with_callback(
                    "visibilitychange",
                    visibility_closure.as_ref().unchecked_ref(),
                );
            }

            // クロージャ・observerともにこのパネルの生存期間ずっと必要なのでforgetする。
            closure.forget();
            visibility_closure.forget();
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
            try_init(state.clone(), canvas, Some(data), origin_state, status, radar_markers, water_visibility);
        });
    }

    // --- Effect 3: OriginStateの変化に追従してメッシュを再計算する(DETAILED_DESIGN.md 3.3節) ---
    {
        let state = state.clone();
        Effect::new(move |_| {
            let Some(new_origin) = origin_state.0.get() else {
                return;
            };

            let mut s = state.borrow_mut();
            let Some(terrain) = s.terrain.clone() else {
                return;
            };
            if s.renderer.is_none() {
                return;
            }
            if s.mesh_origin == Some(new_origin) {
                return;
            }
            let new_mesh = mesh::build_mesh(&terrain, &new_origin);
            // 注視点の高さも新しい原点の地表標高へ更新する(古い標高のままだと、
            // 原点移動後にズームインした際カメラが地面に埋まって真っ黒になりうる)。
            s.target_up =
                mesh::sample_heightmap(&terrain, new_origin.lat_deg, new_origin.lon_deg).unwrap_or(0.0);
            s.camera.target.z = s.target_up;
            let renderer = s.renderer.as_ref().expect("checked is_some above");
            renderer.update_vertices(&new_mesh);
            let camera = s.camera.to_camera(renderer.aspect_ratio());
            if let Err(e) = renderer.render(&camera) {
                log::error!("[terrain] re-render after origin change failed: {e}");
            }
            s.mesh_origin = Some(new_origin);
            drop(s);
            // マーカー・覆域リングも新しい原点基準のENU座標へ再変換する。
            rebuild_markers(&state, radar_markers);
            render_now(&state);
        });
    }

    // --- Effect 4: レーダー観測点(一覧・選択状態・覆域高度)の変化に追従して3D描画を更新する ---
    // 覆域高度(coverage_altitude_m)はメニューの「覆域高度設定...」フローティングパネル
    // (`coverage_altitude_dialog.rs`)側で編集されるため、ここでの購読が2Dモードの
    // 覆域表示を更新する唯一の経路になる。
    {
        let state = state.clone();
        Effect::new(move |_| {
            let _ = radar_markers.markers.get();
            let _ = radar_markers.selected.get();
            let _ = radar_markers.coverage_altitude_m.get();
            rebuild_markers(&state, radar_markers);
            render_now(&state);
        });
    }

    // --- Effect 5: 表示メニュー「海を表示」チェックボックスの変化に追従して描画を更新する ---
    {
        let state = state.clone();
        Effect::new(move |_| {
            let show = water_visibility.0.get();
            let has_renderer = state.borrow().renderer.is_some();
            if has_renderer {
                if let Some(renderer) = state.borrow().renderer.as_ref() {
                    renderer.set_show_water(show);
                }
                render_now(&state);
            }
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
                match s.camera.mode {
                    ViewMode::ThreeD => {
                        s.camera.orbit(dx * ORBIT_SENSITIVITY, dy * ORBIT_SENSITIVITY);
                    }
                    ViewMode::TwoD => {
                        // 正射影の画面縦幅(distance)と実際のcanvas高さ(ピクセル)の比から、
                        // 画面上のドラッグ量をワールド座標(メートル)の移動量へ変換する。
                        let canvas_h = s.renderer.as_ref().map(|r| r.canvas_height_px()).unwrap_or(1).max(1);
                        let world_per_px = s.camera.distance / canvas_h as f32;
                        // 画面上は北=上(up=Vec3::Y)なので、上方向のドラッグ(dy<0)は北への移動。
                        s.camera.pan(dx * world_per_px, -dy * world_per_px);
                    }
                }
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

    // 地図上への右クリックでレーダー観測点(見通し範囲)を追加する。ブラウザ既定の
    // コンテキストメニューは出さない。
    let state_ctx = state.clone();
    let on_context_menu = move |ev: leptos::ev::MouseEvent| {
        ev.prevent_default();
        let Some(target) = ev.target() else {
            return;
        };
        let Ok(canvas) = target.dyn_into::<web_sys::HtmlCanvasElement>() else {
            return;
        };
        let rect = canvas.get_bounding_client_rect();
        let x = ev.client_x() as f32 - rect.left() as f32;
        let y = ev.client_y() as f32 - rect.top() as f32;
        let canvas_w = canvas.width() as f32;
        let canvas_h = canvas.height() as f32;

        let s = state_ctx.borrow();
        let (Some(terrain), Some(mesh_origin), Some(renderer)) =
            (s.terrain.clone(), s.mesh_origin, s.renderer.as_ref())
        else {
            return;
        };
        let camera = s.camera.to_camera(renderer.aspect_ratio());
        let hit = pick::pick_lat_lon(&terrain, &mesh_origin, &camera, x, y, canvas_w, canvas_h);
        drop(s);
        if let Some((lat, lon)) = hit {
            radar_markers.add(lat, lon);
        }
    };

    // 2D/3D表示モード切り替え。実際のモードは`state`(camera.mode)が持つが、ボタン表示・
    // 条件分岐(見た目の切り替え)にはリアクティブなsignalが必要なので、ここで複製して持つ。
    let view_mode = RwSignal::new(ViewMode::ThreeD);

    let state_toggle = state.clone();
    let on_toggle_view_mode = move |_| {
        let new_mode = match view_mode.get_untracked() {
            ViewMode::ThreeD => ViewMode::TwoD,
            ViewMode::TwoD => ViewMode::ThreeD,
        };
        state_toggle.borrow_mut().camera.mode = new_mode;
        view_mode.set(new_mode);
        rebuild_markers(&state_toggle, radar_markers);
        render_now(&state_toggle);
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
                on:contextmenu=on_context_menu
            ></canvas>
            <div class="terrain-view-controls">
                <button on:click=on_toggle_view_mode title="2D/3D表示切り替え">
                    {move || if view_mode.get() == ViewMode::ThreeD { "2D表示に切替" } else { "3D表示に切替" }}
                </button>
            </div>
            {move || {
                let s = status.get();
                (!s.is_empty()).then(|| view! { <p class="placeholder map-status">{s}</p> })
            }}
        </div>
    }
}
