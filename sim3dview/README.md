# sim3dview

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
  作図エディタ、右クリックメニュー)
- `style/sim3dview.css`: 上記コンポーネントのスタイル

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
複数の解像度レベル**を持ちます(地形LOD。離れたタイルは全体で1枚の粗いメッシュ、カメラに近いタイルは
6x6のチャンクに分けて、近いチャンクほど細かいレベル(最細は元データの30m)を取得して描画します。
`terrain::lod`・DETAILED_DESIGN.md 6.10節)。呼び出し側が用意するサーバーは、任意のベースURL
(例: `http://localhost:9001/terrain`)の下に以下を返す必要があります(`terrain::loader`参照)。

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

```rust
use sim3dview::ui::origin_dialog::{OriginDialog, OriginDialogState};

provide_context(OriginDialogState(RwSignal::new(false))); // 開閉状態

view! {
    <OriginDialog
        base_url="http://localhost:9001/terrain"
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
  `AboveGround`=地表から。地形の高さを持たないサーバーの車両などは後者)・`heading_deg`(北から時計回り)・`speed_mps`。
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

## 右クリックメニュー

`ui::context_menu::ContextMenu`は、項目を使う側が決める汎用の右クリックメニューです(`FloatingPanel`と同じ考え方)。`ContextMenuState`を`provide_context`し、
`<ContextMenu/>`をどこかに1つ置きます。地図の右クリックにつなぐには、さらに`ui::terrain_view::MapMenuState`に「右クリックした場所から項目を作る関数」を渡して
`provide_context`します。`TerrainView`は右クリックで`MapMenuTarget { position: 地表の(緯度, 経度), track: 航跡のシンボル }`を求め、関数が返した項目でメニューを出します
(両方のcontextが無ければ、従来どおり右クリックでレーダー観測点を追加します)。

```rust
use sim3dview::ui::context_menu::{copy_to_clipboard, ContextMenu, ContextMenuState, MenuItem};
use sim3dview::ui::terrain_view::MapMenuState;

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
