//! 地形メッシュを描画する再利用可能なcanvasコンポーネント。地形データ本体は`TerrainStore`
//! context(1回だけフェッチ)、原点は`terrain::origin::OriginState` context、レーダー観測点は
//! `terrain::markers::RadarMarkersState` contextから読む(呼び出し側が`provide_context`する。
//! `sim3dview/README.md`参照)。自由視点カメラはドラッグで回転、ホイールでズーム、
//! 3DモードではShift+ドラッグで注視点(中心点)を平行移動できる(シミュレーション原点
//! [`terrain::origin::OriginState`]は変更しない)。`terrain::recenter::RecenterRequestState`
//! contextの通知(表示メニューの「中心点を原点に戻す」ボタン)で中心点を原点へ戻す。
//! `terrain::origin_pick::OriginPickState` contextが提供されていて`active`の間は、地図の
//! 左クリック(ドラッグではない単発クリック)の地点を原点として`on_pick`へ渡す。

use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::terrain::camera::{CameraPreset, OrbitCamera, ViewMode};
use crate::terrain::loader::TerrainData;
use crate::terrain::markers::{self, RadarMarkersState};
use crate::terrain::mesh::{self, Origin};
use crate::terrain::origin::OriginState;
use crate::terrain::origin_pick::OriginPickState;
use crate::terrain::pick;
use crate::terrain::recenter::RecenterRequestState;
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
    /// ボタンを押した位置。離した位置との距離で「クリック」か「ドラッグ」かを判別する。
    down_x: f64,
    down_y: f64,
}

/// この距離(CSSピクセル)未満の移動なら、ドラッグではなく単発クリックとして扱う。
const CLICK_MAX_MOVE_PX: f64 = 5.0;

/// canvas上の画面座標(client座標)が指す地表の緯度経度。地形データ範囲外・未初期化ならNone。
fn pick_at_client(
    state: &Rc<RefCell<ViewState>>,
    canvas: &web_sys::HtmlCanvasElement,
    client_x: f64,
    client_y: f64,
) -> Option<(f64, f64)> {
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
    // 未提供でもデフォルト(何もしない)で動作するよう、他のcontextと違いunwrap_or_defaultにしてある
    // (既存の利用側コードに影響を与えない、後から追加したオプション機能のため)。
    let recenter_request = use_context::<RecenterRequestState>().unwrap_or_default();
    // 未提供なら「クリックで原点指定」機能なしで動作する(上と同じく後付けのオプション機能)。
    let origin_pick = use_context::<OriginPickState>();

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
        down_x: 0.0,
        down_y: 0.0,
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
            // スロットリング(完全停止)することがある(DEVELOPMENT_HISTORY.md「スプリッタードラッグ時の
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
            try_init(state.clone(), canvas, Some(data), origin_state, status, radar_markers);
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

    // --- Effect 6: 表示メニュー「中心点を原点に戻す」ボタンの通知を受けて注視点をリセットする ---
    // 中心点(camera.target)はShift+ドラッグ(3D)・通常ドラッグ(2D)で動かせるが、これは
    // カメラのローカル状態のみを動かす操作でシミュレーション原点(OriginState)には
    // 触れていない。ここでのリセットも同様にOriginStateへは一切触れず、camera.targetを
    // ENU座標(0,0,原点の実際の地表標高)へ戻すだけ。
    {
        let state = state.clone();
        Effect::new(move |_| {
            let count = recenter_request.0.get();
            if count == 0 {
                return; // 初期値0はボタン未クリックの状態なので無視する。
            }
            let mut s = state.borrow_mut();
            if s.renderer.is_none() {
                return;
            }
            s.camera.target.x = 0.0;
            s.camera.target.y = 0.0;
            s.camera.target.z = s.target_up;
            drop(s);
            render_now(&state);
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
            s.down_x = s.last_x;
            s.down_y = s.last_y;
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
                        if ev.shift_key() {
                            // Shift+ドラッグ: 回転ではなく注視点(中心点)を平行移動する
                            // (「原点は変えないでね」との要望通り、OriginStateには触れない)。
                            let canvas_h =
                                s.renderer.as_ref().map(|r| r.canvas_height_px()).unwrap_or(1).max(1);
                            s.camera.pan_orbit_target(dx, dy, canvas_h as f32);
                            // 移動先の実際の地表標高へtarget.zを更新する(古い標高のまま
                            // だと、原点変更時と同様にズームインした際カメラが地面に
                            // 埋まって真っ黒になりうる)。
                            if let (Some(terrain), Some(mesh_origin)) =
                                (s.terrain.clone(), s.mesh_origin)
                            {
                                let transform =
                                    mesh::EnuTransform::new(&mesh_origin, &terrain.metadata.ellipsoid);
                                let (lat, lon) = transform
                                    .inverse(s.camera.target.x as f64, s.camera.target.y as f64);
                                if let Some(elev) = mesh::sample_heightmap(&terrain, lat, lon) {
                                    s.camera.target.z = elev;
                                }
                            }
                        } else {
                            s.camera.orbit(dx * ORBIT_SENSITIVITY, dy * ORBIT_SENSITIVITY);
                        }
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

    let state_pc = state.clone();
    let on_pointer_cancel = move |_ev: leptos::ev::PointerEvent| {
        state_pc.borrow_mut().dragging = false;
    };

    // 「クリックで原点指定」モード中に、ドラッグではない左クリックが離されたら、その地点を
    // 原点として`on_pick`へ渡してモードを解除する(通常のドラッグ=回転・パンは従来通り動く)。
    let state_pu = state.clone();
    let on_pointer_up = move |ev: leptos::ev::PointerEvent| {
        let (down_x, down_y) = {
            let mut s = state_pu.borrow_mut();
            s.dragging = false;
            (s.down_x, s.down_y)
        };
        let Some(pick_state) = origin_pick else {
            return;
        };
        if !pick_state.active.get_untracked() || ev.button() != 0 {
            return;
        }
        let moved = (ev.client_x() as f64 - down_x).hypot(ev.client_y() as f64 - down_y);
        if moved >= CLICK_MAX_MOVE_PX {
            return;
        }
        let Some(target) = ev.target() else {
            return;
        };
        let Ok(canvas) = target.dyn_into::<web_sys::HtmlCanvasElement>() else {
            return;
        };
        // 地形データ範囲外(海の外側など)をクリックした場合は、モードを維持して指定し直せるようにする。
        if let Some((lat, lon)) =
            pick_at_client(&state_pu, &canvas, ev.client_x() as f64, ev.client_y() as f64)
        {
            pick_state.active.set(false);
            pick_state.on_pick.run((lat, lon));
        }
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
        if let Some((lat, lon)) =
            pick_at_client(&state_ctx, &canvas, ev.client_x() as f64, ev.client_y() as f64)
        {
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

    let pick_active = move || origin_pick.is_some_and(|p| p.active.get());

    view! {
        <div class="terrain-view">
            <canvas
                node_ref=canvas_ref
                class="terrain-canvas"
                class:origin-pick-active=pick_active
                on:pointerdown=on_pointer_down
                on:pointermove=on_pointer_move
                on:pointerup=on_pointer_up
                on:pointercancel=on_pointer_cancel
                on:wheel=on_wheel
                on:contextmenu=on_context_menu
            ></canvas>
            {move || {
                origin_pick.filter(|p| p.active.get()).map(|p| {
                    view! {
                        <div class="origin-pick-hint">
                            <span>"原点にする地点をクリックしてください"</span>
                            <button on:click=move |_| p.active.set(false)>"キャンセル"</button>
                        </div>
                    }
                })
            }}
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
