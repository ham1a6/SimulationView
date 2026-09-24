# sim3dview APIリファレンス

アプリから利用する公開APIの入口、状態の更新方法、組み込み時の前提をまとめる。
使用例とCSS契約は[README](README.md)、変更時の作業ルールは[実装ガイドライン](IMPLEMENTATION_GUIDELINES.md)を参照する。
設計判断とアルゴリズムの正は[設計書](docs/DETAILED_DESIGN.md)に置く。

## 型・メソッドの詳細を調べる

リポジトリのルートで次を実行すると、現在のソースのシグネチャ・フィールド・列挙値・説明を検索できる。

```powershell
cargo doc -p sim3dview --no-deps --target wasm32-unknown-unknown --open
```

既定の出力は`target/wasm32-unknown-unknown/doc/sim3dview/index.html`。
以下の表は公開モジュール全体の索引で、個々のメソッドの完全な一覧はrustdocを参照する。
`pub(crate)`の測地変換、地形ローダー、LOD、レンダラー、見通し計算は外部からimportできない。
`TerrainStore::get()`の戻り値は型推論で利用できるが、内部モジュールの型パスをアプリに書かない。

## 初期化と状態の寿命

[`viewer::ViewerState`](src/viewer.rs)は共有状態をまとめて生成する任意の補助API。
LeptosのコンポーネントなどOwnerが有効な場所で生成し、子ビューの生成前に`provide()`する。
ハンドルのコピーは同じ状態を参照する。Ownerの破棄後にハンドルを使わない。

| API | 引数・戻り値と動作 |
|---|---|
| `ViewerState::new(terrain_base_url)` | `impl Into<String>`を受けて状態を返す。生成だけでは通信しない |
| `.persist_drawings(key)` | `&'static str`のlocalStorageキーで対話作図の保存・復元を有効化し、自分を返す。登録前に1回呼ぶ |
| `.provide()` | 10個の共有状態を現在のOwnerへ登録し、自分を返す。同じOwnerで二重登録しない |

```rust
use leptos::prelude::*;
use sim3dview::{
    terrain::camera::CameraPreset,
    ui::terrain_view::TerrainView,
    viewer::ViewerState,
};

#[component]
fn App() -> impl IntoView {
    let viewer = ViewerState::new("/terrain").provide();
    // ボタンなどのアプリ固有UIから、共有状態を操作する。
    view! {
        <button on:click=move |_| viewer.recenter.request()>"原点を見る"</button>
        <div style="height: 600px; position: relative;">
            <TerrainView preset=CameraPreset::Overview/>
        </div>
    }
}
```

CSSの読み込みとテーマ設定は[README](README.md)の「最小構成の使用例」「テーマ」を参照。
親要素の高さが0だとビューを初期化できない。地形配信のベースURLはアプリが決める。

### TerrainViewのcontext

| context | ViewerStateのフィールド | 個別登録時の扱い |
|---|---|---|
| `TerrainStore` | `terrain` | 必須。全パネルで同じストアを共有 |
| `OriginState` | `origin` | 必須。`None`は地形metadataの既定原点を使用 |
| `RadarMarkersState` | `radar_markers` | 必須 |
| `RecenterRequestState` | `recenter` | 任意。注視点の移動要求 |
| `HillshadeState` | `hillshade` | 任意。未提供時は陰影ON |
| `CaptureState` | `capture` | 任意。保存・録画要求 |
| `DrawingState` | `drawings` | 任意。図形一覧 |
| `DrawToolState` | `draw_tool` | 任意。同じ`DrawingState`から生成する |
| `TracksState` | `tracks` | 任意。未提供時は航跡なし |
| `ModelsState` | `models` | 任意。未提供時はシンボル表示 |
| `OriginPickState` | 登録しない | 原点指定用コールバックをアプリが設定 |
| `ContextMenuState`と`MapMenuState` | 登録しない | 両方の登録で地図右クリックがメニューになる |

ダイアログの開閉contextも`ViewerState`には含まれない。必須contextの欠落はpanicになる。
個別登録は[README](README.md)の最小構成を参照する。

## 地形・原点・カメラ

| モジュール | 主なAPI | 利用契約 |
|---|---|---|
| [`terrain::store`](src/terrain/store.rs) | `TerrainStore::new`, `ensure_loaded`, `get`, `get_untracked`, `error` | 取得中・取得済みなら`ensure_loaded()`は何もしない。失敗は`error: RwSignal<Option<String>>`へ通知 |
| [`terrain::origin`](src/terrain/origin.rs) | `Origin { lat_deg, lon_deg }`, `OriginState` | 緯度・経度は度。受信した確定原点を`.0.set(Some(origin))`で反映 |
| [`terrain::recenter`](src/terrain/recenter.rs) | `RecenterRequestState::request`, `request_at` | `request()`は原点、`request_at(lat, lon)`は指定位置を見る。原点自体は変更しない |
| [`terrain::origin_pick`](src/terrain/origin_pick.rs) | `OriginPickState` | `active`でクリック指定を開始し、`on_pick`で緯度経度を受け取る |
| [`terrain::camera`](src/terrain/camera.rs) | `CameraPreset`, `ViewMode`, `Camera`, `Projection`, `OrbitCamera` | 通常の組み込みは`TerrainView`の`preset`を使う。カメラの低水準操作はENU座標・射影を扱う |
| [`terrain::hillshade`](src/terrain/hillshade.rs) | `HillshadeState::new`, `enabled` | `enabled.set(bool)`で陰影を切り替える |
| [`terrain::capture`](src/terrain/capture.rs) | `CaptureState::request_screenshot`, `toggle_recording`, `is_recording` | 保存要求をビューへ通知。録画ボタンの表示には要求値でなく実際の`is_recording`を使う |

`get()`はリアクティブな購読、`get_untracked()`はその時点の読み取り。
地形の初回取得失敗後は`ensure_loaded()`を再度呼べるが、自動再試行や成功時の`error`クリアは実装されていない。
録画開始に失敗すると録画要求はfalseへ戻る。画像・録画の対象などは設計書6.14節を参照。

サンプル構成では原点の正はC++の`OriginState`である。原点変更要求を送った時点で楽観更新せず、
サーバーが認めて配信した値を反映する。停止中だけ変更できる制御と拒否理由の表示はアプリ側で行う。

## 観測点・作図・航跡・モデル

| モジュール | 主なAPI | 更新方法と注意点 |
|---|---|---|
| [`terrain::markers`](src/terrain/markers.rs) | `RadarMarker`, `RadarMarkersState`, `coverage_colors` | `add(lat, lon) -> u64`, `remove(id)`。`markers`・`selected`・`coverage_altitude_m`・`show_all_coverage`を共有 |
| [`terrain::drawing`](src/terrain/drawing.rs) | `DrawingState`, `DrawingId`, `Drawing`, `Shape`, `Position`, `Space`, `Altitude`, `Style`, `Color`, `Corner` | `add(shape, style) -> DrawingId`, `update(id, closure)`, `remove(id)`, `clear()` |
| [`terrain::draw_tool`](src/terrain/draw_tool.rs) | `DrawToolState`, `ToolKind`, `UserShape`, `build_shape` | `start`/`start_at`で開始、`click`で点追加、`finish`で確定、`undo`で戻す、`cancel`で中止 |
| [`terrain::tracks`](src/terrain/tracks.rs) | `TracksState`, `Track`, `TrackEntry`, `TrackId`, `SymbolKind`, `Affiliation` | `set(Vec<Track>)`で毎回全件を置換。`select`で選択、`selected_track`で最新値を購読、`clear`で全消去 |
| [`terrain::models`](src/terrain/models/mod.rs) | `ModelsState`, `ModelSource`, `ModelDisplayMode` | `set_source(kind, source)`で種別ごとのGLB URLを登録。`mode`で表示方式を指定 |

座標は緯度・経度の順で度、距離と高度はm、速度はm/s。`Altitude::Msl`は海抜、
`AboveGround`は地形からの高さ。`Track`のheadingは北から時計回り、pitchは機首上げ、rollは右翼下げが正の度。
描画の海抜の扱いと楕円体高の近似は設計書3.2節を参照する。

作図の`Shape::validate()`は座標空間の整合性を確認する。`DrawingState::add`は検証を行わないため、
アプリで任意入力を組み立てる場合は事前に検証する。`update`はIDが存在しなければ何もしない。
対話作図の編集は`DrawToolState::update_shape`・`rename`・`duplicate`・`remove`を使い、選択やプレビューとの整合性を保つ。
対話ツールが管理する図形だけが保存・復元と`remove_all`の対象で、アプリが直接追加した図形とは区別される。

航跡は同じ実体に同じIDを与える。差分だけを`set`すると未送信のトラックが消える。
消えたIDの軌跡と選択も削除される。差分配信を使うアプリは自身で全件の最新状態を組み立てる。
`selected_track_untracked()`は位置更新を購読せずに読みたい場合に使う。

GLBは単一ファイルの三角形メッシュ・法線・頂点色・基本色に対応する。
テクスチャ、アニメーション、スキン、外部ファイル参照は非対応。
未登録・読込中・読込失敗時はシンボルへフォールバックする。尺度と向きの調整例は[README](README.md)を参照。

## 距離・方位の測定

[`terrain::measurement`](src/terrain/measurement.rs)はcontext・ブラウザ・地形取得なしで利用できる。

| API | 戻り値 | 入力・測定対象 |
|---|---|---|
| `distance_and_bearing(lat0, lon0, lat1, lon1)` | `(f64, f64)` | WGS84最短測地距離mと初期方位度。有限値と有効緯度を呼び出し側が保証 |
| `measure_route(&[(lat, lon)])` | `Result<RouteMeasurement, InvalidRoutePoint>` | 隣接点の測地距離。緯度±90度・経度±180度の有限値を検証 |
| `measure_route_at_height(&[(lat, lon)], height_m)` | `Result<RouteMeasurement, FlightMeasurementError>` | 地表の測地線を一定のWGS84楕円体高へ持ち上げた経路。高度は0〜1,000,000mの有限値 |

`RouteMeasurement`は`segments`と`total_distance_m`、各`RouteSegment`は区間距離・累積距離・初期方位を持つ。
経路は入力順で、閉じる場合は末尾にも始点を渡す。0〜1点は区間なし・距離0。
不定・不安定な方位は`initial_bearing_deg: None`で返す。不正座標は0始まりの`index`で特定できる。
高度付き測定は海抜・対地高度でも空間直線距離でもなく、地形との衝突判定もしない。

## UIコンポーネント

各項目の型パスは`sim3dview::ui::<モジュール>::<項目>`。propsの正確な型・既定値はリンク先とrustdocを参照。

| モジュール / 項目 | 主なprops・必須context |
|---|---|
| [`terrain_view::TerrainView`](src/ui/terrain_view/mod.rs) | `preset: CameraPreset`。地形・原点・観測点の3 contextが必須 |
| [`los_view::LosView`](src/ui/los_view.rs) | 地形・観測点のcontext。見通し範囲と観測点設定 |
| [`cross_section_view::CrossSectionView`](src/ui/cross_section_view.rs) | 地形・原点・観測点のcontext。`TracksState`は任意 |
| [`drawing_editor::DrawingEditor`](src/ui/drawing_editor.rs) | `DrawToolState`。`ContextMenuState`は任意 |
| [`origin_dialog::OriginDialog`](src/ui/origin_dialog.rs) | `on_submit: UnsyncCallback<(f64, f64)>`、地形・原点・`OriginDialogState` |
| [`coverage_altitude_dialog::CoverageAltitudeDialog`](src/ui/coverage_altitude_dialog.rs) | `RadarMarkersState`と`CoverageAltitudeDialogState` |
| [`model_settings_dialog::ModelSettingsDialog`](src/ui/model_settings_dialog.rs) | `ModelsState`と`ModelSettingsDialogState` |
| [`context_menu::ContextMenu`](src/ui/context_menu.rs) | `ContextMenuState`。`MenuItem`で項目、`MapMenuState`で地図用項目生成を設定 |
| [`floating_panel::FloatingPanel`](src/ui/floating_panel.rs) | `open`, `title`, children。`modal`, `draggable`, `initial_position`で動作を指定 |
| [`tabbed_panel::TabbedPanel`, `tab`, `Tab`](src/ui/tabbed_panel.rs) | `tabs`と任意の`active`。タブの内容を保持したまま表示切替 |
| [`split_pane::SplitPane`](src/ui/split_pane.rs) | `first`, `second`。比率・最小幅・右ペイン表示を指定 |
| [`pointer_drag::DragTracker`, `DragUpdate`, `DragEnd`](src/ui/pointer_drag.rs) | `begin`, `update`, `end`, `cancel`。DOMのpointer captureは呼び出し側で行う |
| [`util::copy_to_clipboard`](src/ui/util.rs) | `&str`。Clipboard APIが利用できない場合はログを出す |

右クリックメニューは`MapMenuTarget`の地表位置とトラックからアプリが項目を構築する。
`ContextMenuState`と`MapMenuState`のどちらかが無ければ、地図右クリックは観測点追加になる。
作図中の右クリックは点を戻す操作が優先される。

## 組み込み例の所在

| 目的 | 実装例 |
|---|---|
| 状態登録と画面構成 | [sample/sim_frontend/src/app.rs](sample/sim_frontend/src/app.rs) |
| 通信データから航跡への変換 | [track_bridge.rs](sample/sim_frontend/src/track_bridge.rs) |
| 地図の右クリックメニュー | [map_menu.rs](sample/sim_frontend/src/components/map_menu.rs) |
| 各座標空間の図形 | [drawing_demo.rs](sample/sim_frontend/src/components/drawing_demo.rs) |

データ配信形式とCSSは[README](README.md)、内部変更の検証は[実装ガイドライン](IMPLEMENTATION_GUIDELINES.md)へ。
