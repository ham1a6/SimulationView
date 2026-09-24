# sim3dview

## ドキュメントの入口

| 目的 | 文書 |
|---|---|
| 組み込み・機能別の使用例・CSS | 本README |
| 公開APIの検索・contextと更新方法 | [APIリファレンス](API_REFERENCE.md) |
| 機能追加・修正時の責務、実装ルール、検証 | [実装ガイドライン](IMPLEMENTATION_GUIDELINES.md) |
| 設計理由・アルゴリズム・データ形式 | [統合設計書](../docs/DETAILED_DESIGN.md) |

型・メソッドの完全なシグネチャは`cargo doc -p sim3dview --no-deps --target wasm32-unknown-unknown --open`で参照できます。

ALOS DEMベースの3D地形描画(wgpu)・レーダー覆域/見通し(Line of Sight)計算・レーダー観測点
管理を提供する、[Leptos](https://leptos.dev/)(WASM/CSR)向けのRustライブラリです。

**このライブラリに含まれないもの**: VAB(操作ボタン)・状況パネル・メニューバーのような
アプリ固有のUI、および特定のバックエンドと通信するためのプロトコル(WebSocket/msgpack等)は
含みません。それらは呼び出し側(あなたのアプリ)がRust等で実装します。実際にこのライブラリを
組み込んだ最小構成のサンプルアプリ(C++サーバー込み)は、リポジトリの`sample/`ディレクトリに
あります。まずそちらを一通り動かしてから、このREADMEで各部品の使い方を確認するのが早道です。

## 提供するもの

- `terrain`モジュール: 地形データ取得・座標変換・カメラ・wgpu描画・見通し/覆域計算・
  レーダー観測点の状態管理・作図(図形・線。絶対座標固定/カメラ固定。UIから作る図形の対話作成を含む)・
  航跡表示(航空機等の現在位置とシンボル)
- `ui`モジュール: 上記を使ったLeptosコンポーネント一式(3D/2D地形描画canvas、見通し範囲タブ、断面図タブ、
  タブ付きパネル、汎用フローティングパネル(モーダル/動かせるウインドウ)、原点設定・覆域高度設定ダイアログ、
  作図エディタ、右クリックメニュー)と、Pointer Events用の純粋なドラッグ状態管理
- `style/sim3dview.css`: 上記コンポーネントのスタイル

公開しているのは、アプリが使う`terrain::{camera, capture, draw_tool, drawing, hillshade, markers, models, origin, origin_pick, recenter, store, tracks}`と
`terrain::measurement`の距離・方位計算、`viewer::ViewerState`と`ui`の各部品です。それ以外の`terrain`のモジュール(ENU座標変換・LOD・描画・見通し計算など)はライブラリの内部(`pub(crate)`)です。
モジュール構成は[DETAILED_DESIGN.md](../docs/DETAILED_DESIGN.md) 6.0節を参照してください。

> このライブラリを**1から再実装したい**(仕様どおりに作り直す・別環境へ移植する)場合は、
> [Sim3dView設計書](../docs/DETAILED_DESIGN.md)の9節(実装仕様)と10節(再実装ガイド)を参照してください。

## 依存関係への追加

モノレポ内(このリポジトリの`sample/sim_frontend`のように)ならpath依存で:

```toml
[dependencies]
sim3dview = { path = "../../sim3dview" }
```

別リポジトリから使う場合はgit依存で:

```toml
[dependencies]
sim3dview = { git = "https://example.com/your-fork/Sim3dView.git" }
```

呼び出し側のCargo.tomlにも`leptos = { version = "0.8", features = ["csr"] }`が必要です
(Leptosコンポーネントを`view!{}`マクロで使うため)。

## サーバーに必要なもの(データ契約)

このライブラリはHTTPで配信される地形データを前提とします。地形は**1度x1度のタイル単位で、タイルごとに
複数の解像度レベル**を持ちます(地形LOD。全タイルを6x6のチャンクに分けて常駐し(最も粗くても約620m/セル。
起動直後だけタイル全体1枚の粗いメッシュ)、カメラに近いチャンクほど細かいレベル(最細は元データの30m)を
取得して描画します。`terrain::lod`・[DETAILED_DESIGN.md](../docs/DETAILED_DESIGN.md) 6.10節)。呼び出し側が用意するサーバーは、任意のベースURL
(例: `http://localhost:9001/terrain`)の下に以下を返す必要があります(`terrain::fetch`・`terrain::loader`のソースのコメント参照。内部モジュールなので`pub(crate)`)。

- `{base_url}/metadata.json`(`Content-Type: application/json`):

  ```json
  {
    "tile_levels": [60, 180, 600, 1800, 3600],
    "chunks_per_tile": 6,
    "elevation_min": -330.0,
    "elevation_max": 3937.0,
    "geodetic_bounds": { "min_lat": 20.0, "max_lat": 50.0, "min_lon": 120.0, "max_lon": 150.0 },
    "ellipsoid": { "a_m": 6378137.0, "inv_f": 298.257222101 },
    "has_texture": false,
    "default_origin": { "lat_deg": 35.355556, "lon_deg": 138.859722 }
  }
  ```

  `tile_levels`はレベルごとの1度タイル1辺のセル数(先頭がレベル0=最粗)。レベル1以上は
  `chunks_per_tile`で割り切れること。

- `{base_url}/tile_index.json`: 存在するタイルの一覧
  `{"tiles": [{"lat": 35, "lon": 138, "elevation_min": 0, "elevation_max": 3776}, ...]}`
  (`lat`/`lon`はタイル南西角の整数度。陸のないタイルは含めない)。
- `{base_url}/base.bin`(`application/octet-stream`): レベル0(タイル全体で1枚)を全タイル分、
  `tile_index.json`の順に連結したもの。
- `{base_url}/tiles/L{k}/N035E138.bin`(k=1以上): レベルkのタイル別ファイル。1度タイルを
  `chunks_per_tile`x`chunks_per_tile`のチャンクに分け、チャンク(行(南→北)*分割数+列(西→東)の順)ごとの
  グリッドを**固定サイズのレコード**として連結したもの。大きいファイル(最細で約26MB)は、フロントが
  **HTTP Range**(`bytes=a-b`、単一範囲)でチャンク1個分だけ取得するので、サーバーはRangeに対応してください
  (未対応でも全体を返せば動きますが、毎回全体を転送することになります)。

各グリッドは`(N+1)x(N+1)`ノード(`N`は一辺のセル数。レベル0はタイル全体、レベル1以上はチャンク)の
int16(標高メートル)、リトルエンディアン、row-major、**行は南→北・列は西→東**。データなし(海)は
`-32768`。隣のチャンクとは縁のノードを共有します。

このリポジトリの`tools/geotiff_preprocess`(C++ + GDAL)は、ALOS DSM GeoTIFFタイルからこれらを
生成する前処理ツールで、ライブラリの一部として提供しています。同じ形式さえ満たせば
サーバーの実装言語・データソースは問いません。

## 最小構成の使用例

```rust
use leptos::prelude::*;
use sim3dview::terrain::markers::RadarMarkersState;
use sim3dview::terrain::origin::OriginState;
use sim3dview::terrain::store::TerrainStore;
use sim3dview::ui::terrain_view::TerrainView;
use sim3dview::terrain::camera::CameraPreset;

#[component]
pub fn App() -> impl IntoView {
    // 地形データ(1回だけフェッチして全パネルで共有)。
    provide_context(TerrainStore::new("http://localhost:9001/terrain"));
    // 現在の原点。あなたのアプリが自分のプロトコルから受け取った値をここへ反映する
    // (下記「原点をサーバーと同期する」参照)。未設定ならmetadata.jsonのdefault_originを使う。
    let origin_state = OriginState(RwSignal::new(None));
    provide_context(origin_state);
    // レーダー観測点(見通し範囲)の一覧・選択・覆域高度。
    provide_context(RadarMarkersState::new());

    view! { <TerrainView preset=CameraPreset::Overview/> }
}
```

`index.html`(Trunk使用時)で、このライブラリのCSSを追加で読み込みます(相対パスは
あなたのアプリのCargo.tomlからの相対位置に合わせて調整してください)。

```html
<link data-trunk rel="css" href="../../sim3dview/style/sim3dview.css" />
<link data-trunk rel="css" href="style/app.css" /> <!-- あなたのアプリ独自のCSS -->
```

Trunkがこの相対パス参照に対応していない構成(例: gitサブモジュール越しの参照や、別ホストの
crates.io公開クレートとして使う場合)では、`sim3dview/style/sim3dview.css`を自分のスタイル
ディレクトリへコピーするか、自前のCSSから`@import`してください。

### テーマ(CSSカスタムプロパティ)

`sim3dview.css`は以下のCSSカスタムプロパティを参照します。あなたのアプリの`:root`で
定義してください(未定義でも動作しますが配色が付きません)。

| 変数 | 用途 | 参考値 |
|---|---|---|
| `--bg-panel` | パネル背景色 | `#181818` |
| `--fg` | 通常の文字色 | `#eee` |
| `--border` | 枠線色 | `#3a3a3a` |
| `--accent` | 強調色(選択中・フォーカス等) | `#9cf` |

## 状態をまとめて初期化する

機能を組み合わせる場合は`ViewerState`で状態の生成とcontext登録をまとめられます。
Leptosコンポーネント内で、子ビューを生成する前に1回呼んでください。
従来の個別登録も利用できます。同じOwnerで同じ状態を二重登録しないでください。

```rust
use sim3dview::viewer::ViewerState;

let viewer = ViewerState::new("http://localhost:9001/terrain")
    .persist_drawings("my_app.drawings") // 任意。省略すると保存・復元しない
    .provide();

// 受信データはアプリが既存の公開ハンドルへ反映する。
// viewer.origin.0.set(Some(origin));
// viewer.tracks.set(tracks);
// viewer.models.set_source(kind, source);
```

生成するのは地形ストア・原点・観測点・中心点移動・陰影・キャプチャ・作図・作図ツール・航跡・モデルの
10個の状態です。`new`自体は通信を開始しません。保存キーはアプリが選び、`persist_drawings`は生成直後に1回だけ呼びます。
原点クリックのコールバック、メニュー項目、ダイアログの開閉と配置はアプリ側で設定します。

## パネルを左右に分割する

```rust
use sim3dview::ui::split_pane::SplitPane;

let initial_fraction = 2.0 / 3.0;
view! {
    <SplitPane
        initial_fraction=initial_fraction
        min_first=320.0
        min_second=260.0
        first=move || view! { <div>"地図など"</div> }
        second=move || view! { <div>"情報パネルなど"</div> }
    />
}
```

`first`と`second`は常時マウントされる左右のビューです。高さは親の100%を使うため親の高さを確保してください。
`second_visible`には`Signal<bool>`を指定できます(省略時は常に表示)。`false`では右区画と仕切りを隠し、左区画だけで全幅を使います。内容と分割比率は保持し、最小幅は`min_first`だけになります。
最小幅は正の有限値(CSS px、既定各160)、左側の初期比率は有限値(既定0.5、0.001〜0.999へ制限)です。
狭い画面では最小幅の合計+6pxを維持するため、親で`overflow-x: auto`を指定します。
`.sim3d-split-pane`・`.sim3d-split-content`・`.sim3d-split-handle`のCSSはライブラリに含まれます。

## 距離と方位を計算する

```rust
use sim3dview::terrain::measurement::distance_and_bearing;
let (distance_m, bearing_deg) = distance_and_bearing(35.0, 139.0, 35.5, 139.0);
```

引数は度単位の緯度経度、戻り値はWGS84楕円体上の最短測地距離(m)と北から時計回りの初期方位(0以上360未満の度)です。
`geographiclib-rs`によるカーニー法を使い、対蹠点付近も計算できます。有限値・緯度-90〜90度を前提とします。
同一点や複数の最短測地線がある場合の方位はGeographicLibの規約値で、一意ではありません。
日本語方位名や表示単位の整形はアプリ側で行います。従来の球面近似から変更したため、計算結果は変わります。

### 複数地点を通る経路を測定する

`measure_route`で経路計画や航跡分析に使う区間距離・累積距離・初期方位をまとめて取得できます。
地形データやLeptosのcontextは不要です。

```rust
use sim3dview::terrain::measurement::measure_route;

let route = measure_route(&[(35.0, 139.0), (35.5, 139.0), (35.5, 139.5)])
    .expect("有効な緯度経度");
let total_m = route.total_distance_m;
for segment in &route.segments {
    let length_m = segment.distance_m;
    let distance_from_start_m = segment.cumulative_distance_m;
    let bearing_deg = segment.initial_bearing_deg; // Option<f64>
}
```

緯度[-90,90]・経度[-180,180]の有限値を受け付け、不正な点があると`InvalidRoutePoint.index`
(0始まり)を返します。空または1点の経路は総距離0で、区間はありません。
補助球上の弧長が0・πから1e-7 rad以内では、保守的に方位を`None`にします(距離は計算します)。日付変更線にも対応します。
閉路にする場合は始点を末尾にも指定してください。距離はカーニー法によるWGS84楕円体上の測地距離で、
高度差や地形に沿った距離は含みません。

### 一定高度を飛行する経路を測定する

```rust
use sim3dview::terrain::measurement::{measure_route, measure_route_at_height};

let points = [(35.0, 139.0), (35.5, 139.0), (35.5, 139.5)];
let surface = measure_route(&points).expect("有効な経路");
let flight = measure_route_at_height(&points, 10_000.0).expect("有効な経路と楕円体高");
let extra_distance_m = flight.total_distance_m - surface.total_distance_m;
```

地表のカーニー法による経路の真上を、地球の丸みに沿って一定高度で飛ぶ曲線の長さを数値積分します。
区間距離・累積距離・総距離・初期方位は飛行経路の値になり、高度0では`measure_route`と一致します。
高度面上の最短経路を再探索する機能や、2点間の空間直線距離の計算ではありません。

高度は**WGS84楕円体高(m)**で、対応範囲は0〜1,000,000mの有限値です。
海抜高度・対地高度・気圧高度を直接渡さないでください。海抜高度はジオイド高を加えて楕円体高に
変換しますが、ジオイド高が地点ごとに変わるため、一定海抜高度の経路とは厳密には一致しません。
既存の`Altitude::Msl`も別の高度基準です。地形・障害物との衝突や上昇下降は計算しません。
不正な高度は`FlightMeasurementError::InvalidHeight`、不正な座標は`InvalidPoint`を返します。

## 原点をサーバーと同期する

`OriginState`(`terrain::origin::OriginState`)はこのライブラリが通信プロトコルを知らずに
済むよう、`RwSignal<Option<terrain::origin::Origin>>`を包んだだけの薄い型です。あなたのアプリが
自分のプロトコルから受け取った緯度経度を、Effectでこのシグナルへミラーしてください
(`sample/sim_frontend/src/app.rs`に実例があります):

```rust
Effect::new(move |_| {
    if let Some(o) = my_protocol_signals.origin.get() {
        origin_state.0.set(Some(sim3dview::terrain::origin::Origin { lat_deg: o.lat_deg, lon_deg: o.lon_deg }));
    }
});
```

`ui::origin_dialog::OriginDialog`(原点入力フォームのフローティングパネル)を使う場合、
「設定」ボタンが押されたときの送信方法もあなたのアプリに委ねられています
(`on_submit: UnsyncCallback<(f64, f64)>`。`Send`不要なので、`Rc`などを持つ接続をそのまま捕捉できる)。
入力値の範囲チェックには、`TerrainView`が使うのと同じ`TerrainStore`(context)の地形データの範囲を使います。

```rust
use sim3dview::ui::origin_dialog::{OriginDialog, OriginDialogState};

provide_context(OriginDialogState(RwSignal::new(false))); // 開閉状態

view! {
    <OriginDialog
        on_submit=UnsyncCallback::new(move |(lat, lon)| {
            // ここであなたのプロトコルで実際に送信する。
        })
    />
}
```

地図を直接クリックして原点を決めたい場合は、`terrain::origin_pick::OriginPickState`を
`provide_context`し、メニュー等から`active`を`true`にします(未提供なら機能なし)。
`TerrainView`は次の左クリック(ドラッグではない単発クリック)の緯度経度を`on_pick`へ渡し、
`active`を自動で`false`に戻します。送信方法はここでもあなたのアプリに委ねられています。

```rust
use sim3dview::terrain::origin_pick::OriginPickState;

let pick = OriginPickState::new(UnsyncCallback::new(move |(lat, lon)| {
    // ここであなたのプロトコルで原点変更を送信する。
}));
provide_context(pick);
// メニュー項目などから: pick.active.set(true);
```

## レーダー観測点(見通し範囲・覆域)

`terrain::markers::RadarMarkersState`が観測点一覧・選択状態・(2D表示モード時の)覆域高度を
保持します。`ui::terrain_view::TerrainView`は自身のcanvas上の右クリックで観測点を追加し(右クリックメニューを使う場合は、
その項目として追加します。下の「右クリックメニュー」参照)、
`ui::los_view::LosView`はその一覧の選択・編集・削除UIと、選択中観測点の2D極座標見通し図を
提供します。

```rust
use sim3dview::ui::los_view::LosView;

view! { <LosView/> } // RadarMarkersState・TerrainStore contextが必要
```

`ui::coverage_altitude_dialog::CoverageAltitudeDialog`は、`TerrainView`の2D表示モードで
選択中観測点の探知可能領域を表示する対象の海抜高度を編集するフローティングパネルです
(`RadarMarkersState`のみ参照、通信は一切行いません)。

**複数の覆域の同時表示**: 覆域(3Dドーム・2D領域)は、既定では選択中の観測点だけです。`RadarMarkersState::show_all_coverage`を`true`にすると
(`LosView`の「すべての観測点の覆域を同時に表示」チェックボックスと同じ)、**すべての観測点の覆域を同時に**出します。観測点ごとに色が違います(`coverage_colors(id)`)。

**断面図の中心**: `ui::cross_section_view::CrossSectionView`は、画面内のコンボボックスで選んだ航跡(`TracksState`を`provide_context`していれば、その一覧から選べます。
地図上のシンボルクリックで変わる`TracksState::selected`とは独立したローカルな選択です)の位置を中心に、方位角の直線に沿った断面を出します。何も選んでいなければ基準位置
(`OriginState`)が中心です。片側の長さを選べ、「進行方向」ボタンで方位角を選んだ航跡の進行方向に合わせられます。

## スクリーンショット・画面録画

`terrain::capture::CaptureState`を`provide_context`すると、`TerrainView`が自身のcanvasの
スクリーンショット(PNG)保存・画面録画(WebM)を行えるようになります(未提供でも動作しますが、
その場合は何もできません)。ボタンをどこに置くかはこのライブラリの関知しないアプリ固有のUIなので、
呼び出し側が置いてください(`sample/sim_frontend`ではVABパネルに置いています)。

```rust
use sim3dview::terrain::capture::CaptureState;

provide_context(CaptureState::new());
```

```rust
let capture = use_context::<CaptureState>().expect("CaptureState context not found");

view! {
    <button on:click=move |_| capture.request_screenshot()>"スクリーンショット"</button>
    <button on:click=move |_| capture.toggle_recording()>
        {move || if capture.is_recording.get() { "録画停止" } else { "録画開始" }}
    </button>
}
```

- `request_screenshot()`: canvasの現在の内容を`sim3dview_YYYYMMDD_HHMMSS.png`としてダウンロードします。
- `toggle_recording()`: 呼ぶたびに録画の開始/停止を切り替えます。停止すると
  `sim3dview_YYYYMMDD_HHMMSS.webm`としてダウンロードされます(`is_recording`で実際に録画中かどうかを
  見られます。ブラウザがMediaRecorder等に未対応で開始に失敗した場合は自動的にfalseへ戻ります)。
- サーバーへは何も送らない、完全にブラウザ内で完結する機能です(`HTMLCanvasElement.captureStream`+
  `MediaRecorder`)。3D地形・図形・航跡シンボルはcanvas上の描画のためどちらにも写りますが、
  航跡ラベル等のHTML要素の重ね合わせ(`ui::terrain_view`のDOMオーバーレイ)は対象外です。

## 作図(図形・線)

`terrain::drawing::DrawingState`を`provide_context`し、`add`/`update`/`remove`/`clear`で図形を出し入れします
(`TerrainView`が一覧の変化に追従して描き直します。未提供なら作図なしで動作します)。

- **図形**(`Shape`): 2D=`Circle`・`Rect`(回転可)・`Polygon`(凹も可)・`Sector`(扇形)、3D=`Sphere`・`Cuboid`・`Cylinder`・`Cone`、
  線=`Polyline`。回転角・方位は時計回りで0度が上(北)。
- **見た目**(`Style`): `fill`(塗り)・`stroke`(輪郭線・線の色)・`stroke_width_px`(太さ、画面のピクセル)。色は`Color`(RGBA、
  アルファ<1で半透明)。`None`にすると塗りなし/枠なし。3D図形の`stroke`は稜線(ワイヤーフレーム)。
- **位置の置き方**(`Position`): 絶対座標に固定するか、カメラに固定するかを、図形を置く座標の種類で選びます
  (1つの図形の中では同じ種類にそろえる)。

| 種類 | 座標 | 動き | 置ける図形 |
|---|---|---|---|
| `Position::world(lat, lon, altitude)` | 緯度経度+高度 | 絶対座標に固定。地形と一緒に動き、山の陰に隠れる | すべて |
| `Position::view(right, up, forward)` | カメラからの相対(m) | カメラに追従。遠近法つきで地形の手前に浮かぶ | すべて |
| `Position::screen(corner, x, y)` | 画面の角からのpx | 画面に固定(HUD)。地形の手前に追加順で重なる | 2D図形・線 |

- 高度(`Altitude`): `Msl(h)`は海抜で、2D図形は**その高さの水平な面**。`AboveGround(o)`は地表からで、2D図形・線は**地形の起伏に沿って
  貼り付く**(3D図形は真下の地表を基準に置くだけ)。
- 大きさの単位は`world`/`view`ではメートル、`screen`ではピクセル。3D図形の位置は、球は中心・それ以外は底面の中心。

```rust
use sim3dview::terrain::drawing::*;

let drawings = DrawingState::new();
provide_context(drawings);

// 地形に貼り付く半透明の緑の円(輪郭線つき)
drawings.add(
    Shape::Circle { center: Position::world(35.36, 138.73, Altitude::AboveGround(0.0)), radius: 5_000.0 },
    Style::fill_and_stroke(Color::rgba(0.2, 0.9, 0.3, 0.35), Color::rgb(0.6, 1.0, 0.6), 3.0),
);
// 地表から200mの高さで地形に沿う折れ線(太さ4px)
drawings.add(
    Shape::Polyline { points: vec![
        Position::world(35.0, 138.0, Altitude::AboveGround(200.0)),
        Position::world(35.4, 138.7, Altitude::AboveGround(200.0)),
    ] },
    Style::stroked(Color::rgb(1.0, 0.6, 0.1), 4.0),
);
// 画面の左上(20,20)から160x80の半透明の枠(カメラを動かしても動かない)
let id = drawings.add(
    Shape::Rect { center: Position::screen(Corner::TopLeft, 100.0, 60.0), width: 160.0, height: 80.0, rotation_deg: 0.0 },
    Style::fill_and_stroke(Color::rgba(0.0, 0.0, 0.0, 0.5), Color::WHITE, 2.0),
);
// 後から書き換え・削除
drawings.update(id, |d| d.style.fill = Some(Color::rgba(1.0, 0.0, 0.0, 0.5)));
drawings.remove(id);
```

### UIから図形を作る(図形の対話作成)

ユーザーが地図をクリックして図形を作り、数値で編集できるようにするには、`terrain::draw_tool::DrawToolState`を`provide_context`し、
`ui::drawing_editor::DrawingEditor`をどこかに置きます(`DrawingState`が先に必要。`TerrainView`が地図のクリックを受けます)。
円・矩形・多角形・扇形・折れ線・球・直方体・円柱・円錐を、ツールを選んで地図をクリックして置きます(点の置き方は画面に案内が出ます。
多角形・折れ線はダブルクリック/Enterで確定、右クリック/Backspaceで1つ戻す、Escで終了)。作った図形は一覧から選ぶと地図上で黄色く縁取られ、
位置・大きさ・高度・色などを数値で編集できます。

```rust
use sim3dview::terrain::draw_tool::DrawToolState;
use sim3dview::terrain::drawing::DrawingState;
use sim3dview::ui::drawing_editor::DrawingEditor;

let drawings = DrawingState::new();
provide_context(drawings);
// persist(key)を付けると、作った図形をlocalStorageへ保存して次回の起動時に復元する(付けなければ保存しない)。
provide_context(DrawToolState::new(drawings).persist("my_app.user_drawings"));

view! { <DrawingEditor/> } // 地図(TerrainView)をクリックできるよう、モーダルではなくパネルやタブの中に置く
```

このエディタで作った図形だけが一覧・保存の対象です(アプリが`drawings.add`で足した図形は別扱いで、`DrawToolState`の操作では消えません)。

`sample/sim_frontend`の「表示」→「作図デモ」(`components/drawing_demo.rs`)に、全種類の図形を絶対座標・視点空間・画面座標で
置く実例があります(表示メニューの「作図...」で開く移動可能なウインドウが、上の対話作成の実例です)。

## 航跡(航空機・艦船・車両等の現在位置とシンボル)

シミュレーションなどから受け取った位置を、向きつきのシンボル・ラベル・航跡(軌跡)・高度線で表示します。
`terrain::tracks::TracksState`を`provide_context`し、**受信のたびに全トラックの最新状態を`set`する**だけです
(通信プロトコルはこのライブラリの外。前回に無いIDは新規、今回に無いIDは航跡ごと消えます)。`TerrainView`が一覧の変化に追従して
描き直します(未提供なら航跡表示なしで動作します)。

- **`Track`**: `id`(同じ実体は常に同じID)・`kind`(`SymbolKind`: 固定翼機・ヘリ・艦船・地上車両・ミサイル・不明)・
  `affiliation`(`Affiliation`: 友軍=青・敵=赤・中立=緑・不明=黄)・`label`・緯度経度・`altitude`(`Altitude::Msl`=海抜 /
  `AboveGround`=地表から。地形の高さを持たないサーバーの車両などは後者)・`heading_deg`(北から時計回り)・`speed_mps`・
  `pitch_deg`(機首上げが正)・`roll_deg`(右翼が下がるのが正。3Dモデルの向きだけに使う。使わなければ0)。
- **シンボル**は画面サイズ固定で、進行方向が画面上の実際の向きを指すよう回ります(3Dでカメラを回しても、2Dの地図でも)。
- **ラベル**は名前+「高度 速度」。**航跡**は過去の位置の折れ線、**高度線**は地表へ下ろす細い線(3Dのみ)。
  `TracksState`の`show_labels`/`show_trails`/`show_altitude_lines`(`RwSignal<bool>`、既定ON)で切り替えます。
- ラベルは`TerrainView`が重ねるHTML要素です(`sim3dview.css`の`.track-label`)。
- **クリックで選択**: 地図上のシンボルをクリックすると`TracksState::selected`にそのIDが入り、シンボルに白い輪が付きます(何もない所をクリックすると解除)。
  詳細の表示はアプリ側で、`tracks.selected_track()`(最新の`Track`。位置の更新にも追従)を読んで作ります。`tracks.select(Some(id))`で
  コードから選択もできます。`TabbedPanel`に`active`(`RwSignal<usize>`)を渡すと、選択されたら詳細のタブへ切り替える、ということもできます。

```rust
use sim3dview::terrain::drawing::Altitude;
use sim3dview::terrain::tracks::{Affiliation, SymbolKind, Track, TracksState};

let tracks = TracksState::new();
provide_context(tracks);

// 自分のシミュレーション結果(や受信したメッセージ)から、毎回「全トラックの最新状態」を作って渡す。
tracks.set(vec![
    Track {
        id: 1,
        kind: SymbolKind::Aircraft,
        affiliation: Affiliation::Friendly,
        label: "AC101".into(),
        lat_deg: 35.5,
        lon_deg: 138.9,
        altitude: Altitude::Msl(4000.0),
        heading_deg: 90.0,
        speed_mps: 200.0,
        pitch_deg: 0.0,
        roll_deg: 0.0,
    },
    Track {
        id: 2,
        kind: SymbolKind::Vehicle,
        affiliation: Affiliation::Neutral,
        label: "TRK1".into(),
        lat_deg: 35.3,
        lon_deg: 139.0,
        altitude: Altitude::AboveGround(0.0), // 地形の高さは不要(ライブラリが地表に置く)
        heading_deg: 180.0,
        speed_mps: 15.0,
        pitch_deg: 0.0,
        roll_deg: 0.0,
    },
]);
```

```rust
// 選択された航跡の詳細を出す(sample/sim_frontend/src/components/track_detail.rs が実例)
move || match tracks.selected_track() {
    None => view! { <p>"シンボルをクリックしてください"</p> }.into_any(),
    Some(t) => view! { <p>{t.label} " " {t.kind.label()} " " {t.affiliation.label()}</p> }.into_any(),
}
```

サンプルアプリ(`sample/`)に、サーバー(C++)からのデータ受信を含む一通りの実例があります。`sample/sim_server`の
`Simulation::make_demo_scenario`が7つのトラックを周回させて`TrackList`(msg_type 0x07)として配信し、`sample/sim_frontend`の
`track_bridge.rs`が受信した値を上の`Track`へ変換して`TracksState::set`へ渡します(受信〜表示までの橋渡しがこの数十行だけ)。
左パネルの「開始」でシミュレーションを進めると動き、表示メニューの「航跡ラベル/航跡(軌跡)/高度線」で表示を切り替えられます。

## 3Dモデル(glTF)で航跡を描く(任意)

航跡のシンボルの代わりに、glTF 2.0のGLBを3Dモデルとして地図に置けます。UnityやBlenderなどで作ったモデルを、**GLB形式で書き出して**使います
(FBX・.prefabは対応しないので、Blenderなどでglbへ変換してください)。`terrain::models::ModelsState`を`provide_context`し、**種別ごとに使うモデルのURLを登録する**だけです
(登録しない種別・読み込み中・読み込みに失敗したものは、今までどおりシンボルで描きます。`ModelsState`を提供しなければモデルなしで動作します)。

- **表示方式**(`ModelsState::mode`。`ui::model_settings_dialog::ModelSettingsDialog`で利用者が切り替えられます):
  - `SwitchToSymbol`(既定): カメラからの距離が`switch_distance_m`(既定1,500m)以内はモデル(実寸)、それより遠いとシンボル。
  - `MinScreenSize`: 常にモデル。画面での大きさが`min_screen_px`(既定32px)に満たないモデルは、その大きさになるよう実寸より大きくします。
  - `Off`: シンボルのみ。
- **向き**: `Track`のヘディング・ピッチ・ロールで決まります(地球の丸みで傾く「上」にも沿います)。
- **モデルの作り方**: 単位はメートル(cmなどなら`ModelSource::scale`で直す)、glTFの規約(+Y上・+Z前)。原点は基準点(航空機・ヘリは中心、艦船は水線、車両は接地面が便利)。
  前が+Zからずれていれば`ModelSource::yaw_offset_deg`で直します。
- **対応する内容**: 三角形メッシュの形・法線・頂点色・マテリアルの基本色。**テクスチャ・アニメーション・スキン・外部ファイル参照は対応しません**(GLBの1ファイルにしてください)。
- モデルで描いているトラックも、航跡(軌跡)・高度線・ラベル・選択は今までどおりです。

```rust
use sim3dview::terrain::models::{ModelSource, ModelsState};
use sim3dview::terrain::tracks::SymbolKind;
use sim3dview::ui::model_settings_dialog::{ModelSettingsDialog, ModelSettingsDialogState};

let models = ModelsState::new();
models.set_source(SymbolKind::Aircraft, ModelSource::new("models/aircraft.glb")); // URLはページからの相対でも絶対でもよい
models.set_source(SymbolKind::Ship, ModelSource { scale: 0.01, ..ModelSource::new("models/ship_cm.glb") }); // cm単位のモデル
provide_context(models);

// 表示方式・距離を利用者が変えるウインドウ(任意。メニューから`ModelSettingsDialogState.0.set(true)`で開く)
provide_context(ModelSettingsDialogState(RwSignal::new(false)));
// ...ビューの中に <ModelSettingsDialog/> を置く
```

サンプルアプリでは、`app.rs`が5種類のモデルを登録し、表示メニューの「3Dモデル...」が設定ウインドウです。モデルは`scripts/gen_sample_models.py`が生成する簡易なもの
(`sample/sim_frontend/assets/models/`。`index.html`のcopy-dirで`models/`として配信)で、`python scripts/gen_sample_models.py`で作り直せます。
`sample/sim_server`のデモシナリオは、`TrackList`のピッチ・ロールに、旋回のバンク・上昇降下・波の揺れを入れています。
設計は[DETAILED_DESIGN.md](../docs/DETAILED_DESIGN.md) 6.13節。カメラの地表からの最小距離は`terrain::camera::MIN_EYE_CLEARANCE_M`を参照してください。

## 右クリックメニュー

`ui::context_menu::ContextMenu`は、項目を使う側が決める汎用の右クリックメニューです(`FloatingPanel`と同じ考え方)。`ContextMenuState`を`provide_context`し、
`<ContextMenu/>`をどこかに1つ置きます。地図の右クリックにつなぐには、さらに`ui::context_menu::MapMenuState`に「右クリックした場所から項目を作る関数」を渡して
`provide_context`します。`TerrainView`は右クリックで`MapMenuTarget { position: 地表の(緯度, 経度), track: 航跡のシンボル }`を求め、関数が返した項目でメニューを出します
(両方のcontextが無ければ、従来どおり右クリックでレーダー観測点を追加します)。

```rust
use sim3dview::ui::context_menu::{ContextMenu, ContextMenuState, MenuItem};
use sim3dview::ui::util::copy_to_clipboard;
use sim3dview::ui::context_menu::MapMenuState;

provide_context(ContextMenuState::new());
provide_context(MapMenuState::new(move |target| {
    let mut items = Vec::new();
    if let Some((lat, lon)) = target.position {
        items.push(MenuItem::label(format!("緯度 {lat:.5}°  経度 {lon:.5}°"))); // 押せない見出し
        items.push(MenuItem::action("ここにレーダー観測点を追加", move || {
            radar_markers.add(lat, lon);
        }));
        items.push(MenuItem::action("ここを中心点にする", move || recenter.request_at(lat, lon)));
        items.push(MenuItem::separator());
        items.push(MenuItem::submenu("ここに図形を作成", vec![
            MenuItem::action("円", move || draw_tool.start_at(ToolKind::Circle, lat, lon)),
            MenuItem::action("矩形", move || draw_tool.start_at(ToolKind::Rect, lat, lon)),
        ]));
        items.push(MenuItem::action("緯度経度をコピー", move || copy_to_clipboard(&format!("{lat:.6}, {lon:.6}"))));
    }
    items // 空ならメニューは出ない
}));

view! { <ContextMenu/> }
```

- 項目: `MenuItem::action`(押せる。`.enabled(false)`か`MenuItem::disabled`で無効に)・`submenu`(入れ子可)・`label`(押せない見出し)・`separator`。
  項目を選ぶとメニューを閉じてからコールバックを呼びます。メニューの外のクリック・右クリック・Escでも閉じ、画面の端では収まるようにずれます。
- 中心点の移動は`RecenterRequestState::request_at(lat, lon)`(`request()`は原点へ戻す)、図形の作成の開始は`DrawToolState::start_at(kind, lat, lon)`(その地点を1点目にして開始)。
- 作図ウインドウ(`DrawingEditor`)の図形一覧の行も、`ContextMenuState`があれば右クリックメニュー(名前変更・複製・表示切替・削除)が出ます。
- `sample/sim_frontend/src/components/map_menu.rs`に、航跡のシンボルと地表の両方に対する項目の実例があります。

## 汎用UI部品の再利用

- `ui::pointer_drag::DragTracker`: 同時に1本のpointerを追跡し、直前位置からの差分と開始位置からの
  合計移動量を返す純粋な状態管理です。`set_pointer_capture`/`release_pointer_capture`などのDOM操作と、
  移動量を何へ反映するかは呼び出し側が担当します。
- `ui::tabbed_panel::{TabbedPanel, tab}`: タブ付きパネル。VAB設定など、独自タブを持つ
  パネルを作る際にも使えます。
- `ui::floating_panel::FloatingPanel`: フローティングウインドウ。`OriginDialog`/`CoverageAltitudeDialog`が内部で
  使っています。独自のフローティングウインドウ(VAB設定パネルなど)を同じ見た目で作りたい場合に使ってください。
  既定は、半透明バックドロップが画面を覆う**モーダル**(中央に出て、背景クリックか✕で閉じる)です。

  | プロパティ | 既定 | 意味 |
  |---|---|---|
  | `modal` | `true` | `false`にするとバックドロップなしの**ウインドウ**になり、背後(地図など)を操作したまま出しておける(✕でだけ閉じる) |
  | `draggable` | `false` | `true`でタイトルバーのドラッグで動かせる。タイトルバーの一部(横80px・縦40px)が必ず画面内に残る範囲に制限され、動かした位置は閉じて開き直しても保たれる |
  | `initial_position` | `(80, 60)` | ウインドウ(`modal=false`)の初期位置(画面左上からの`(x, y)`、px)。モーダルは常に中央 |

  ```rust
  use sim3dview::ui::floating_panel::FloatingPanel;

  let open = RwSignal::new(false);
  view! {
      <FloatingPanel open=open title="独自設定">
          <p>"ここに好きな内容を置ける。"</p>
      </FloatingPanel>
  }
  ```

  地図を操作しながら使う、動かせるウインドウにする例(サンプルの作図ウインドウ。`sample/sim_frontend/src/components/drawing_window.rs`):

  ```rust
  view! {
      <FloatingPanel open=open title="作図" modal=false draggable=true initial_position=(360.0, 60.0)>
          <DrawingEditor/>
      </FloatingPanel>
  }
  ```

## フルの実装例

`sample/sim_frontend`(Rust、このライブラリを実際に使うアプリ)と`sample/sim_server`
(C++、地形データ配信+WebSocketサーバーの参照実装)を参照してください。VAB・状況パネル・
メニューバー・WebSocket/msgpackプロトコルなど、このライブラリに含まれないアプリ固有の
実装例が一通り揃っています。
