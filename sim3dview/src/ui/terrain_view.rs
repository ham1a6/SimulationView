//! 地形メッシュを描画する再利用可能なcanvasコンポーネント。地形データ本体は`TerrainStore`
//! context(1回だけフェッチ)、原点は`terrain::origin::OriginState` context、レーダー観測点は
//! `terrain::markers::RadarMarkersState` contextから読む(呼び出し側が`provide_context`する。
//! `sim3dview/README.md`参照)。自由視点カメラはドラッグで回転、ホイールでズーム、
//! 3DモードではShift+ドラッグで注視点(中心点)を平行移動できる(シミュレーション原点
//! [`terrain::origin::OriginState`]は変更しない)。`terrain::recenter::RecenterRequestState`
//! contextの通知(表示メニューの「中心点を原点に戻す」ボタン)で中心点を原点へ戻す。
//! `terrain::origin_pick::OriginPickState` contextが提供されていて`active`の間は、地図の
//! 左クリック(ドラッグではない単発クリック)の地点を原点として`on_pick`へ渡す。
//! `terrain::draw_tool::DrawToolState` contextが提供されていてツールを選んでいる間は、地図の
//! 左クリックで図形の点を置く(カーソル移動で仮の図形が追従、ダブルクリック/Enterで多角形・折れ線を確定、
//! 右クリック/Backspaceで1つ戻す、Escで終了)。
//! 地図の右クリックは、`MapMenuState`(と`ui::context_menu::ContextMenuState`)が提供されていれば、
//! アプリが決めた項目の右クリックメニューを出す(提供されていなければ、その地点にレーダー観測点を追加する)。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::terrain::camera::{CameraPreset, OrbitCamera, ViewMode};
use crate::terrain::draw_tool::DrawToolState;
use crate::terrain::drawing::DrawingState;
use crate::terrain::drawing_geometry;
use crate::terrain::loader::{self, MeshKey, TerrainData, TileKey, WHOLE_TILE};
use crate::terrain::lod::{self, Resident, TilePlan};
use crate::terrain::markers::{self, RadarMarkersState};
use crate::terrain::mesh::{self, Origin};
use crate::terrain::origin::OriginState;
use crate::terrain::origin_pick::OriginPickState;
use crate::terrain::pick;
use crate::terrain::hillshade::HillshadeState;
use crate::terrain::recenter::RecenterRequestState;
use crate::terrain::renderer::TerrainRenderer;
use crate::terrain::store::TerrainStore;
use crate::terrain::tracks::{self, TrackId, TrackLabel, TrackOptions, TracksState};
use crate::ui::context_menu::{ContextMenuState, MenuItem};

/// 地図を右クリックした場所にあるもの(右クリックメニューの項目を決める材料)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapMenuTarget {
    /// 地表の(緯度, 経度)。地形データの範囲外・空ならNone。
    pub position: Option<(f64, f64)>,
    /// 右クリックした航跡のシンボル。あればTerrainViewが先にそのトラックを選択する。
    pub track: Option<TrackId>,
}

/// 地図の右クリックメニューの項目を作るコールバック(`provide_context`する。`ContextMenuState`も必要)。
/// 右クリックのたびに、その場所(`MapMenuTarget`)から項目を作って返す。空を返せばメニューは出ない。
/// レーダー観測点の追加・原点の指定・作図の開始など、何を並べるかはアプリが決める。
#[derive(Clone, Copy)]
pub struct MapMenuState(pub Callback<MapMenuTarget, Vec<MenuItem>>);

impl MapMenuState {
    pub fn new(build: impl Fn(MapMenuTarget) -> Vec<MenuItem> + Send + Sync + 'static) -> Self {
        Self(Callback::new(build))
    }
}

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
    radar_markers: RadarMarkersState,
    /// 作図(図形・線)の一覧(`terrain::drawing`)。
    drawings: DrawingState,
    /// 航跡(トラック)の一覧・表示設定(`terrain::tracks`)。
    tracks: TracksState,
    /// 航跡のラベルを置くHTML要素(canvasに重ねる層)と、いま置いているラベル。
    labels_ref: NodeRef<leptos::html::Div>,
    labels: Vec<LabelView>,
    /// クリックでの選択(当たり判定)に使う、各トラックのシンボルの位置(ENU座標)。ラベルの表示設定に関係なく持つ。
    pick_anchors: Vec<(TrackId, [f32; 3])>,
    /// 陰影(ヒルシェード)のON/OFF。レンダラー作成時の初期値に使う(以後の変更はEffect 7が反映する)。
    hillshade: HillshadeState,
    /// 各タイルの、いまGPUに載っている状態(全体1枚か、チャンクごとのレベルか。`terrain::lod`参照)。
    resident: HashMap<TileKey, Resident>,
    /// 取得中のグリッド。
    loading: HashSet<FetchKey>,
    /// 取得に失敗したグリッド。同じ取得を延々と繰り返さないよう覚えておく。
    failed: HashSet<FetchKey>,
    /// LOD更新のタイマー待ち中か(連続する操作をまとめるため)。
    lod_pending: bool,
    /// 取得の完了・メッシュ反映の続きによる、短い待ちのLOD更新の予約中か(`schedule_lod_soon`)。
    lod_soon_pending: bool,
}

/// 画面に重ねている航跡ラベル1つ分(HTML要素と、その元のデータ)。
struct LabelView {
    anchor: TrackLabel,
    root: web_sys::HtmlElement,
    name: web_sys::HtmlElement,
    detail: web_sys::HtmlElement,
}

/// 取得するグリッドの識別子: (タイル, レベル, チャンク)。チャンクがNoneなら、タイル1枚分・
/// 1レベルのファイル全体(小さいレベル)を指す(`loader::WHOLE_FILE_MAX_LEVEL`)。
type FetchKey = (TileKey, usize, Option<usize>);

/// カメラ操作が止まってからLODを更新するまでの待ち時間(ミリ秒)。操作が続く間はメッシュ生成・
/// 取得を繰り返さないためのデバウンスで、取得の完了やメッシュ反映の続きには使わない
/// (それらは`LOD_CONTINUE_MS`。ここを待つと、取得の合間が空いて全体が遅くなる)。
const LOD_DEBOUNCE_MS: u32 = 150;
/// 取得が終わった・メッシュ反映を続けたいときに、次のLOD更新まで待つ時間(ミリ秒)。同じ瞬間に
/// 終わった複数の取得を1回の更新にまとめる程度の短さで、入力イベントや描画に処理を譲る間でもある。
const LOD_CONTINUE_MS: u32 = 8;
/// 1回のLOD更新でメッシュを作ってGPUへ上げる時間の目安(ミリ秒)。これを超えたら残りは次の更新に
/// 回す(メッシュ生成は頂点数に比例して時間がかかるため、画面が固まらないようにする)。時間で区切るので、
/// 速い端末ほど1回で多く進み、遅い端末では自動的に細かく刻まれる。
const UPLOAD_TIME_BUDGET_MS: f64 = 12.0;
/// 1回のLOD更新でメッシュを作ってGPUへ上げる頂点数の安全上限(最細レベルのチャンク1個で約36万
/// 頂点)。時間の目安は次の1個の重さを予測できないので、その保険として頂点数でも区切る。
const MAX_UPLOAD_VERTICES_PER_ROUND: usize = 600_000;
/// 同時に取得するタイル数の上限。全タイルのレベル1(約69KBのファイルが390個)を起動後に
/// 取得していくので、6件だと1分ほどかかった。取得が終わるたびに`schedule_lod_soon`で補充するので、
/// ブラウザの同一ホストへの接続数(HTTP/1.1で通常6)を埋めるのに足りる数にしてある。
const MAX_CONCURRENT_TILE_FETCHES: usize = 16;
/// 取得済みタイルグリッド(細かいレベル)をメモリに残す上限。超えたら、いま使っていないものから
/// 古い順に捨てる。
const DETAIL_CACHE_LIMIT_BYTES: usize = 300 * 1024 * 1024;

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

/// canvas上の画面座標(client座標)にある航跡のシンボルのID(なければNone)。`terrain::tracks::pick_track`で、
/// シンボルの位置を画面へ射影して最も近いものを選ぶ。
fn pick_track_at_client(
    state: &Rc<RefCell<ViewState>>,
    canvas: &web_sys::HtmlCanvasElement,
    client_x: f64,
    client_y: f64,
) -> Option<TrackId> {
    let rect = canvas.get_bounding_client_rect();
    // CSSのpxからcanvasの内部解像度のpxへ(通常は同じ)。
    let x = (client_x - rect.left()) as f32 * canvas.width() as f32 / rect.width().max(1.0) as f32;
    let y = (client_y - rect.top()) as f32 * canvas.height() as f32 / rect.height().max(1.0) as f32;
    let s = state.borrow();
    let renderer = s.renderer.as_ref()?;
    let view_proj = s.camera.to_camera(renderer.aspect_ratio()).view_proj_matrix();
    let (width, height) = renderer.canvas_size_px();
    tracks::pick_track(&s.pick_anchors, &view_proj, (width as f32, height as f32), (x, y), tracks::PICK_RADIUS_PX)
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
    // 注視点は原点の実際の地表標高に置く(Vec3::ZEROのままだと、原点が高山の
    // 斜面にある場合にズームインした際カメラが地面に埋まって真っ黒になる)。
    let target_up = mesh::sample_heightmap(&data, origin.lat_deg, origin.lon_deg).unwrap_or(0.0);

    wasm_bindgen_futures::spawn_local(async move {
        match TerrainRenderer::new(canvas).await {
            Ok(mut renderer) => {
                // 全タイルを最粗のレベル0(タイル全体で1枚)で載せる(細かいレベルはカメラに近い
                // チャンクだけ、あとから`update_lod`が差し替える)。
                let transform = mesh::EnuTransform::new(&origin, &data.metadata.ellipsoid);
                let mut resident = HashMap::new();
                for tile in data.tiles() {
                    let tile_mesh = mesh::build_whole_tile_mesh(&data, tile, &transform);
                    renderer.set_mesh((tile.key.0, tile.key.1, WHOLE_TILE), &tile_mesh);
                    resident.insert(tile.key, Resident::Whole);
                }
                {
                    let mut s = state.borrow_mut();
                    s.target_up = target_up;
                    s.camera.target.z = target_up;
                    s.resident = resident;
                }
                renderer.set_hillshade(state.borrow().hillshade.enabled.get_untracked());
                renderer.set_ellipsoid_origin(&transform);
                let camera = state.borrow().camera.to_camera(renderer.aspect_ratio());
                if let Err(e) = renderer.render(&camera) {
                    log::error!("[terrain] initial render failed: {e}");
                }
                let mut s = state.borrow_mut();
                s.renderer = Some(renderer);
                s.terrain = Some(data);
                s.mesh_origin = Some(origin);
                s.initializing = false;
                // signalの更新で購読しているEffectが同期的に走っても`state`を借用し直せるよう、
                // 借用を手放してから更新する。
                drop(s);
                status.set(String::new());
                rebuild_markers(&state, radar_markers);
                rebuild_drawings(&state);
                rebuild_tracks(&state);
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

/// 3Dモードのカメラ(視点)が地面の下にもぐらないようにする(`OrbitCamera::keep_above_ground`)。
/// 視点の真下の地面の高さは、いま画面に出している地形(`mesh::ground_at_enu`)から引く。
/// カメラの操作(回転・ズーム・移動)・原点変更・LODの切り替えのあとの描画の前に必ず通る。
fn keep_camera_above_ground(state: &Rc<RefCell<ViewState>>) {
    let mut s = state.borrow_mut();
    let (Some(terrain), Some(origin)) = (s.terrain.clone(), s.mesh_origin) else {
        return;
    };
    let transform = mesh::EnuTransform::new(&origin, &terrain.metadata.ellipsoid);
    s.camera.keep_above_ground(|east, north| {
        mesh::ground_at_enu(&terrain, &transform, east as f64, north as f64).2
    });
}

/// 現在の状態で1フレーム描くだけ(LODの更新は予約しない)。
fn render_frame(state: &Rc<RefCell<ViewState>>) {
    keep_camera_above_ground(state);
    let s = state.borrow();
    if let Some(renderer) = s.renderer.as_ref() {
        let camera = s.camera.to_camera(renderer.aspect_ratio());
        if let Err(e) = renderer.render(&camera) {
            log::error!("[terrain] render failed: {e}");
        }
    }
    update_labels(&s);
}

/// 1フレーム描き、カメラなどが変わった可能性があるのでLODの更新を予約する。
fn render_now(state: &Rc<RefCell<ViewState>>) {
    render_frame(state);
    schedule_lod(state);
}

/// 少し待ってからLODを更新する。待っている間に来た予約はまとめる(操作が続く間は
/// メッシュ生成・取得を繰り返さず、操作が落ち着いた時点の最新のカメラで一度だけ計画し直す)。
fn schedule_lod(state: &Rc<RefCell<ViewState>>) {
    {
        let mut s = state.borrow_mut();
        if s.lod_pending || s.terrain.is_none() {
            return;
        }
        s.lod_pending = true;
    }
    let state = state.clone();
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(LOD_DEBOUNCE_MS).await;
        state.borrow_mut().lod_pending = false;
        update_lod(&state);
    });
}

/// 取得の完了・メッシュ反映の続きなど、カメラ操作とは関係なく「すぐ続きをやりたい」ときの予約。
/// `schedule_lod`のデバウンス(150ms)を待たず、`LOD_CONTINUE_MS`のあいだに来た予約だけをまとめる。
fn schedule_lod_soon(state: &Rc<RefCell<ViewState>>) {
    {
        let mut s = state.borrow_mut();
        if s.lod_soon_pending || s.terrain.is_none() {
            return;
        }
        s.lod_soon_pending = true;
    }
    let state = state.clone();
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(LOD_CONTINUE_MS).await;
        state.borrow_mut().lod_soon_pending = false;
        update_lod(&state);
    });
}

/// カメラに合わせて各タイルの描き方(全体1枚か、チャンクごとのレベルか)を更新する: 必要な
/// グリッドが取得済みならメッシュを作ってGPUへ差し替え、未取得なら取得を始める(届いたら再度
/// この関数が走る)。取得済みの範囲で少しずつ細かくしていく(たとえば、レベル1→2→最細)。
fn update_lod(state: &Rc<RefCell<ViewState>>) {
    let (terrain, origin, plan) = {
        let s = state.borrow();
        if s.dragging {
            drop(s);
            schedule_lod(state); // ドラッグ中は重い処理を避け、落ち着いてからやり直す。
            return;
        }
        let (Some(terrain), Some(origin), Some(renderer)) =
            (s.terrain.clone(), s.mesh_origin, s.renderer.as_ref())
        else {
            return;
        };
        let transform = mesh::EnuTransform::new(&origin, &terrain.metadata.ellipsoid);
        let camera = s.camera.to_camera(renderer.aspect_ratio());
        let plan = lod::plan_levels(
            &terrain,
            &transform,
            &camera,
            renderer.canvas_height_px() as f32,
            &s.resident,
        );
        (terrain, origin, plan)
    };
    let transform = mesh::EnuTransform::new(&origin, &terrain.metadata.ellipsoid);
    let chunk_count = terrain.chunk_count();

    let round_start = js_sys::Date::now();
    let mut uploaded_vertices = 0usize;
    let mut changed = false;
    // 取得の同時数の上限で、取得を始められなかったものがあるか(取得が終われば補充される)。
    let mut deferred = false;
    // メッシュ反映の時間・頂点数の目安を超えて、反映を次の更新に回したものがあるか。
    let mut deferred_upload = false;
    // 1個は必ず進める(0個だと永遠に終わらない)。以後は、時間の目安か頂点数の上限を超えたら回す。
    let over_budget = |uploaded: usize, cost: usize| {
        uploaded > 0
            && (js_sys::Date::now() - round_start >= UPLOAD_TIME_BUDGET_MS
                || uploaded + cost > MAX_UPLOAD_VERTICES_PER_ROUND)
    };
    let mut to_fetch: Vec<FetchKey> = Vec::new();
    // 取得が必要なグリッドを登録する(同じものは重複させず、同時取得数の上限を守る)。
    let mut request = |state: &Rc<RefCell<ViewState>>, key: TileKey, level: usize, chunk: usize| {
        let fetch_key: FetchKey = if level <= loader::WHOLE_FILE_MAX_LEVEL {
            (key, level, None)
        } else {
            (key, level, Some(chunk))
        };
        let mut s = state.borrow_mut();
        if s.failed.contains(&fetch_key) || s.loading.contains(&fetch_key) {
            return false;
        }
        if s.loading.len() >= MAX_CONCURRENT_TILE_FETCHES {
            return true; // 上限に達したので、あとの更新に回す。
        }
        s.loading.insert(fetch_key);
        to_fetch.push(fetch_key);
        false
    };

    for (key, tile_plan) in plan {
        let Some(tile) = terrain.tile(key) else { continue };
        let whole_key: MeshKey = (key.0, key.1, WHOLE_TILE);
        let current: Option<Vec<u8>> = match state.borrow().resident.get(&key) {
            Some(Resident::Chunks(v)) => Some(v.clone()),
            _ => None,
        };

        match tile_plan {
            TilePlan::Whole => {
                if current.is_some() {
                    // タイル全体の1枚のメッシュへ戻す(小さいので頂点数の上限には数えない)。
                    let whole = mesh::build_whole_tile_mesh(&terrain, tile, &transform);
                    let mut s = state.borrow_mut();
                    if let Some(renderer) = s.renderer.as_mut() {
                        renderer.set_mesh(whole_key, &whole);
                        for c in 0..chunk_count {
                            renderer.remove_mesh((key.0, key.1, c as u8));
                        }
                    }
                    s.resident.insert(key, Resident::Whole);
                    drop(s);
                    terrain.set_whole_tile(key);
                    changed = true;
                }
            }
            TilePlan::Chunks(targets) => {
                // 各チャンクの目標レベルのうち、取得済みで最も細かいレベル(無ければ0)。
                let available: Vec<usize> = (0..chunk_count)
                    .map(|c| terrain.best_cached_level(tile, c, targets[c] as usize))
                    .collect();

                let mut levels: Vec<u8> = match current {
                    Some(levels) => levels,
                    None => {
                        // 全体表示からチャンク表示へ切り替えるには、全チャンクのレベル1が要る。
                        if available.iter().any(|&l| l == 0) {
                            if request(state, key, 1, 0) {
                                deferred = true;
                            }
                            continue;
                        }
                        let cost: usize =
                            available.iter().map(|&l| lod::chunk_vertex_cost(&terrain, l)).sum();
                        if over_budget(uploaded_vertices, cost) {
                            deferred_upload = true;
                            continue;
                        }
                        let mut s = state.borrow_mut();
                        for c in 0..chunk_count {
                            if let Some(m) = mesh::build_chunk_mesh(&terrain, tile, c, available[c], &transform)
                            {
                                if let Some(renderer) = s.renderer.as_mut() {
                                    renderer.set_mesh((key.0, key.1, c as u8), &m);
                                }
                                terrain.set_chunk_level(key, c, available[c]);
                            }
                        }
                        if let Some(renderer) = s.renderer.as_mut() {
                            renderer.remove_mesh(whole_key);
                        }
                        let levels: Vec<u8> = available.iter().map(|&l| l as u8).collect();
                        s.resident.insert(key, Resident::Chunks(levels.clone()));
                        uploaded_vertices += cost;
                        changed = true;
                        levels
                    }
                };

                // 目標レベルへ近づける。目標のグリッドが取得済みならそのレベルへ(上げ下げとも)、
                // 未取得なら取得を始め、取得済みの範囲でより細かければ先にそこまで上げる。
                let mut resident_changed = false;
                for c in 0..chunk_count {
                    let want = targets[c] as usize;
                    let have = levels[c] as usize;
                    let now = available[c];
                    let new_level = if now == 0 {
                        have
                    } else if want < have {
                        now
                    } else {
                        now.max(have)
                    };
                    if !terrain.has_chunk_grid(tile, want, c) && request(state, key, want, c) {
                        deferred = true;
                    }
                    if new_level == have {
                        continue;
                    }
                    let cost = lod::chunk_vertex_cost(&terrain, new_level);
                    if over_budget(uploaded_vertices, cost) {
                        deferred_upload = true;
                        continue;
                    }
                    if let Some(m) = mesh::build_chunk_mesh(&terrain, tile, c, new_level, &transform) {
                        let mut s = state.borrow_mut();
                        if let Some(renderer) = s.renderer.as_mut() {
                            renderer.set_mesh((key.0, key.1, c as u8), &m);
                        }
                        drop(s);
                        terrain.set_chunk_level(key, c, new_level);
                        levels[c] = new_level as u8;
                        uploaded_vertices += cost;
                        resident_changed = true;
                        changed = true;
                    }
                }
                if resident_changed {
                    state.borrow_mut().resident.insert(key, Resident::Chunks(levels));
                }
            }
        }
    }

    for fetch_key in to_fetch {
        let state = state.clone();
        let terrain = terrain.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let (key, level, chunk) = fetch_key;
            let result = match chunk {
                None => loader::fetch_tile_level(
                    &terrain.base_url,
                    key,
                    level,
                    terrain.chunk_cells(level),
                    terrain.chunk_count(),
                )
                .await
                .map(|all| terrain.insert_tile_level(key, level, &all)),
                Some(c) => loader::fetch_chunk_grid(
                    &terrain.base_url,
                    key,
                    level,
                    c,
                    terrain.chunk_cells(level),
                )
                .await
                .map(|grid| terrain.insert_chunk_grid(key, level, c, grid)),
            };
            {
                let mut s = state.borrow_mut();
                s.loading.remove(&fetch_key);
                if let Err(e) = result {
                    log::warn!("[terrain] tile fetch failed: {e}");
                    s.failed.insert(fetch_key);
                }
            }
            // 取得が終わったら、デバウンスを待たずに続き(反映と、次の取得の補充)へ進む。
            schedule_lod_soon(&state);
        });
    }

    if changed {
        // 地形の高さが変わったので、注視点の高さ(ズームインしても地面に埋まらないように)と、
        // 地表に貼り付いている観測点・覆域を合わせ直す。
        {
            let mut s = state.borrow_mut();
            s.target_up = mesh::sample_heightmap(&terrain, origin.lat_deg, origin.lon_deg)
                .unwrap_or(0.0);
            let (tx, ty) = (s.camera.target.x as f64, s.camera.target.y as f64);
            s.camera.target.z = mesh::ground_at_enu(&terrain, &transform, tx, ty).2;
            let chunks = terrain.chunks_per_tile() * terrain.chunks_per_tile();
            let resident = &s.resident;
            terrain.evict_unused(
                |key, chunk, level| match resident.get(&key) {
                    Some(Resident::Chunks(levels)) => {
                        chunk < chunks && levels[chunk] as usize == level
                    }
                    _ => false,
                },
                DETAIL_CACHE_LIMIT_BYTES,
            );
        }
        let has_markers = !state.borrow().radar_markers.markers.get_untracked().is_empty();
        if has_markers {
            let radar_markers = state.borrow().radar_markers;
            rebuild_markers(state, radar_markers);
        }
        // 地表に貼り付けた作図も、地形の高さが変わったので作り直す。
        let follows_terrain = state
            .borrow()
            .drawings
            .items
            .with_untracked(|list| list.iter().any(|d| d.visible && d.shape.depends_on_terrain()));
        if follows_terrain {
            rebuild_drawings(state);
        }
        // 航跡(地表基準のトラック・高度線)も地形の高さが変わったので作り直す。
        if !state.borrow().tracks.entries.with_untracked(|list| list.is_empty()) {
            rebuild_tracks(state);
        }
        render_frame(state);
    }
    if deferred_upload {
        // メッシュ反映の続き。入力や描画に処理を譲ってから、すぐ次の分に進む。
        schedule_lod_soon(state);
    } else if deferred {
        // 取得の同時数が上限だった分。取得が終わるたびに`schedule_lod_soon`されるので、これは念のため。
        schedule_lod(state);
    }
}

/// レーダー観測点マーカー(ピン)・見通し範囲の覆域(3Dはドーム、2Dは塗り+輪郭線)のジオメトリを、現在の地形・原点・
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
    let marker_vertices = markers::build_marker_geometry(&terrain, &mesh_origin, &marker_list, selected);
    // 覆域表示は3D(半球ドーム)と2D(指定高度での探知可能領域)で見せ方自体が別物なので、
    // モードに応じて別のジオメトリ・別のバッファ(パイプライン)に渡す(使わない方は空にする)。
    let (dome_vertices, coverage_2d_vertices) = match mode {
        ViewMode::ThreeD => (
            markers::build_dome_surface_geometry(&terrain, &mesh_origin, &marker_list, selected),
            Vec::new(),
        ),
        ViewMode::TwoD => {
            let altitude_m = radar_markers.coverage_altitude_m.get_untracked();
            (
                Vec::new(),
                markers::build_coverage_2d_geometry(&terrain, &mesh_origin, &marker_list, selected, altitude_m),
            )
        }
    };
    renderer.update_markers(&marker_vertices);
    renderer.update_dome(&dome_vertices);
    renderer.update_coverage_2d(&coverage_2d_vertices);
}

/// 作図(`terrain::drawing`)の一覧から頂点列を作り直してGPUバッファへ反映する。一覧の変更・原点変更
/// (メッシュ再構築後)・地表に貼り付けた図形があるときの地形LOD切り替え・canvasのリサイズ
/// (画面座標の角の位置が変わる)のときに呼ぶ。描画自体は呼び出し側で`render_now`すること。
fn rebuild_drawings(state: &Rc<RefCell<ViewState>>) {
    let mut s = state.borrow_mut();
    let (Some(terrain), Some(mesh_origin)) = (s.terrain.clone(), s.mesh_origin) else {
        return;
    };
    let drawings = s.drawings;
    let Some(renderer) = s.renderer.as_mut() else {
        return;
    };
    let transform = mesh::EnuTransform::new(&mesh_origin, &terrain.metadata.ellipsoid);
    let (width, height) = renderer.canvas_size_px();
    // 地形データの範囲外・海は標高0mとして扱う(`mesh::sample_heightmap`)。
    let ground = |lat: f64, lon: f64| mesh::sample_heightmap(&terrain, lat, lon).unwrap_or(0.0) as f64;
    let ctx = drawing_geometry::BuildContext {
        mesh_transform: &transform,
        ellipsoid: &terrain.metadata.ellipsoid,
        ground: &ground,
        viewport_px: (width as f32, height as f32),
    };
    let batches = drawings.items.with_untracked(|list| drawing_geometry::build(&ctx, list));
    renderer.update_drawings(&batches);
}

/// 航跡(`terrain::tracks`)の一覧・表示設定から、シンボル・航跡・高度線の頂点列とラベルを作り直して反映する。
/// トラックの受信・表示設定の変更・原点変更・地形のLOD切り替え・2D/3D切り替えのときに呼ぶ。
/// 描画自体は呼び出し側で`render_frame`(または`render_now`)すること。
fn rebuild_tracks(state: &Rc<RefCell<ViewState>>) {
    let mut s = state.borrow_mut();
    let (Some(terrain), Some(mesh_origin), mode) = (s.terrain.clone(), s.mesh_origin, s.camera.mode) else {
        return;
    };
    let tracks_state = s.tracks;
    let layer = label_layer(&s);
    let geometry = {
        let Some(renderer) = s.renderer.as_mut() else {
            return;
        };
        let transform = mesh::EnuTransform::new(&mesh_origin, &terrain.metadata.ellipsoid);
        let (width, height) = renderer.canvas_size_px();
        let ground = |lat: f64, lon: f64| mesh::sample_heightmap(&terrain, lat, lon).unwrap_or(0.0) as f64;
        let ctx = drawing_geometry::BuildContext {
            mesh_transform: &transform,
            ellipsoid: &terrain.metadata.ellipsoid,
            ground: &ground,
            viewport_px: (width as f32, height as f32),
        };
        let options = TrackOptions {
            selected: tracks_state.selected.get_untracked(),
            trails: tracks_state.show_trails.get_untracked(),
            // 真上から見る2D地図では、縦の線は点になるので出さない。
            altitude_lines: tracks_state.show_altitude_lines.get_untracked() && mode == ViewMode::ThreeD,
        };
        let geometry = tracks_state
            .entries
            .with_untracked(|entries| tracks::build_track_geometry(&ctx, entries, options));
        renderer.update_tracks(&geometry.vertices);
        geometry
    };
    s.pick_anchors = geometry.labels.iter().map(|l| (l.id, l.position)).collect();
    let labels = if tracks_state.show_labels.get_untracked() { geometry.labels } else { Vec::new() };
    set_labels(&mut s, layer.as_ref(), labels);
}

/// ラベルを覆う層(`TerrainView`が`canvas`の上に置くHTML要素)。
fn label_layer(s: &ViewState) -> Option<web_sys::HtmlElement> {
    let element = s.labels_ref.get_untracked()?;
    (*element).clone().dyn_into::<web_sys::HtmlElement>().ok()
}

/// 画面に重ねるラベルを`labels`にそろえる。数が同じなら要素を使い回して文字だけ更新し、変わったら作り直す。
fn set_labels(s: &mut ViewState, layer: Option<&web_sys::HtmlElement>, labels: Vec<TrackLabel>) {
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
fn update_labels(s: &ViewState) {
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
const LABEL_OFFSET_X_PX: f32 = 18.0;
const LABEL_OFFSET_Y_PX: f32 = -16.0;

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
    // 未提供なら既定(陰影ON)のまま切り替えなしで動作する(上と同じく後付けのオプション機能)。
    let hillshade = use_context::<HillshadeState>().unwrap_or_default();
    // 未提供なら「クリックで原点指定」機能なしで動作する(上と同じく後付けのオプション機能)。
    let origin_pick = use_context::<OriginPickState>();
    // 未提供なら作図なしで動作する(上と同じく後付けのオプション機能)。
    let drawings = use_context::<DrawingState>().unwrap_or_default();
    // 未提供なら図形の対話作成なしで動作する(同上)。
    let draw_tool = use_context::<DrawToolState>();
    // 未提供なら航跡表示なしで動作する(同上)。
    let tracks = use_context::<TracksState>().unwrap_or_default();
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
        dragging: false,
        last_x: 0.0,
        last_y: 0.0,
        down_x: 0.0,
        down_y: 0.0,
        radar_markers,
        drawings,
        tracks,
        labels_ref,
        labels: Vec::new(),
        pick_anchors: Vec::new(),
        hillshade,
        resident: HashMap::new(),
        loading: HashSet::new(),
        failed: HashSet::new(),
        lod_pending: false,
        lod_soon_pending: false,
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

            // クロージャ・observerはこのパネルの生存期間ずっと必要。パネルが破棄されるとき(このEffectの
            // オーナーの後始末)に、observerとdocumentのリスナーを外してからクロージャごと解放する
            // (`forget`すると、クロージャが握る`state`=GPUデバイスまでずっと解放されない)。
            // `on_cleanup`は`Send`を要求するので、JSオブジェクトはローカル専用の`StoredValue`に入れて渡す。
            let resources = StoredValue::new_local((closure, visibility_closure, observer));
            on_cleanup(move || {
                resources.with_value(|(_, visibility_closure, observer)| {
                    observer.disconnect();
                    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
                        let _ = document.remove_event_listener_with_callback(
                            "visibilitychange",
                            visibility_closure.as_ref().unchecked_ref(),
                        );
                    }
                });
            });
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
            let new_transform = mesh::EnuTransform::new(&new_origin, &terrain.metadata.ellipsoid);
            // 注視点(中心点)の扱い: 原点の真上を見ていた(x=y=0)なら新しい原点に追従する。
            // パンして別の場所を見ていたなら、ENU座標のオフセットが新しい原点基準のまま残って
            // 表示が飛んでしまわないよう、同じ緯度経度を見続けるよう新しいENU座標へ変換し直す。
            let follows_origin = s.camera.target.x == 0.0 && s.camera.target.y == 0.0;
            s.target_up =
                mesh::sample_heightmap(&terrain, new_origin.lat_deg, new_origin.lon_deg).unwrap_or(0.0);
            if follows_origin {
                // 高さも新しい原点の地表標高へ更新する(古い標高のままだと、原点移動後に
                // ズームインした際カメラが地面に埋まって真っ黒になりうる)。
                s.camera.target.z = s.target_up;
            } else if let Some(old_origin) = s.mesh_origin {
                let old_transform =
                    mesh::EnuTransform::new(&old_origin, &terrain.metadata.ellipsoid);
                let (lat, lon, _) = mesh::ground_at_enu(
                    &terrain,
                    &old_transform,
                    s.camera.target.x as f64,
                    s.camera.target.y as f64,
                );
                let (x, y, up) = mesh::ground_at_geodetic(&terrain, &new_transform, lat, lon);
                s.camera.target.x = x;
                s.camera.target.y = y;
                s.camera.target.z = up;
            }
            // 常駐している全メッシュ(タイル全体・各チャンク、各自の解像度レベル)の頂点位置を、新しい原点のENU座標で
            // 作り直してアップロードする(頂点数・並びは原点に依存しない)。
            let Some(renderer) = s.renderer.as_ref() else {
                return;
            };
            // 水域レイヤー(楕円体の海抜0mの面)も新しい原点基準にする。
            renderer.set_ellipsoid_origin(&new_transform);
            for (&key, resident) in s.resident.iter() {
                let Some(tile) = terrain.tile(key) else { continue };
                match resident {
                    Resident::Whole => {
                        let vertices = mesh::build_whole_tile_vertices(&terrain, tile, &new_transform);
                        renderer.update_mesh_vertices((key.0, key.1, WHOLE_TILE), &vertices);
                    }
                    Resident::Chunks(levels) => {
                        for (c, &level) in levels.iter().enumerate() {
                            if let Some(vertices) = mesh::build_chunk_vertices(
                                &terrain,
                                tile,
                                c,
                                level as usize,
                                &new_transform,
                            ) {
                                renderer.update_mesh_vertices((key.0, key.1, c as u8), &vertices);
                            }
                        }
                    }
                }
            }
            let camera = s.camera.to_camera(renderer.aspect_ratio());
            if let Err(e) = renderer.render(&camera) {
                log::error!("[terrain] re-render after origin change failed: {e}");
            }
            s.mesh_origin = Some(new_origin);
            drop(s);
            // マーカー・覆域リング・作図も新しい原点基準のENU座標へ再変換する。
            rebuild_markers(&state, radar_markers);
            rebuild_drawings(&state);
            rebuild_tracks(&state);
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
            let target = recenter_request.target();
            let mut s = state.borrow_mut();
            if s.renderer.is_none() {
                return;
            }
            match (target, s.terrain.clone(), s.mesh_origin) {
                // 右クリックメニュー等で指定した地点へ。高さはその地点の実際の地表(ENU上座標)にする
                // (Shift+ドラッグでの移動と同じ。古い高さのままだとズームインしたときカメラが地面に埋まる)。
                (Some((lat, lon)), Some(terrain), Some(mesh_origin)) => {
                    let transform = mesh::EnuTransform::new(&mesh_origin, &terrain.metadata.ellipsoid);
                    let (east, north, up) = mesh::ground_at_geodetic(&terrain, &transform, lat, lon);
                    s.camera.target.x = east;
                    s.camera.target.y = north;
                    s.camera.target.z = up;
                }
                // 地形が未取得の間は動かさない。
                (Some(_), _, _) => return,
                // 原点へ戻す。
                (None, _, _) => {
                    s.camera.target.x = 0.0;
                    s.camera.target.y = 0.0;
                    s.camera.target.z = s.target_up;
                }
            }
            drop(s);
            render_now(&state);
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
    // 次のフレームで反映する予定のカーソル位置(canvasとclient座標)。
    let hover_pending: Rc<RefCell<Option<(web_sys::HtmlCanvasElement, (f64, f64))>>> = Rc::new(RefCell::new(None));
    let on_pointer_move = move |ev: leptos::ev::PointerEvent| {
        // 図形の作成中は、カーソルの指す地点へ仮の図形の先端を追従させる(ドラッグ中は動かさない)。
        // 仮の図形を更新するたびに作図全体の再構築+描画が走るので、マウス移動は1フレームに1回へまとめる。
        if let Some(tool) = draw_tool.filter(|t| t.wants_hover()) {
            if !state_pm.borrow().dragging {
                if let Some(canvas) = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlCanvasElement>().ok()) {
                    let pos = (ev.client_x() as f64, ev.client_y() as f64);
                    let already_scheduled = hover_pending.borrow_mut().replace((canvas, pos)).is_some();
                    if !already_scheduled {
                        let hover_pending = hover_pending.clone();
                        let state = state_pm.clone();
                        request_animation_frame(move || {
                            let pending = hover_pending.borrow_mut().take();
                            if let Some((canvas, (x, y))) = pending {
                                if let Some((lat, lon)) = pick_at_client(&state, &canvas, x, y) {
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
                            // 移動先の実際の地表(ENU上座標)へtarget.zを更新する(古い高さの
                            // ままだと、原点変更時と同様にズームインした際カメラが地面に
                            // 埋まって真っ黒になりうる)。原点から遠いほど地球の丸みで地表が
                            // 下がるため、標高ではなく丸みを含む上座標を使う。
                            if let (Some(terrain), Some(mesh_origin)) =
                                (s.terrain.clone(), s.mesh_origin)
                            {
                                let transform =
                                    mesh::EnuTransform::new(&mesh_origin, &terrain.metadata.ellipsoid);
                                let (_, _, up) = mesh::ground_at_enu(
                                    &terrain,
                                    &transform,
                                    s.camera.target.x as f64,
                                    s.camera.target.y as f64,
                                );
                                s.camera.target.z = up;
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

    // ドラッグではない左クリックが離されたとき:
    // - 「クリックで原点指定」モード中なら、その地点を原点として`on_pick`へ渡してモードを解除する
    // - 図形の作成ツールを選んでいれば、その地点を図形の点として置く(`terrain::draw_tool`)
    // - そうでなければ、クリックしたシンボルの航跡(`terrain::tracks`)を選択する(何もない所なら選択解除)
    // (通常のドラッグ=回転・パンは従来通り動く)
    let state_pu = state.clone();
    let on_pointer_up = move |ev: leptos::ev::PointerEvent| {
        let (down_x, down_y) = {
            let mut s = state_pu.borrow_mut();
            s.dragging = false;
            (s.down_x, s.down_y)
        };
        if ev.button() != 0 {
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
        if let Some(pick_state) = origin_pick.filter(|p| p.active.get_untracked()) {
            // 地形データ範囲外(海の外側など)をクリックした場合は、モードを維持して指定し直せるようにする。
            if let Some((lat, lon)) =
                pick_at_client(&state_pu, &canvas, ev.client_x() as f64, ev.client_y() as f64)
            {
                pick_state.active.set(false);
                pick_state.on_pick.run((lat, lon));
            }
            return;
        }
        if let Some(tool) = draw_tool.filter(|t| t.tool.get_untracked().is_some()) {
            // 図形の作成中。地形データ範囲外のクリックは無視する(点を置き直せる)。
            if let Some((lat, lon)) =
                pick_at_client(&state_pu, &canvas, ev.client_x() as f64, ev.client_y() as f64)
            {
                tool.click(lat, lon);
            }
            return;
        }
        let picked = pick_track_at_client(&state_pu, &canvas, ev.client_x() as f64, ev.client_y() as f64);
        tracks.select(picked);
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
        if let (Some(menu), Some(map_menu)) = (context_menu, map_menu) {
            let (x, y) = (ev.client_x() as f64, ev.client_y() as f64);
            let position = pick_at_client(&state_ctx, &canvas, x, y);
            let track = pick_track_at_client(&state_ctx, &canvas, x, y);
            if track.is_some() {
                tracks.select(track);
            }
            if position.is_some() || track.is_some() {
                menu.show(x, y, map_menu.0.run(MapMenuTarget { position, track }));
            }
            return;
        }
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

    // 図形の作成中のキー操作(Esc=終了、Enter=確定、Backspace=1つ戻す)。入力欄への入力は邪魔しない。
    if let Some(tool) = draw_tool {
        let keydown_handle = window_event_listener(leptos::ev::keydown, move |ev: web_sys::KeyboardEvent| {
            if tool.tool.get_untracked().is_none() {
                return;
            }
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
                    ev.prevent_default();
                    tool.undo();
                }
                _ => {}
            }
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
