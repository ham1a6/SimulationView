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
  レーダー観測点の状態管理
- `ui`モジュール: 上記を使ったLeptosコンポーネント一式(3D/2D地形描画canvas、見通し範囲タブ、
  タブ付きパネル、汎用フローティングパネル、原点設定・覆域高度設定ダイアログ)
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

このライブラリはHTTPで配信される地形データを前提とします。呼び出し側が用意するサーバーは、
任意のベースURL(例: `http://localhost:9001/terrain`)の下に以下の2ファイルを返す必要が
あります(`terrain::loader`参照)。

- `{base_url}/metadata.json`(`Content-Type: application/json`):

  ```json
  {
    "width": 2048,
    "height": 2048,
    "elevation_min": -22.0247,
    "elevation_max": 3710.76,
    "geodetic_bounds": { "min_lat": 35.0, "max_lat": 40.0, "min_lon": 135.0, "max_lon": 140.0 },
    "ellipsoid": { "a_m": 6378137.0, "inv_f": 298.257223563 },
    "has_texture": false,
    "default_origin": { "lat_deg": 35.355556, "lon_deg": 138.859722 }
  }
  ```

- `{base_url}/heightmap.bin`(`Content-Type: application/octet-stream`): `width * height * 4`
  バイトのリトルエンディアンf32配列(row-major、南→北の行順)。NODATA/海は`NaN`。

このリポジトリの`tools/geotiff_preprocess`(C++ + GDAL)は、ALOS DSM GeoTIFFタイルからこの2ファイルを
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
済むよう、`RwSignal<Option<terrain::mesh::Origin>>`を包んだだけの薄い型です。あなたのアプリが
自分のプロトコルから受け取った緯度経度を、Effectでこのシグナルへミラーしてください
(`sample/sim_frontend/src/app.rs`に実例があります):

```rust
Effect::new(move |_| {
    if let Some(o) = my_protocol_signals.origin.get() {
        origin_state.0.set(Some(sim3dview::terrain::mesh::Origin { lat_deg: o.lat_deg, lon_deg: o.lon_deg }));
    }
});
```

`ui::origin_dialog::OriginDialog`(原点入力フォームのフローティングパネル)を使う場合、
「設定」ボタンが押されたときの送信方法もあなたのアプリに委ねられています
(`on_submit: Callback<(f64, f64)>`)。

```rust
use sim3dview::ui::origin_dialog::{OriginDialog, OriginDialogState};

provide_context(OriginDialogState(RwSignal::new(false))); // 開閉状態

view! {
    <OriginDialog
        base_url="http://localhost:9001/terrain"
        on_submit=Callback::new(move |(lat, lon)| {
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

let pick = OriginPickState::new(Callback::new(move |(lat, lon)| {
    // ここであなたのプロトコルで原点変更を送信する。
}));
provide_context(pick);
// メニュー項目などから: pick.active.set(true);
```

## レーダー観測点(見通し範囲・覆域)

`terrain::markers::RadarMarkersState`が観測点一覧・選択状態・(2D表示モード時の)覆域高度を
保持します。`ui::terrain_view::TerrainView`は自身のcanvas上の右クリックで観測点を追加し、
`ui::los_view::LosView`はその一覧の選択・編集・削除UIと、選択中観測点の2D極座標見通し図を
提供します。

```rust
use sim3dview::ui::los_view::LosView;

view! { <LosView/> } // RadarMarkersState・TerrainStore contextが必要
```

`ui::coverage_altitude_dialog::CoverageAltitudeDialog`は、`TerrainView`の2D表示モードで
選択中観測点の探知可能領域を表示する対象の海抜高度を編集するフローティングパネルです
(`RadarMarkersState`のみ参照、通信は一切行いません)。

## 汎用UI部品の再利用

- `ui::tabbed_panel::{TabbedPanel, tab}`: タブ付きパネル。VAB設定など、独自タブを持つ
  パネルを作る際にも使えます。
- `ui::floating_panel::FloatingPanel`: `OriginDialog`/`CoverageAltitudeDialog`が内部で
  使っている、半透明バックドロップ+中央パネルのフローティングウインドウ。独自のフローティング
  ウインドウ(VAB設定パネルなど)を同じ見た目で作りたい場合に使ってください。

  ```rust
  use sim3dview::ui::floating_panel::FloatingPanel;

  let open = RwSignal::new(false);
  view! {
      <FloatingPanel open=open title="独自設定">
          <p>"ここに好きな内容を置ける。"</p>
      </FloatingPanel>
  }
  ```

## フルの実装例

`sample/sim_frontend`(Rust、このライブラリを実際に使うアプリ)と`sample/sim_server`
(C++、地形データ配信+WebSocketサーバーの参照実装)を参照してください。VAB・状況パネル・
メニューバー・WebSocket/msgpackプロトコルなど、このライブラリに含まれないアプリ固有の
実装例が一通り揃っています。
