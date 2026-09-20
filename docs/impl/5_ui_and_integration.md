# 実装仕様 第5部: UI層(Leptosコンポーネント)・context・LOD適用ループ・CSS契約・crate構成

[IMPLEMENTATION_GUIDE.md](../../IMPLEMENTATION_GUIDE.md)の一部。`sim3dview::ui::*`、`terrain::{origin, origin_pick, recenter, hillshade, store}`。
前提: 第1〜4部。**Leptos 0.8(`csr`)** 前提。

---

## 1. crate構成・依存・公開範囲

### 1.1 `sim3dview/Cargo.toml`

```toml
[package] name = "sim3dview", version = "0.1.0", edition = "2021"
[dependencies]
leptos = { version = "0.8", features = ["csr"] }
wasm-bindgen = "0.2"      wasm-bindgen-futures = "0.4"      js-sys = "0.3"
serde = { version = "1", features = ["derive"] }            serde_json = "1"
gloo-net = "0.7"          gloo-timers = { version = "0.4", features = ["futures"] }
log = "0.4"               wgpu = "30"                        bytemuck = { version = "1", features = ["derive"] }
glam = "0.33"             earcutr = "0.5.0"
[dependencies.web-sys] version = "0.3"
features = ["Event","EventTarget","PointerEvent","WheelEvent","MouseEvent","HtmlCanvasElement","ResizeObserver","ResizeObserverEntry",
            "DomRectReadOnly","DomRect","Element","Window","Document","Node","HtmlElement","CssStyleDeclaration","Storage","Navigator","KeyboardEvent"]
[dev-dependencies]
naga = { version = "30", features = ["wgsl-in"] }
```
ワークスペースルートの`Cargo.toml`: `members = ["sim3dview", "sample/sim_frontend"]`、`[profile.release] opt-level = "s"`。ターゲットは`wasm32-unknown-unknown`(`rustup target add`)。
確認コマンド: `cargo check -p sim3dview --target wasm32-unknown-unknown`、`cargo test -p sim3dview`(ネイティブ)。

### 1.2 モジュールと公開範囲

```
src/lib.rs:  pub mod terrain;  pub mod ui;
src/terrain/mod.rs:  pub mod camera, draw_tool, drawing, hillshade, markers, origin, origin_pick, recenter, store, tracks;
                     pub(crate) mod drawing_geometry, fetch, geodesy, heightmap, loader, lod, los, mesh, pick, profile, render_bias, renderer, vertex;
src/ui/mod.rs:      pub mod context_menu, coverage_altitude_dialog, cross_section_view, drawing_editor, floating_panel, los_view, origin_dialog, tabbed_panel, terrain_view, util;
```
アプリが使うものだけを`pub`にする(座標変換・LOD・描画・見通し計算は内部)。`TerrainStore`が公開するのに`TerrainData`(`loader`)は`pub(crate)`だが、`TerrainStore::get()`の戻り値型として型は見える(実装上`pub struct`)。
ライブラリは**通信プロトコルもサーバーのURLも一切知らない**(`base_url`・原点のミラーはアプリの責務)。`style/sim3dview.css`を同梱。

### 1.3 Leptos 0.8の落とし穴(過去に踏んだもの。必ず守る)

- `view!`の属性値に演算子を含む式を直接書かない(`let`で受けてから渡す。隣の属性が誤認識される)。
- `{move || ...}`等のchildren位置のクロージャは`Send`境界を要求する。`Rc<RefCell<..>>`を直接捕捉できない。
  `StoredValue::new_local`で包んだ`Copy`のハンドルを捕捉する。**`unsafe impl Send`は使わない**。公開コールバックは`Send`不要の`UnsyncCallback`(`Callback`ではない)。
- `Rc<TerrainData>`は`Send/Sync`でないので、`RwSignal<_, LocalStorage>`(`RwSignal::new_local`)に入れる(wasm32は単一スレッドなので安全)。
- 同じ詳細度のCSSクラスは宣言順が後ろの方が勝つ。
- `Effect`内で`state: Rc<RefCell<ViewState>>`を`borrow_mut`したままシグナルを更新すると、同期的に走る別Effectが再借用して`BorrowMutError`になる。**借用を`drop`してからシグナルを更新する**。
- `on_cleanup`は`Send`を要求するので、JSオブジェクト(`Closure`/`ResizeObserver`)は`StoredValue::new_local`に入れて渡す。`Closure::forget`はクロージャが握る`state`(GPUデバイス)が解放されなくなるので使わない。
- `trunk serve`はpath依存先(`sim3dview`)の変更を自動検知しない(ライブラリだけ編集したら`trunk`を再起動)。

---

## 2. アプリが`provide_context`するもの(context一覧)

| context型 | 必須? | 定義 | 役割 |
|---|---|---|---|
| `TerrainStore` | 必須 | `terrain::store` | 地形データの共有キャッシュ。`TerrainStore::new(base_url)`。全パネルで1回だけ取得 |
| `OriginState(RwSignal<Option<Origin>>)` | 必須 | `terrain::origin` | 現在の原点。`None`ならmetadataの`default_origin`にフォールバック。アプリが自分のプロトコルの値を`Effect`でミラー |
| `RadarMarkersState` | 必須 | `terrain::markers` | 観測点一覧・選択・覆域高度 |
| `RecenterRequestState` | 任意(`unwrap_or_default`) | `terrain::recenter` | 注視点の移動要求 |
| `HillshadeState` | 任意(`unwrap_or_default`、既定ON) | `terrain::hillshade` | 陰影ON/OFF |
| `OriginPickState` | 任意(`use_context`のOption) | `terrain::origin_pick` | 地図クリックで原点指定 |
| `DrawingState` | 任意(`unwrap_or_default`) | `terrain::drawing` | 作図の一覧 |
| `DrawToolState` | 任意 | `terrain::draw_tool` | 図形の対話作成 |
| `TracksState` | 任意(`unwrap_or_default`) | `terrain::tracks` | 航跡 |
| `ContextMenuState` + `MapMenuState` | 任意(**両方**あれば地図の右クリックがメニューになる) | `ui::context_menu` | 右クリックメニュー |
| `OriginDialogState(RwSignal<bool>)` | `OriginDialog`使用時 | `ui::origin_dialog` | 開閉 |
| `CoverageAltitudeDialogState(RwSignal<bool>)` | `CoverageAltitudeDialog`使用時 | `ui::coverage_altitude_dialog` | 開閉 |

必須contextが無いと`expect("… context not found")`でpanicする(`TerrainView`/`LosView`/`CrossSectionView`/`OriginDialog`)。任意のものは「後付けのオプション機能」で、未提供でも従来どおり動く。

### 2.1 小さな状態型(そのまま実装する)

```rust
// terrain::origin
#[derive(Debug, Clone, Copy, PartialEq)] pub struct Origin { pub lat_deg: f64, pub lon_deg: f64 }
#[derive(Clone, Copy)] pub struct OriginState(pub RwSignal<Option<Origin>>);

// terrain::origin_pick
#[derive(Clone, Copy)] pub struct OriginPickState { pub active: RwSignal<bool>, pub on_pick: UnsyncCallback<(f64,f64)> }
OriginPickState::new(on_pick) → active=false

// terrain::hillshade
#[derive(Clone, Copy)] pub struct HillshadeState { pub enabled: RwSignal<bool> }     // new(enabled), Default=new(true)

// terrain::recenter — 「押された」という単発イベントを運ぶので、状態ではなく単調増加カウンタ
#[derive(Clone, Copy)] pub struct RecenterRequestState { pub count: RwSignal<u32>, target: RwSignal<Option<(f64,f64)>> }
new(): count=0,target=None
request():                 target=None(=原点へ戻す); count = count.wrapping_add(1)
request_at(lat, lon):      target=Some((lat,lon));    count += 1
target() -> Option<(f64,f64)>:  target.get_untracked()
Default = new()

// terrain::store
#[derive(Clone, Copy)] pub struct TerrainStore { base_url: RwSignal<String>, data: RwSignal<Option<Rc<TerrainData>>, LocalStorage>, loading: RwSignal<bool>, pub error: RwSignal<Option<String>> }
new(base_url)
get() -> Option<Rc<TerrainData>>            // リアクティブ(届いたら依存Effectが再実行)
get_untracked()
ensure_loaded():   data or loading があれば何もしない。loading=true→spawn_local{ fetch::load_terrain(base_url) → Ok:data.set(Some(Rc)), Err:log::error+error.set(Some(e)) ; loading=false }
```

---

## 3. 地図コンポーネント `ui::terrain_view::TerrainView(preset: CameraPreset)`

ファイル分割: `mod.rs`(コンポーネント・イベント・Effect)、`state.rs`、`frame.rs`、`lod_driver.rs`、`overlay.rs`、`labels.rs`、`picking.rs`。

### 3.1 `ViewState`(`Rc<RefCell<..>>`。GPUを含むので`Send`でない)

```
renderer: Option<TerrainRenderer>       terrain: Option<Rc<TerrainData>>      mesh_origin: Option<Origin>   // 現在GPUにあるメッシュの原点
camera: OrbitCamera                     target_up: f32                         // 注視点(原点付近)の地表標高
initializing, dragging: bool            last_x,last_y, down_x,down_y: f64      // ドラッグ管理/押下位置(クリックとドラッグの判別)
radar_markers, drawings, tracks         labels_ref: NodeRef<Div>, labels: Vec<LabelView>, pick_anchors: Vec<(TrackId,[f32;3])>
hillshade: HillshadeState               resident: HashMap<TileKey, TileLayout>  // いまGPUにある状態
loading: HashSet<FetchKey>              failed: HashSet<FetchKey>              // 取得中/失敗(再試行しない)
lod_pending, lod_soon_pending: bool     // 予約中フラグ
type FetchKey = (TileKey, usize /*level*/, Option<usize> /*chunk。None=タイル1ファイル(level<=2)*/)
LabelView { anchor: TrackLabel, root/name/detail: HtmlElement }
```
初期: `camera = OrbitCamera::preset(preset, 0.0)`、他は空/false。`status: RwSignal<String>`は初期`"地形データを読み込み中..."`。

### 3.2 DOM構造

```html
<div class="terrain-view">
  <canvas class="terrain-canvas" [class:origin-pick-active] pointerdown/move/up/cancel, wheel, contextmenu, dblclick/>
  <div class="origin-pick-hint">…</div>                 <!-- OriginPickState.activeのとき: 「原点にする地点をクリックしてください」+[キャンセル] -->
  <div class="origin-pick-hint">…</div>                 <!-- DrawToolState.is_active()のとき: hint() + [確定(can_finishで有効)][1つ戻す][終了] -->
  <div class="terrain-track-labels" ref/>               <!-- 航跡ラベルを置く層 -->
  <div class="terrain-view-controls"><button title="2D/3D表示切り替え">2D表示に切替|3D表示に切替</button></div>
  <p class="placeholder map-status">{status}</p>        <!-- statusが空でなければ -->
</div>
```
`origin-pick-active`クラス: 原点指定または図形作成ツール選択中(`pick_active`)。カーソルをcrosshairにする。

### 3.3 初期化 `try_init`

canvasのサイズ確定(ResizeObserver)と地形データ取得(`TerrainStore`)は**非同期かつ独立**に完了するので、両方から`try_init`を呼び、揃った時点で実際に初期化する。
`canvas.width()==0 || height()==0`・`data`がNone・`renderer.is_some() || initializing` のいずれかなら何もしない。`initializing=true`にして続行:
1. `origin = origin_state.get_untracked() or Origin(default_origin)`。`target_up = sample_heightmap(origin) or 0`。
2. `spawn_local`: `TerrainRenderer::new(canvas).await`。失敗は`log::error`+`status="地形描画エラー: {e}"`+`initializing=false`。
3. 成功: `transform=EnuTransform::new(origin)`。**全タイルを`build_whole_tile_mesh`でレベル0のメッシュとして`set_mesh((lat,lon,WHOLE_TILE))`**、`resident[tile]=Whole`。
   `state.target_up = target_up; camera.target.z = target_up`、`renderer.set_hillshade(hillshade.enabled)`、`renderer.set_ellipsoid_origin(&transform)`、初回`render`。
   `state.renderer/terrain/mesh_origin`を保存、`initializing=false`。**借用を`drop`してから** `status.set("")`、`rebuild_markers`・`rebuild_drawings`・`rebuild_tracks`・`render_now`。

### 3.4 描画・LODの予約

- `keep_camera_above_ground(state)`: `camera.keep_above_ground(|e,n| ground_at_enu(terrain, transform(mesh_origin), e, n).up)`。
- `render_frame(state)`: `keep_camera_above_ground`→`renderer.render(camera.to_camera(aspect))`(失敗は`log::error`)→`update_labels`。**LODは予約しない**(航跡の高頻度更新用)。
- `render_now(state)`: `render_frame`→`schedule_lod`。カメラ操作・原点変更・リサイズ・購読の変化で呼ぶ。

### 3.5 Effect(この番号で実装する。すべて`state`のクローンを捕捉)

| # | 購読 | 処理 |
|---|---|---|
| 1 | `canvas_ref` | canvasマウント時に`ResizeObserver`を作り、`apply_size(w,h)`(0なら無視)。`canvas.set_width/height(w,h)`。レンダラーがあれば`resize`→`rebuild_drawings`(`Screen`の角が動く)→`render_now`、無ければ`try_init`。`visibilitychange`(`document.hidden`でなくなったとき)で`getBoundingClientRect`を取り直し、canvasのwidth/heightとずれていれば`apply_size`(**非表示タブではResizeObserverがスロットリングされ、初期解像度300×150のまま引き伸ばされるため**)。`on_cleanup`で`observer.disconnect`と`remove_event_listener` |
| 2 | `terrain_store.get()` | データが届いたら`try_init` |
| 3 | `origin_state.0.get()` | 原点変更(下記3.6) |
| 4 | `radar_markers.{markers, selected, coverage_altitude_m}` | `rebuild_markers`→`render_now` |
| 5 | `drawings.items.get()` | `rebuild_drawings`→`render_now` |
| 5b | `tracks.{entries, show_labels, show_trails, show_altitude_lines, selected}` | `rebuild_tracks`→`render_frame`(LODは予約しない) |
| 6 | `recenter_request.count.get()` | `count==0`は無視。`renderer`無しは無視。`target()`が`Some((lat,lon))`: 地形と`mesh_origin`があれば`ground_at_geodetic`の(東,北,上)を`camera.target`に(無ければ何もせず`return`)。`None`: `target.x=y=0`, `target.z=target_up`。→`render_now`。**`OriginState`には触れない** |
| 7 | `hillshade.enabled.get()` | `renderer.set_hillshade`→`render_now`(メッシュ再作成不要) |

### 3.6 原点変更(Effect 3)

`new_origin`が`None`・地形/レンダラー未初期化・`mesh_origin == Some(new_origin)`なら何もしない。
1. `new_transform`。`target_up = sample_heightmap(new_origin) or 0`。
2. **注視点の扱い**: `follows_origin = camera.target.x==0 && camera.target.y==0`。真なら`target.z = target_up`(古い標高のままだとズームインで地面に埋まる)。
   偽(パンして別の場所を見ていた)なら、旧原点で`ground_at_enu(target.xy)`の(lat,lon)を求め、新原点の`ground_at_geodetic`のENUを`target.xyz`にする(同じ緯度経度を見続ける)。
3. `renderer.set_ellipsoid_origin(&new_transform)`。**常駐する全メッシュ**(`resident`: Wholeなら`build_whole_tile_vertices`→`update_mesh_vertices((lat,lon,WHOLE_TILE))`、Chunks(levels)なら各チャンクを`build_chunk_vertices(tile,c,level)`→`update_mesh_vertices((lat,lon,c))`)の頂点位置を新原点で作り直して書き換える(頂点数・並び・インデックスは原点非依存で不変。地形データの再取得は不要)。
4. `render`(即時)、`mesh_origin=new_origin`、借用を`drop`してから`rebuild_markers`・`rebuild_drawings`・`rebuild_tracks`・`render_now`。

### 3.7 入力イベント

定数: `ORBIT_SENSITIVITY = 0.0075`、`CLICK_MAX_MOVE_PX = 5.0`(押下位置からこの距離未満の移動はドラッグでなく単発クリック)、ホイールのズーム係数 `1.12`(`delta_y>0`で`×1.12`、それ以外`÷1.12`)。

- **`pointerdown`**: `dragging=true`、`last`/`down`を`client_x/y`に。`set_pointer_capture(pointer_id)`。
- **`pointermove`**:
  - 図形作成中(`draw_tool.wants_hover()`)かつ非ドラッグなら、カーソル位置(canvas,client座標)を保持し**`request_animation_frame`で1フレームに1回へまとめて**`pick_at_client`→`tool.set_hover`(仮の図形更新のたびに作図全体の再構築+描画が走るため)。
  - `dragging`中: `dx,dy = client差分`、`last`更新。3D: `shift`なら`pan_orbit_target(dx,dy,canvas_h)`後に`target.z = ground_at_enu(target.xy).up`、それ以外は`orbit(dx*0.0075, dy*0.0075)`。
    2D: `world_per_px = distance/canvas_h`、`pan(dx*wpp, -dy*wpp)`(上へのドラッグ=北へ)。→`render_now`。
- **`pointercancel`**: `dragging=false`。
- **`pointerup`**: `dragging=false`。`button != 0`は終了。移動が5px以上は終了。ターゲットがcanvasでなければ終了。優先順:
  1. `origin_pick.active`: `pick_at_client`が`Some`なら`active=false`にして`on_pick.run((lat,lon))`(範囲外は**モードを維持**して指定し直せる)。どちらにせよ`return`。
  2. `draw_tool`のツール選択中: `pick_at_client`が`Some`なら`tool.click(lat,lon)`(範囲外は無視)。`return`。
  3. それ以外: `pick_track_at_client`の結果を`tracks.select(...)`(**何もない所のクリックは`None`=選択解除**)。
- **`wheel`**: `prevent_default`、`camera.zoom(factor)`、`render_now`。
- **`contextmenu`**: `prevent_default`。図形作成中なら`tool.undo()`して終了。ターゲットがcanvasでなければ終了。
  `ContextMenuState`と`MapMenuState`の**両方**があれば: `position = pick_at_client`、`track = pick_track_at_client`。`track`があれば先に`tracks.select(track)`。
  `position`か`track`のどちらかがあれば`menu.show(x, y, map_menu.0.run(MapMenuTarget{position, track}))`(項目が空ならメニューは出ない)。
  どちらか無ければ従来どおり`pick_at_client`が`Some`なら`radar_markers.add(lat, lon)`。
- **`dblclick`**: `draw_tool.finish()`(1回目・2回目のクリックはpointerupが点として置く。2回目は直前とほぼ同位置なので`click`が無視する)。
- **`keydown`(window)**: ツール選択中のみ。イベント対象が`INPUT/TEXTAREA/SELECT`なら無視。`Escape`→`cancel`、`Enter`→`finish`、`Backspace`→`prevent_default`+`undo`。`on_cleanup`でリスナーを外す。
- **2D/3D切替ボタン**: `camera.mode`を反転、表示用シグナル`view_mode`更新、`rebuild_markers`・`rebuild_tracks`・`render_now`。ボタン文言は現在3Dなら「2D表示に切替」、2Dなら「3D表示に切替」。

### 3.8 ピッキング(`picking.rs`)

- `pick_at_client(state, canvas, client_x, client_y) -> Option<(lat,lon)>`: `rect=canvas.getBoundingClientRect()`、`x=client_x-rect.left`、`y=client_y-rect.top`。terrain/mesh_origin/renderer無しはNone。
  `pick::pick_lat_lon(terrain, mesh_origin, camera, x, y, canvas.width(), canvas.height())`。
- `pick_track_at_client`: `x=(client_x-rect.left)*canvas.width/rect.width`(CSS pxからcanvas内部解像度へ。通常は同じ)、`y`同様。`view_proj = camera.view_proj_matrix()`、
  `(width,height)=renderer.canvas_size_px()`、`tracks::pick_track(pick_anchors, view_proj, (w,h), (x,y), PICK_RADIUS_PX)`。

### 3.9 オーバーレイの再構築(`overlay.rs`)

`GeometryInputs{ terrain, transform(mesh_origin), viewport_px }`から`BuildContext`を作る(`ground = |lat,lon| sample_heightmap(...).unwrap_or(0.0) as f64`)。

- `rebuild_markers(state, radar_markers)`: `build_marker_geometry`→`update_markers`。モードが3Dなら`build_dome_surface_geometry`→`update_dome`・`update_coverage_2d(&[])`、
  2Dなら`build_coverage_2d_geometry(…, radar_markers.coverage_altitude_m)`→`update_coverage_2d`・`update_dome(&[])`。
- `rebuild_drawings(state)`: `drawing_geometry::build(ctx, drawings.items)`→`update_drawings`。
- `rebuild_tracks(state)`: `TrackOptions{ selected, trails: show_trails, altitude_lines: show_altitude_lines && mode==3D }`(2Dは縦の線が点になるので出さない)→`build_track_geometry`→`update_tracks`。
  `pick_anchors = labels.map(|l| (l.id, l.position))`(**ラベルの表示設定に関係なく保持**)。`show_labels`がfalseならラベルは空を渡す(`set_labels`)。

### 3.10 航跡ラベル(`labels.rs`)

HTML要素の重ね合わせ(WebGPUに文字を描く機能が無いため)。`.terrain-track-labels`の子として、ラベル1つにつき
`<div class="track-label"><div class="track-label-name"/><div class="track-label-detail"/></div>`。
- `set_labels`: 数が変わったら子を全消去して作り直す(名前・詳細・色は初回に必ず異なる扱いにして入れる)。数が同じなら要素を使い回し、**変わった文字だけ**`set_text_content`。
  `selected`が変わったらクラスを`"track-label selected"`/`"track-label"`に。色が変わったら`style.color = rgb(r*255,g*255,b*255)`。
- `update_labels`(毎フレーム、`render_frame`から): 各ラベルのアンカーを`view_proj`でクリップ座標へ。`visible = clip.w>0 && |ndc_x|<=1.1 && |ndc_y|<=1.1`。
  可視なら`transform: translate({px}px, {py}px)`(`px=(ndc_x+1)/2·W + LABEL_OFFSET_X_PX(18)`、`py=(1-ndc_y)/2·H + LABEL_OFFSET_Y_PX(-16)`)と`display:block`、不可視なら`display:none`。

---

## 4. LOD適用ループ(`lod_driver.rs`)

計画(第2部)を、**画面が固まらないよう小分けに**進める。

### 4.1 定数

| 定数 | 値 | 意味 |
|---|---|---|
| `LOD_DEBOUNCE_MS` | 150 | カメラ操作が止まってからLOD更新するまでの待ち(操作が続く間はまとめる) |
| `LOD_CONTINUE_MS` | 8 | 取得完了・メッシュ反映の続きから次の更新までの待ち(デバウンスを待たない。150msだと取得の補充が間延びして全タイルのレベル1取得に約4.7秒かかっていた) |
| `UPLOAD_TIME_BUDGET_MS` | 12.0 | 1回の更新でメッシュを作ってGPUへ上げる時間の目安 |
| `MAX_UPLOAD_VERTICES_PER_ROUND` | 600,000 | 頂点数の安全上限(次の1個の重さを予測できないための保険) |
| `MAX_CONCURRENT_TILE_FETCHES` | 16 | 同時取得数(6だと全タイルのレベル1取得に1分ほど。HTTP/1.1同一ホスト接続は通常6本) |
| `DETAIL_CACHE_LIMIT_BYTES` | 300 MiB | 取得済み細かいレベルのグリッドの保持上限 |

### 4.2 予約

- `schedule_lod(state)`: `lod_pending`か地形未取得なら何もしない。`lod_pending=true`→`TimeoutFuture(150ms)`→`lod_pending=false`→`update_lod`。
- `schedule_lod_soon(state)`: 同様に`lod_soon_pending`と`TimeoutFuture(8ms)`。取得完了・反映の続きから呼ぶ。

### 4.3 `update_lod(state)`

`dragging`中は`schedule_lod`して終了(ドラッグ中は重い処理を避ける)。terrain/mesh_origin/renderer無しは終了。

`plan = lod::plan_levels(terrain, transform, camera, renderer.canvas_height_px(), &state.resident)`。`round_start = Date.now()`、`uploaded_vertices=0`、`changed=false`、`deferred=false`(取得の同時数上限で始められなかった)、`deferred_upload=false`(反映を次に回した)。
`over_budget(uploaded, cost) = uploaded > 0 && (経過 >= 12ms || uploaded + cost > 600,000)`(**1個は必ず進める**。0個だと永遠に終わらない)。

`request(key, level, chunk) -> bool`(true=上限で後回し): `fetch_key = level <= WHOLE_FILE_MAX_LEVEL(2) ? (key,level,None) : (key,level,Some(chunk))`。
`failed`か`loading`に含まれれば`false`(何もしない)。`loading.len() >= 16`なら`true`。それ以外は`loading`に入れ`to_fetch`へ積み`false`。

`plan`の各`(key, layout)`(優先度順)について:
- **`Whole`**: 現在`Chunks`なら、`build_whole_tile_mesh`→`set_mesh(WHOLE)`+全チャンクの`remove_mesh`、`resident[key]=Whole`、`terrain.set_whole_tile(key)`、`changed=true`(小さいので頂点数の上限には数えない)。
- **`Chunks(targets)`**:
  1. `available[c] = terrain.best_cached_level(tile, c, targets[c])`(目標以下で取得済みの最も細かいレベル。無ければ0)。
  2. 現在`Whole`(`Chunks`でない)なら、**全チャンクのレベル1が要る**: `available`に0があれば`request(key,1,0)`(戻りtrueなら`deferred=true`)して`continue`。
     揃っていれば`cost = Σ chunk_vertex_cost(available[c])`、`over_budget`なら`deferred_upload=true; continue`。
     各チャンクを`build_chunk_mesh(tile,c,available[c])`→`set_mesh((lat,lon,c))`+`terrain.set_chunk_level`、`remove_mesh(WHOLE)`、`resident[key]=Chunks(available)`、`uploaded+=cost`、`changed=true`。
  3. 各チャンクcについて目標に近づける(`want=targets[c]`, `have=levels[c]`, `now=available[c]`):
     `new_level = now==0 ? have : (want < have ? now : max(now, have))`。
     `!has_chunk_grid(tile, want, c)`なら`request(key, want, c)`(trueなら`deferred=true`)。`new_level==have`なら次へ。
     `cost=chunk_vertex_cost(new_level)`、`over_budget`なら`deferred_upload=true; continue`。`build_chunk_mesh`成功で`set_mesh`+`set_chunk_level`+`levels[c]=new_level`+`uploaded+=cost`+`resident_changed`。
     終わったら`resident_changed`なら`resident[key]=Chunks(levels)`。
     (取得済みの範囲でより細かければ先にそこまで上げ、目標のグリッドが届いたらさらに上げる=レベル1→2→3→4と段階的に細かくなる。)

`to_fetch`の各キーを`spawn_local`で取得: `chunk=None`→`fetch_tile_level`+`insert_tile_level`、`Some(c)`→`fetch_chunk_grid`+`insert_chunk_grid`。
完了後に`loading.remove`し、エラーなら`log::warn`+`failed.insert`。**デバウンスなしで`schedule_lod_soon`**(次の補充へ)。

`changed`のとき: ① 地形の高さが変わったので`target_up = sample_heightmap(origin) or 0`、`camera.target.z = ground_at_enu(target.xy).up`。
`terrain.evict_unused(keep = 画面に出している(resident[key]がChunksでlevels[chunk]==level), 300MiB)`。
② 観測点があれば`rebuild_markers`。③ `d.visible && shape.depends_on_terrain()`の作図があれば`rebuild_drawings`。④ トラックがあれば`rebuild_tracks`。⑤ `render_frame`。
最後に、`deferred_upload`なら`schedule_lod_soon`(反映の続き)、そうでなく`deferred`なら`schedule_lod`(念のため)。

---

## 5. その他のコンポーネント

### 5.1 `ui::tabbed_panel::{TabbedPanel, tab}`

```rust
pub struct Tab { label: &'static str, view: AnyView }
pub fn tab(label: &'static str, view: impl IntoView + 'static) -> Tab
#[component] pub fn TabbedPanel(#[prop(into)] title: String, tabs: Vec<Tab>, #[prop(optional)] active: Option<RwSignal<usize>>) -> impl IntoView
```
`active`を渡すと呼び出し側からタブを切り替えられる(渡さなければ内部で`RwSignal::new(0)`)。DOM:
`div.panel-section.tabbed-panel > (div.tabbed-panel-header > h2{title} + div.tab-bar > button.tab-button[.active]{label}…) + div.tabbed-panel-body > div.tab-content[style:display= 選択中?"flex":"none"]{view}…`。
**全タブの中身を初回に1度だけ生成してDOMに残し、非選択は`display:none`で隠す**(切替で作り直さない)。タブが1個でもタブバーは表示する。

### 5.2 `ui::floating_panel::FloatingPanel`

```rust
#[component] pub fn FloatingPanel(open: RwSignal<bool>, #[prop(into)] title: String, #[prop(default = true)] modal: bool,
                                  #[prop(optional)] draggable: bool, #[prop(optional)] initial_position: Option<(f64,f64)>, children: Children) -> impl IntoView
pub(crate) fn viewport_size() -> (f64, f64)      // window.inner_width/height、失敗時(1024,768)
```
定数: `KEEP_VISIBLE_X_PX=80`, `KEEP_VISIBLE_Y_PX=40`, `DEFAULT_WINDOW_POSITION=(80,60)`。中身は常時マウントし`display`だけ切り替える。
- **モーダル**: `div.floating-panel-backdrop[style:display= open?"flex":"none"][click: open=false] > パネル`。中央表示。
- **ウインドウ**(`modal=false`): `div.floating-window-layer[display= open?"block":"none"] > パネル`。パネルに`floating-panel--window`、`style:left/top`=初期位置。バックドロップなし(✕でだけ閉じる)。
- パネル: `div.floating-panel[.floating-panel--window][style:transform=translate(x,y)(0でなければ)][click: stop_propagation] > div.floating-panel-header[.draggable] > h3{title} + button.floating-panel-close[title=閉じる]{✕} ; div.floating-panel-body{children}`。
- **ドラッグ**(`draggable`): ヘッダの`pointerdown`(左ボタン・`closest("button")`内でない)で`Drag{start, base=offset, dx_range, dy_range}`を記録し`set_pointer_capture`。
  `dx_range = (80 - rect.right, vw - 80 - rect.left)`、`dy_range = (-rect.top, vh - 40 - rect.top)`(**タイトルバーが必ず画面内に残る**: 右端が80px以上・左端が(幅-80)以下・上端が0以上・上端が(高さ-40)以下)。
  `pointermove`で`dx = clamp(client差分, range)`(`hi.max(lo)`で範囲が反転しても安全)、`offset = base + (dx,dy)`。`pointerup/cancel`で`drag=None`。位置は閉じて開き直しても保つ。未実装: リサイズ・最小化・重なり順・位置の永続化。

### 5.3 `ui::context_menu`

```rust
pub enum MenuItem { Action{label, enabled: bool, on_select: UnsyncCallback<()>}, Submenu{label, items}, Label(String), Separator }
MenuItem::action(label, f)  // enabled=true      disabled(label)  // enabled=false, 何もしない
.enabled(bool)              // Actionだけに効く   submenu(label, items)  label(text)  separator()
#[derive(Clone, Copy)] pub struct ContextMenuState { open: RwSignal<Option<OpenMenu{x,y,items}>> }
new() / show(x, y, items)(itemsが空なら何もしない) / close() / is_open()
#[derive(Copy)] pub struct MapMenuTarget { pub position: Option<(f64,f64)>, pub track: Option<TrackId> }
#[derive(Clone, Copy)] pub struct MapMenuState(pub UnsyncCallback<MapMenuTarget, Vec<MenuItem>>);   new(build: Fn(MapMenuTarget)->Vec<MenuItem>)
#[component] pub fn ContextMenu()
```
定数: `EDGE_MARGIN_PX=4`、`SUBMENU_WIDTH_ESTIMATE_PX=240`。
- 本体: `state.open`を見て、開いていれば `div.context-menu-backdrop[pointerdown: close][contextmenu: prevent_default+close] > div.context-menu[left/top/visibility][pointerdown: stop_propagation][contextmenu: prevent_default+stop_propagation]{menu_view(items)}`。
  **背景の`pointerdown`で閉じる(そのクリックは背後へ通さない)**。Escで閉じる(windowの`keydown`、`on_cleanup`で除去)。
- **位置補正**: 開いた直後は指定位置に`visibility:hidden`で置き、`request_animation_frame`で`getBoundingClientRect`を測って、`right > vw-4`なら`x = max(vw - width - 4, 0)`、`bottom > vh-4`なら`y = max(vh - height - 4, 0)`へずらし、それから`visible`にする(指定位置から一瞬ずれて見えるのを防ぐ)。
- `menu_view(items, state)`: `Separator`→`div.context-menu-separator`、`Label`→`div.context-menu-label`、`Action`→`button.context-menu-item[disabled]`(`mouseenter`で開いているサブメニューを閉じる、
  `click`で**先に`state.close()`してから`on_select.run(())`**=呼んだ先で別のメニューを出せる)。
  `Submenu`→`div.context-menu-parent[mouseenter: 右に収まらなければflip判定して開く] > button.context-menu-item.context-menu-item--submenu{label + span.context-menu-arrow ▶}` + 開いていれば `div.context-menu.context-menu-sub[.flip]`(入れ子)。
  flip判定: `項目の right + 240 > vw`なら左へ開く。
- 未実装: 矢印キー移動・ショートカット表示・チェック付き項目。

### 5.4 `ui::origin_dialog::{OriginDialog, OriginDialogState}`

`OriginDialog(on_submit: UnsyncCallback<(f64,f64)>)`。`OriginState`・`OriginDialogState`・`TerrainStore`を`use_context`(無ければpanic)。`Effect::new(|| terrain_store.ensure_loaded())`。
- `lat_input`/`lon_input: RwSignal<String>`、`dirty: RwSignal<bool>`(手で編集を始めたら`OriginState`受信による自動上書きを止める=再配信で入力中の値が消えない)。
- `OriginState`が変わるたび、`dirty`でなければ入力欄へ`format!("{:.6}", …)`。
- `parsed() -> Result<(f64,f64), String>`: `trim().parse::<f64>()`失敗→「緯度は数値で入力してください」/「経度は数値で入力してください」。地形データ未取得→「地形データ範囲を取得中です...」。
  範囲外→「緯度は{min:.1}〜{max:.1}の範囲で入力してください」/「経度は…」(`geodetic_bounds`)。
- UI: `FloatingPanel(open=dialog.0, title="原点設定")`内に `div.origin-form-row > label{緯度 + input[type=text][inputmode=decimal]} + label{経度 …} + button{設定}[disabled=parsed().is_err()]`、その下に`p.field-error{msg}`(エラー時)。
  `input`で`dirty=true`。「設定」で`Ok`なら`on_submit.run((lat,lon))`、`dirty=false`(次に届く`OriginState`で表示を更新)。
- 送信は**アプリの責務**(ライブラリは`on_submit`を呼ぶだけ)。クロージャがムーブする非`Copy`値(接続)は、`Fn/FnMut`の外で1回だけ作ると2回目以降でムーブ済みエラーになるので、開閉のたびに実行される内側で`clone()`する。

### 5.5 `ui::coverage_altitude_dialog::{CoverageAltitudeDialog, CoverageAltitudeDialogState}`

`FloatingPanel(open, title="覆域高度設定")`内: `p.dialog-description{"選択中のレーダーの探知可能領域を表示する際の海抜高度(m)(2D表示モードで使用)。"}`、
`div.origin-form-row > label{"高度(m)" + input[type=number][step=10][prop:value=coverage_altitude_m]}`。`input`で数値としてパースできたら`radar_markers.coverage_altitude_m.set(v)`。
再構築は`TerrainView`のEffect 4がシグナル経由で行う(ダイアログからジオメトリ再構築を直接呼ばない)。通信なし。

### 5.6 `ui::los_view::LosView`(見通し範囲タブ)

`TerrainStore`・`RadarMarkersState`が必要。`div.los-view > div.los-chart{chart} + div.los-controls > div.los-marker-list{list}`。
- **一覧**: 観測点が無ければ`p.placeholder.los-status`「メインパネル(中央の地図)を右クリックして、レーダー観測点を追加してください」。各行
  `div.los-marker-row[.los-marker-row-selected][click: selected=id]` > `span.los-marker-label{"緯度{:.4} 経度{:.4}"}` + `label.los-param-label{"高(m)" input[number][min=0][step=1]}`(`input`で`height_m = max(v,0)`)
  + `label.los-param-label{"範囲(km)" input[number][min=1][step=1]}`(表示は`max_range_m/1000`、`input`で`max_range_m = max(v,1)*1000`)+ `button.los-delete-btn[title=このレーダーを削除]{削除}`(`stop_propagation`+`remove(id)`)。入力欄のclickは`stop_propagation`。
- **極座標図(SVG)**: 地形未取得→「地形データを読み込み中...」、未選択→「レーダーが選択されていません」、計算不能→「計算できません」。選択中の観測点を原点として`compute_los(data, origin=観測点, LosParams)`。
  `VIEW_SIZE=300, PAD=26, RADIUS=(300-52)/2=124, CENTER=150`。`svg.los-svg[viewBox="0 0 300 300"][role=img][aria-label=見通し範囲図(レーダー覆域図)]`:
  距離グリッド円(半径の0.25/0.5/0.75/1.0倍、`circle.los-grid-circle`)、十字軸(`line.los-axis`×2)、境界`path.los-area`(塗り)と`path.los-outline[fill=none]`、観測点`circle.los-observer[r=3]`、
  N/E/S/Wの`text.los-label`(N: (150, 150-124-6, middle) / E: (150+124+8, 154) / S: (150, 150+124+14, middle) / W: (150-124-8, 154, end))、最大距離`"{:.0}km"`のラベル(150+4, 150-124+11)。
  境界パス: 各点`r = clamp(range_m/max_range, 0, 1)*RADIUS`、`x = CENTER + r·sin(az)`、`y = CENTER - r·cos(az)`、`M`/`L`でつないで`Z`(小数1桁)。`max_range = max(marker.max_range_m, 1)`。

### 5.7 `ui::cross_section_view::CrossSectionView`(断面図タブ)

`OriginState`・`TerrainStore`・`RadarMarkersState`が必要。`azimuth: RwSignal<f64>`(既定0)。`div.cross-section-view > div.cross-section-chart{chart} + div.cross-section-controls > label.cross-section-slider-label{"方位角(北=0°・時計回り)" input[type=range][0..359][step=1]} + span.cross-section-azimuth-value{"{:.0}°"}`。
- 原点 = `OriginState`(無ければ`default_origin`)。`points = build_profile(data, origin, azimuth)`。2点未満→「断面を計算できません」。地形未取得→「地形データを読み込み中...」。
- 定数: `VIEW_W=400, VIEW_H=220, PAD_L=46, PAD_R=10, PAD_T=10, PAD_B=22, SKY_MARGIN_M=10_000`。`plot_w = VIEW_W-PAD_L-PAD_R`、`plot_h = VIEW_H-PAD_T-PAD_B`。
  `max_distance = max(最後の点のdistance, 1)`、`(min_elev, max_elev)`=断面の最小最大、`sky_ceiling = max_elev + 10000`(Y軸の上端)。
  座標: `x = PAD_L + d/max_distance·plot_w`、`y = PAD_T + plot_h - (elev-min_elev)/(sky_ceiling-min_elev)·plot_h`(`elev_range = max(_,1)`)。
- 描画: 軸(左と下、`line.cs-axis`)、覆域(上空を含む)`path.cs-airspace`(観測点があれば)、地表の塗り`path.cs-area`(折れ線+右下・左下を通って閉じる)、地表の折れ線`path.cs-line[fill=none]`、
  覆域内の地表トラック`path.cs-coverage[fill=none]`、最高標高の目盛`line.cs-axis`と`text.cs-label`(`"{max_elev:.0}m"`、`"+{sky_ceiling-max_elev:.0}m"`(上端)、`"{min_elev:.0}m"`(下端)、`"{max_distance/1000:.1}km"`(右下))。
  `svg.cross-section-svg[viewBox="0 0 400 220"][preserveAspectRatio=none][role=img][aria-label=地形断面図(覆域は上空を含む)]`。
- 覆域(観測点が1つ以上のとき):
  - `covered[i] = いずれかの観測点で is_visible(観測点, 断面上の点i)` → 連続する`true`の区間だけを`M…L…`でつなぎ、途切れたら新しい部分パス(`build_coverage_path`)。
  - `boundary[i] = 全観測点の min_visible_altitude の最小値`(1つでも見えれば覆域内なので最も緩い=低い下限。どの観測点の範囲にも入らなければ`None`)。
    `build_airspace_path`: `Some(h)`かつ`h < sky_ceiling`の連続区間を、下端`y_of(max(h, min_elev))`から天井`PAD_T`までの帯として閉じたポリゴンにする(2点未満の区間は捨てる)。
    `None`または`h>=sky_ceiling`で区間を切る。

### 5.8 `ui::drawing_editor::DrawingEditor`

`DrawToolState`(`use_context`、無ければpanic)。`ContextMenuState`は任意。DOM: `div.drawing-editor`
1. `p.drawing-help`「図形を選んで、地図をクリックして置きます。数値は下の一覧から選んで編集できます。」
2. `div.drawing-tool-grid` > `ToolKind::ALL`の`button.drawing-tool-button[.active = tool == kind][click: tool.start(kind)]{label}`。
3. `details.drawing-section > summary{"新しい図形の見た目・高度"}` + `style_fields(new_style, with_fill=true)` + `altitude_fields(new_altitude)`。
4. `div.drawing-list-header > span{"作った図形({n})"} + button.los-delete-btn{すべて削除}[disabled=空]`(`window.confirm("作った図形をすべて削除しますか?")`が真のときだけ`remove_all`。`confirm`が出せない環境では**削除しない**)。
5. `div.drawing-list > <For each=shapes key=(id,name)>` 行 + 空なら`p.placeholder{"まだ図形がありません"}`。行=`div.drawing-row-item[.selected] > input[checkbox][title=表示/非表示] + button.drawing-row-name[title=選択して編集]{name}(クリックで選択トグル) + button.los-delete-btn[title=この図形を削除]{削除}`。
   行の右クリック(`ContextMenuState`があれば): 選択して`名前を変更`(次フレームで`.drawing-form .drawing-name input`へfocus)/`複製`/`非表示にする|表示する`/区切り/`削除`。
   **行は「一覧の追加・削除・改名」でだけ作り直す**(図形の中身の変化=編集や仮の図形の更新では作り直さない。入力中のフォーカスが外れるため。表示/非表示は行の中で読む)。
6. 編集フォーム(`Memo`で`(選択id, 種類タグ, 点の数)`が変わったときだけ作り直す): `div.drawing-form`:
   `label.drawing-field.drawing-name{名前 input[text][change: rename]}` → 位置行(点が複数なら`"点{i}"`、1つなら`"位置"`。緯度・経度`num_field`、`clamp(-90,90)`/`clamp(-180,180)`)
   → 種類ごとのパラメータ → 高度(`altitude_fields`。基準select「地表から(地形に沿う)」/「海抜」+値。**全点に適用**)→ `style_fields`(折れ線は塗り行なし)。

パラメータ表(ラベル・単位): Circle/Sphere=半径(m) / Rect=幅(東西)(m)・高さ(南北)(m)・回転(°) / Sector=半径(m)・開始方位(°)・終了方位(°) /
Cuboid=幅(東西)(m)・奥行(南北)(m)・高さ(m)・方位(°) / Cylinder/Cone=半径(m)・高さ(m) / Polygon/Polyline=なし(位置のみ)。
**大きさは`max(v, 1.0)`m以上**にする(0以下だと図形が描かれず見失う)。角度はそのまま。

`num_field(label, unit, get, set)`: `label.drawing-field > span{"{label}({unit})"} + input[type=number][step=any][prop:value=fmt_num(get(), unit=="°" ? 6 : 3)]`、`change`(フォーカスを外す・Enter)で`trim().parse::<f64>()`が有限なら`set(v)`。`fmt_num(v, d) = (v·10^d).round()/10^d`。
`style_fields(get, set, with_fill)`: 塗り行(チェック+`input[type=color]`+不透明度`input[type=range][0.05..1][step 0.05]`。チェックONの既定色=`Style::default().fill`)、線行(チェック(`with_fill`のとき)+色+太さ`num_field("太さ","px")`を`clamp(0.5, 50)`)。色は`#rrggbb`⇔`Color`(`to_u8`=`round(v·255)`)。

### 5.9 `ui::util::copy_to_clipboard(text)`

`navigator.clipboard`(https/localhostのみ)を`js_sys::Reflect`で引いて`writeText(text)`を呼ぶ。無い・非対応なら`log::warn`して何もしない。

---

## 6. CSS契約 `style/sim3dview.css`

見た目(色・余白・フォント)は自由だが、**以下のクラス名とレイアウト上の必須ルール**は守る。テーマは呼び出し側の`:root`の CSS変数 `--bg-panel`(パネル背景)・`--fg`(文字)・`--border`(枠線)・`--accent`(強調)。
参考値: `--bg-panel:#181818; --fg:#eee; --border:#3a3a3a; --accent:#9cf`。未定義でも動くが配色が付かない(`var(--x, fallback)`で保険を書いてよい)。

### 6.1 レイアウト上の必須ルール

| セレクタ | 必須の性質 |
|---|---|
| `.terrain-view` | `position:relative; width:100%; height:100%; flex:1 1 auto; min-height:0`(ResizeObserverの対象。**Browserペインを表示した状態でないと発火しない**) |
| `.terrain-canvas` | `display:block; width:100%; height:100%; touch-action:none`(カーソル`grab`、`:active`で`grabbing`)。`.origin-pick-active`は`cursor:crosshair` |
| `.terrain-track-labels` | `position:absolute; inset:0; overflow:hidden; pointer-events:none` |
| `.track-label` | `position:absolute; left:0; top:0; display:none; white-space:nowrap`(位置は`transform`で毎フレーム指定)。`.selected`は名前を枠で囲む |
| `.origin-pick-hint` | `position:absolute; top:8px; left:8px; z-index:2; display:flex; flex-wrap:wrap; max-width:calc(100% - 150px)`(右上の切替ボタンと重ならない) |
| `.terrain-view-controls` | `position:absolute; top:8px; right:8px; z-index:2; display:flex` |
| `.map-status` | `position:absolute`(z-index 1)。中央付近に状態文言 |
| `.floating-panel-backdrop` | `position:fixed`(全画面)・半透明・z-index 20。`display`は`flex`/`none`をインラインで切替(中央寄せ) |
| `.floating-window-layer` | `position:fixed`(全画面)・z-index 15・**`pointer-events:none`**。`.floating-panel--window`は`position:absolute; pointer-events:auto`。`.floating-panel-body`はスクロール可 |
| `.floating-panel-header.draggable` | `cursor:move; touch-action:none` |
| `.context-menu-backdrop` | `position:fixed`(全画面)・**z-index 30** |
| `.context-menu` | `position:fixed`。`.context-menu-parent`は`position:relative`、`.context-menu-sub`は`position:absolute`(親の右に開く)、`.context-menu-sub.flip`は左に開く |
| `.tab-content` | インライン`display`が`flex`/`none`で切り替わる |

### 6.2 クラス名一覧(すべて使う)

汎用: `.placeholder .panel-section .tabbed-panel .tabbed-panel-header .tab-bar .tab-button(.active) .tabbed-panel-body .tab-content`
地図: `.terrain-view .terrain-canvas .terrain-track-labels .track-label(.selected) .track-label-name .track-label-detail .origin-pick-hint .terrain-view-controls .map-status`
断面図: `.cross-section-view .cross-section-chart .cross-section-svg .cs-axis .cs-area .cs-line .cs-coverage .cs-airspace .cs-label .cross-section-status .cross-section-controls .cross-section-slider-label .cross-section-azimuth-value`
見通し: `.los-view .los-chart .los-svg .los-grid-circle .los-axis .los-area .los-outline .los-observer .los-label .los-status .los-controls .los-marker-list .los-marker-row(.los-marker-row-selected) .los-marker-label .los-param-label .los-delete-btn`
ダイアログ: `.floating-panel-backdrop .floating-panel .floating-panel-header .floating-window-layer .floating-panel--window .floating-panel-body .floating-panel-close .dialog-description .origin-form-row .field-error`
作図: `.drawing-editor .drawing-help .drawing-tool-grid .drawing-tool-button(.active) .drawing-section .drawing-form .drawing-row .drawing-field .drawing-name .drawing-point-label .drawing-inline .drawing-check .drawing-style .drawing-list-header .drawing-list .drawing-row-item(.selected) .drawing-row-name`
右クリックメニュー: `.context-menu-backdrop .context-menu .context-menu-sub(.flip) .context-menu-parent .context-menu-item(--submenu) .context-menu-arrow .context-menu-label .context-menu-separator`

アプリは`index.html`(Trunk)で`<link data-trunk rel="css" href="../../sim3dview/style/sim3dview.css" />`を読み込む(相対パスは自分のCargo.tomlからの位置に合わせる。読めない構成ではファイルをコピーする)。

---

## 7. 実機確認の手順(Browserペイン)

UIの動作確認はBrowserペインを**表示した状態**で行う(非表示だとResizeObserverが発火しない)。プライベートIP宛はブロックされるので`http://localhost:8081`。
- 初回: 「地形データを読み込み中...」→地形が出る(全タイルがレベル0→約20秒でレベル1→近い順に細かく)。
- 3D: ドラッグで回転、ホイールでズーム、Shift+ドラッグで注視点移動。地面の下にもぐらない。2D切替で北が上・ドラッグでパン。
- 右クリック(メニュー未提供): 観測点が追加され、3Dでドーム・2Dで塗り+輪郭が出る。見通しタブの極座標図が更新される。
- 海・データ範囲外は水色の水域。水平線付近で地球の丸みの向こうの地形が水面越しに透けない。
- 判断を1枚のスクリーンショットだけで下さない(同じ操作を複数回再現する)。
