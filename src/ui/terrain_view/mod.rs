//! 地形メッシュを描画する再利用可能なcanvasコンポーネント。地形データ本体は`TerrainStore`
//! context(1回だけフェッチ)、原点は`terrain::origin::OriginState` context、レーダー観測点は
//! `terrain::markers::RadarMarkersState` contextから読む(呼び出し側が`provide_context`する。
//! `README.md`参照)。自由視点カメラはドラッグで回転、ホイールでズーム、
//! 3DモードではShift+ドラッグで注視点(中心点)を平行移動できる(シミュレーション原点
//! [`crate::terrain::origin::OriginState`]は変更しない)。`terrain::recenter::RecenterRequestState`
//! contextの通知(表示メニューの「中心点を原点に戻す」ボタン)で中心点を原点へ戻す。
//! `terrain::origin_pick::OriginPickState` contextが提供されていて`active`の間は、地図の
//! 左クリック(ドラッグではない単発クリック)の地点を原点として`on_pick`へ渡す。
//! `terrain::draw_tool::DrawToolState` contextが提供されていてツールを選んでいる間は、地図の
//! 左クリックで図形の点を置く(カーソル移動で仮の図形が追従、ダブルクリック/Enterで多角形・折れ線を確定、
//! 右クリック/Backspaceで1つ戻す、Escで終了)。
//! 地図の右クリックは、`MapMenuState`(と`ui::context_menu::ContextMenuState`)が提供されていれば、
//! アプリが決めた項目の右クリックメニューを出す(提供されていなければ、その地点にレーダー観測点を追加する)。
mod capture;
mod coverage;
mod frame;
mod frame_request;
mod input;
mod labels;
mod lod_driver;
mod models;
mod overlay;
mod picking;
mod resize;
mod state;

use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use self::{frame::*, overlay::*, picking::*, state::*};
use crate::terrain::camera::{CameraPreset, OrbitCamera, ViewMode};
use crate::terrain::capture::CaptureState;
use crate::terrain::draw_tool::DrawToolState;
use crate::terrain::drawing::DrawingState;
use crate::terrain::hillshade::HillshadeState;
use crate::terrain::markers::RadarMarkersState;
use crate::terrain::models::ModelsState;
use crate::terrain::origin::OriginState;
use crate::terrain::origin_pick::OriginPickState;
use crate::terrain::recenter::RecenterRequestState;
use crate::terrain::store::TerrainStore;
use crate::terrain::tracks::TracksState;
use crate::ui::context_menu::{ContextMenuState, MapMenuState, MapMenuTarget};
use crate::ui::util::client_xy;

type PendingHover = Rc<RefCell<Option<(web_sys::HtmlCanvasElement, (f64, f64))>>>;

#[component]
pub fn TerrainView(preset: CameraPreset) -> impl IntoView {
    let canvas_ref: NodeRef<leptos::html::Canvas> = NodeRef::new();
    let status = RwSignal::new("地形データを読み込み中...".to_string());
    let origin_state = use_context::<OriginState>().expect("OriginState context not found");
    let terrain_store = use_context::<TerrainStore>().expect("TerrainStore context not found");
    let radar_markers =
        use_context::<RadarMarkersState>().expect("RadarMarkersState context not found");
    // 未提供でもデフォルト(何もしない)で動作するよう、他のcontextと違いunwrap_or_defaultにしてある
    // (既存の利用側コードに影響を与えない、後から追加したオプション機能のため)。
    let recenter_request = use_context::<RecenterRequestState>().unwrap_or_default();
    // 未提供なら既定(陰影ON)のまま切り替えなしで動作する(上と同じく後付けのオプション機能)。
    let hillshade = use_context::<HillshadeState>().unwrap_or_default();
    // 未提供ならスクリーンショット・画面録画なしで動作する(同上。`terrain::capture`)。
    let capture = use_context::<CaptureState>().unwrap_or_default();
    // 未提供なら「クリックで原点指定」機能なしで動作する(上と同じく後付けのオプション機能)。
    let origin_pick = use_context::<OriginPickState>();
    // 未提供なら作図なしで動作する(上と同じく後付けのオプション機能)。
    let drawings = use_context::<DrawingState>().unwrap_or_default();
    // 未提供なら図形の対話作成なしで動作する(同上)。
    let draw_tool = use_context::<DrawToolState>();
    // 未提供なら航跡表示なしで動作する(同上)。
    let tracks = use_context::<TracksState>().unwrap_or_default();
    // 未提供なら3Dモデルなし(シンボルだけ)で動作する(同上。`terrain::models`)。
    let models = use_context::<ModelsState>().unwrap_or_default();
    // 両方が提供されていれば、地図の右クリックで右クリックメニューを出す(未提供なら従来どおり観測点の追加)。
    let context_menu = use_context::<ContextMenuState>();
    let map_menu = use_context::<MapMenuState>();
    let labels_ref: NodeRef<leptos::html::Div> = NodeRef::new();

    terrain_store.ensure_loaded();

    let state = Rc::new(RefCell::new(ViewState {
        renderer: None,
        terrain: None,
        mesh_origin: None,
        // target_upは地形データ取得後(try_init)に実際の標高で上書きする。
        camera: OrbitCamera::preset(preset, 0.0),
        target_up: 0.0,
        initializing: false,
        frame_request: Default::default(),
        drag: Default::default(),
        radar_markers,
        drawings,
        tracks,
        models: models::ModelsView::new(models),
        labels_ref,
        labels: Vec::new(),
        pick_anchors: Vec::new(),
        hillshade,
        lod: LodState::default(),
        coverage: Default::default(),
    }));

    // --- Effect 1: canvasのマウント + ResizeObserver(初回サイズ確定・以後のリサイズ追従) ---
    {
        let state = state.clone();
        Effect::new(move |_| {
            let Some(canvas) = canvas_ref.get() else {
                return;
            };
            // canvasの内部解像度(width/height)を実際のCSSサイズへ合わせ、必要なら
            // レンダラーを初期化/リサイズする。
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
                        // 画面座標の作図は、canvasの大きさで角の位置が変わる。
                        rebuild_drawings(&state);
                        render_now(&state);
                    } else {
                        try_init(
                            state.clone(),
                            canvas.clone(),
                            terrain_store.get_untracked(),
                            origin_state,
                            status,
                        );
                    }
                }
            };
            resize::observe_canvas_size(&canvas, apply_size);
        });
    }

    // --- Effect 2: 地形データ(TerrainStore)が届いたら初期化を試みる ---
    {
        let state = state.clone();
        Effect::new(move |_| {
            let Some(data) = terrain_store.get() else {
                return;
            };
            let Some(canvas) = canvas_ref.get_untracked() else {
                return;
            };
            try_init(state.clone(), canvas, Some(data), origin_state, status);
        });
    }

    // --- Effect 3: OriginStateの変化に追従してメッシュを再計算する(DETAILED_DESIGN.md 3.3節) ---
    {
        let state = state.clone();
        Effect::new(move |_| {
            if let Some(new_origin) = origin_state.0.get() {
                change_origin(&state, new_origin);
            }
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
            let _ = radar_markers.show_all_coverage.get();
            rebuild_markers(&state);
            render_now(&state);
        });
    }

    // --- Effect 5: 作図(`terrain::drawing`)の一覧の変化に追従して描き直す ---
    // 図形の追加・削除・書き換え(位置・大きさ・色・表示/非表示)はすべて`items`の更新なので、購読はこれだけ。
    {
        let state = state.clone();
        Effect::new(move |_| {
            let _ = drawings.items.get();
            rebuild_drawings(&state);
            render_now(&state);
        });
    }

    // --- Effect 5b: 航跡(`terrain::tracks`)の一覧・表示設定の変化に追従して描き直す ---
    // トラックは高頻度(数十Hz)で更新されうるので、LODの更新は予約せず(`render_frame`)、描くだけにする。
    {
        let state = state.clone();
        Effect::new(move |_| {
            let _ = tracks.entries.get();
            let _ = tracks.show_labels.get();
            let _ = tracks.show_trails.get();
            let _ = tracks.show_altitude_lines.get();
            let _ = tracks.selected.get();
            rebuild_tracks(&state);
            render_frame(&state);
        });
    }

    // --- Effect 5c: 3Dモデル(`terrain::models`)の表示設定・モデルの登録の変化に追従して描き直す ---
    // 何をモデルで描くかは描画のたびに決まる(`render_frame`→`update_models`)ので、購読して描き直すだけ。
    {
        let state = state.clone();
        Effect::new(move |_| {
            let _ = models.mode.get();
            let _ = models.switch_distance_m.get();
            let _ = models.min_screen_px.get();
            let _ = models.sources.get();
            render_frame(&state);
        });
    }

    // --- Effect 6: 表示メニュー「中心点を原点に戻す」・右クリックメニュー「ここを中心点にする」の通知を受けて注視点を動かす ---
    // 中心点(camera.target)はShift+ドラッグ(3D)・通常ドラッグ(2D)で動かせるが、これは
    // カメラのローカル状態のみを動かす操作でシミュレーション原点(OriginState)には
    // 触れていない。ここでのリセットも同様にOriginStateへは一切触れず、camera.targetを
    // ENU座標(0,0,原点の実際の地表標高)へ戻すだけ。
    {
        let state = state.clone();
        Effect::new(move |_| {
            let count = recenter_request.count.get();
            if count == 0 {
                return; // 初期値0はボタン未クリックの状態なので無視する。
            }
            recenter(&state, recenter_request.target());
        });
    }

    // --- Effect 7: 表示メニュー「陰影表示」のON/OFFをレンダラーへ反映する ---
    // 陰影は法線と光源からシェーダーで掛けるので、メッシュの作り直しは不要(uniformを変えて描き直すだけ)。
    {
        let state = state.clone();
        Effect::new(move |_| {
            let enabled = hillshade.enabled.get();
            let mut s = state.borrow_mut();
            let Some(renderer) = s.renderer.as_mut() else {
                return; // レンダラー作成前はtry_initが初期値を設定する。
            };
            renderer.set_hillshade(enabled);
            drop(s);
            render_now(&state);
        });
    }

    // --- Effect 8: VAB等のスクリーンショットボタンの要求を受けてcanvasをPNG保存する ---
    // `recenter_request`(Effect 6)と同じ「要求カウンタが増えたら実行」パターン。
    // 実際のtoBlob呼び出し・ダウンロードは`capture`モジュール(web_sys直叩き)に任せる。
    {
        let state = state.clone();
        Effect::new(move |_| {
            let count = capture.screenshot_requests.get();
            if count == 0 {
                return; // 初期値0はボタン未クリックの状態なので無視する。
            }
            let Some(canvas) = canvas_ref.get_untracked() else {
                return;
            };
            // 通常描画は次フレームまで集約するため、保存時は最新状態を先に描く。
            draw_frame(&state);
            capture::save_screenshot(&canvas);
        });
    }

    // --- Effect 9: VAB等の録画ボタンの開始/停止要求を受けてMediaRecorderを出し入れする ---
    // `recording_requested`はトグル(bool)なので、Effect 6/8と違い「変化したら」ではなく
    // 「trueなのに録画中でない/falseなのに録画中」というズレを直す形で書く(録画中かどうかの
    // 実体(`web_sys::MediaRecorder`)はこのEffectの外の`Rc<RefCell<..>>`に持たせ、次回の
    // Effect実行(=次の要求)まで生かしておく)。
    {
        let recording: Rc<RefCell<Option<capture::Recording>>> = Rc::new(RefCell::new(None));
        Effect::new(move |_| {
            let requested = capture.recording_requested.get();
            capture::sync_recording(&mut recording.borrow_mut(), capture, requested, || {
                canvas_ref.get_untracked()
            });
        });
    }

    // --- 自由視点カメラの操作(ドラッグ回転・ホイールズーム) ---
    let state_pd = state.clone();
    let on_pointer_down = move |ev: leptos::ev::PointerEvent| {
        let (x, y) = client_xy(&ev);
        state_pd.borrow_mut().drag.begin(ev.pointer_id(), x, y);
        if let Some(target) = ev.target() {
            if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                let _ = el.set_pointer_capture(ev.pointer_id());
            }
        }
    };

    let state_pm = state.clone();
    // 次のフレームで反映する予定のカーソル位置(canvasとclient座標)。
    let hover_pending: PendingHover = Rc::new(RefCell::new(None));
    let on_pointer_move = move |ev: leptos::ev::PointerEvent| {
        let pos = client_xy(&ev);
        // 図形の作成中は、カーソルの指す地点へ仮の図形の先端を追従させる(ドラッグ中は動かさない)。
        // 仮の図形を更新するたびに作図全体の再構築+描画が走るので、マウス移動は1フレームに1回へまとめる。
        if let Some(tool) = draw_tool.filter(|t| t.wants_hover()) {
            if !state_pm.borrow().drag.is_active() {
                if let Some(canvas) = ev
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::HtmlCanvasElement>().ok())
                {
                    let already_scheduled =
                        hover_pending.borrow_mut().replace((canvas, pos)).is_some();
                    if !already_scheduled {
                        let hover_pending = hover_pending.clone();
                        let state = state_pm.clone();
                        request_animation_frame(move || {
                            let pending = hover_pending.borrow_mut().take();
                            if let Some((canvas, pos)) = pending {
                                if let Some((lat, lon)) = pick_at_client(&state, &canvas, pos) {
                                    tool.set_hover(lat, lon);
                                }
                            }
                        });
                    }
                }
            }
        }
        let should_render = {
            let mut s = state_pm.borrow_mut();
            match s.drag.update(ev.pointer_id(), pos.0, pos.1) {
                Some(update) => {
                    let (dx, dy) = (update.delta.0 as f32, update.delta.1 as f32);
                    input::apply_drag(&mut s, dx, dy, ev.shift_key());
                    true
                }
                None => false,
            }
        };
        if should_render {
            render_now(&state_pm);
        }
    };

    let state_pc = state.clone();
    let on_pointer_cancel = move |ev: leptos::ev::PointerEvent| {
        state_pc.borrow_mut().drag.cancel(ev.pointer_id());
    };

    // ドラッグではない左クリックが離されたとき:
    // - 「クリックで原点指定」モード中なら、その地点を原点として`on_pick`へ渡してモードを解除する
    // - 図形の作成ツールを選んでいれば、その地点を図形の点として置く(`terrain::draw_tool`)
    // - そうでなければ、クリックしたシンボルの航跡(`terrain::tracks`)を選択する(何もない所なら選択解除)
    // (通常のドラッグ=回転・パンは従来通り動く)
    let state_pu = state.clone();
    let on_pointer_up = move |ev: leptos::ev::PointerEvent| {
        let pos = client_xy(&ev);
        let drag_end = state_pu
            .borrow_mut()
            .drag
            .end(ev.pointer_id(), pos.0, pos.1);
        if ev.button() != 0 {
            return;
        }
        if drag_end.is_none_or(|drag| drag.distance() >= CLICK_MAX_MOVE_PX) {
            return;
        }
        let Some(target) = ev.target() else {
            return;
        };
        let Ok(canvas) = target.dyn_into::<web_sys::HtmlCanvasElement>() else {
            return;
        };
        let origin_pick = origin_pick.filter(|p| p.active.get_untracked());
        let draw_tool = draw_tool.filter(|t| t.tool.get_untracked().is_some());
        match (origin_pick, draw_tool) {
            // 地形データ範囲外(海の外側など)をクリックした場合は、モードを維持して指定し直せるようにする。
            (Some(pick_state), _) => {
                if let Some((lat, lon)) = pick_at_client(&state_pu, &canvas, pos) {
                    pick_state.active.set(false);
                    pick_state.on_pick.run((lat, lon));
                }
            }
            // 図形の作成中。地形データ範囲外のクリックは無視する(点を置き直せる)。
            (None, Some(tool)) => {
                if let Some((lat, lon)) = pick_at_client(&state_pu, &canvas, pos) {
                    tool.click(lat, lon);
                }
            }
            (None, None) => tracks.select(pick_track_at_client(&state_pu, &canvas, pos)),
        }
    };

    let state_wheel = state.clone();
    let on_wheel = move |ev: leptos::ev::WheelEvent| {
        ev.prevent_default();
        let factor = if ev.delta_y() > 0.0 { 1.12 } else { 1.0 / 1.12 };
        state_wheel.borrow_mut().camera.zoom(factor);
        render_now(&state_wheel);
    };

    // 地図上への右クリック。ブラウザ既定のコンテキストメニューは出さない。
    // - 図形の作成中: 置いた点を1つ戻す
    // - 右クリックメニュー(`MapMenuState`)があれば、その地点・シンボルに対するメニューを出す
    // - なければ、その地点にレーダー観測点(見通し範囲)を追加する
    let state_ctx = state.clone();
    let on_context_menu = move |ev: leptos::ev::MouseEvent| {
        ev.prevent_default();
        // 図形の作成中は、観測点の追加ではなく「置いた点を1つ戻す」に使う。
        if let Some(tool) = draw_tool.filter(|t| t.tool.get_untracked().is_some()) {
            tool.undo();
            return;
        }
        let Some(target) = ev.target() else {
            return;
        };
        let Ok(canvas) = target.dyn_into::<web_sys::HtmlCanvasElement>() else {
            return;
        };
        let pos = client_xy(&ev);
        if let (Some(menu), Some(map_menu)) = (context_menu, map_menu) {
            let position = pick_at_client(&state_ctx, &canvas, pos);
            let track = pick_track_at_client(&state_ctx, &canvas, pos);
            if track.is_some() {
                tracks.select(track);
            }
            if position.is_some() || track.is_some() {
                menu.show(
                    pos.0,
                    pos.1,
                    map_menu.0.run(MapMenuTarget { position, track }),
                );
            }
            return;
        }
        if let Some((lat, lon)) = pick_at_client(&state_ctx, &canvas, pos) {
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
        rebuild_markers(&state_toggle);
        rebuild_tracks(&state_toggle);
        render_now(&state_toggle);
    };

    // ダブルクリックで多角形・折れ線を確定する(1回目・2回目のクリックは`on_pointer_up`が点として置いたあと。
    // 2回目は直前の点とほぼ同じ位置なので`DrawToolState::click`が無視する)。
    let on_dbl_click = move |_ev: leptos::ev::MouseEvent| {
        if let Some(tool) = draw_tool {
            tool.finish();
        }
    };

    // 図形の作成中のキー操作(Esc=終了、Enter=確定、Backspace=1つ戻す)。
    if let Some(tool) = draw_tool {
        let keydown_handle =
            window_event_listener(leptos::ev::keydown, move |ev: web_sys::KeyboardEvent| {
                input::handle_draw_key(tool, &ev)
            });
        // このコンポーネントが破棄されたらリスナーを外す(外さないとwindowに残り続ける)。
        on_cleanup(move || keydown_handle.remove());
    }

    let pick_active = move || {
        origin_pick.is_some_and(|p| p.active.get()) || draw_tool.is_some_and(|t| t.is_active())
    };

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
                on:dblclick=on_dbl_click
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
            {move || {
                draw_tool.filter(|t| t.is_active()).map(|t| {
                    view! {
                        <div class="origin-pick-hint">
                            <span>{move || t.hint()}</span>
                            <button on:click=move |_| t.finish() disabled=move || !t.can_finish()>"確定"</button>
                            <button on:click=move |_| t.undo()>"1つ戻す"</button>
                            <button on:click=move |_| t.cancel()>"終了"</button>
                        </div>
                    }
                })
            }}
            <div class="terrain-track-labels" node_ref=labels_ref></div>
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
