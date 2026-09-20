# Sim3dView 基本設計書

## 0. 本書の位置づけ

本書は、システムの背景・全体構成・技術スタック・確定した設計方針をまとめた**基本設計書**である。
(旧指示書・旧設計書の内容は、本書と詳細設計書に漏れなく引き継いだ上で、両旧文書は削除済み。)

設計書は基本設計(本書)→詳細設計([DETAILED_DESIGN.md](DETAILED_DESIGN.md): 地形データ・座標系・プロトコル・C++・ライブラリの設計方針・UML図)→
ライブラリ実装仕様(詳細設計書9節: 定数・アルゴリズム・バイト配置・テストの要点を実装コードから起こした仕様)の3層で、全体の索引は[README.md](README.md)。
新しいセッション・新しい開発者が本プロジェクトに参加する際は、まず本書を読み、詳細が必要になった箇所で詳細設計書、さらに正確な数値・手順が必要になった箇所で実装仕様を参照する。

---

## 1. 背景・目的

既存のC++シミュレータに、Rust(WASM/Leptos)製のWeb UIを追加する。シミュレータ本体のコアロジックは変更せず、
WebSocketサーバー機能を追加する形で連携させる。中央の地図は、ALOS全球数値地表モデル(GeoTIFF)から生成した
3D地形として表示する。

---

## 2. システム全体構成

### 2.1 プロセス構成

C++側とRust(WASM)側の1プロセス構成(中間サーバーなし)。C++プロセス(`sim_server`)が単体で
WebSocketサーバーとHTTP静的ファイルサーバーを兼ねる。

### 2.2 シミュレーション連携(WebSocket, 高頻度)

```
┌───────────────────────────────────────────┐
│         C++ プロセス(1プロセス完結)          │
│                                             │
│  [Simスレッド]  →(Loop::defer)→ [uWSイベントループ] │
│  Simulation::step()              WS送受信    │
│                                             │
└─────────────────┬───────────────────────────┘
                   │ WebSocket (ws://localhost:9001/sim)
                   │ バイナリフレーム(MessagePack)
┌─────────────────▼───────────────────────────┐
│      ブラウザ (Rust → WASM, Leptos)            │
│  web_sys::WebSocket → rmp_serde → Signal更新 → 再描画(wgpu) │
└─────────────────────────────────────────────┘
```

- 通信はWebSocketのバイナリフレーム、シリアライズはMessagePack
- コマンド伝達(UI→シミュレータ)も同じWebSocket経由の双方向通信
- スレッドモデルの詳細(コマンドキュー等)は詳細設計書6節を参照

### 2.3 地形データ連携(HTTP静的配信, 起動時1回)

```
[map_data/ 内のALOS DSM GeoTIFF(現在390タイル)]
    ↓
  前処理ツール(C++, GDAL使用。geotiff_preprocessという単発CLI。tools/geotiff_preprocess)
    - タイルごとに複数の解像度レベル(平均法、一辺60/180/600/1800/3600セル。最細は元データの30m。レベル1以上は6x6のチャンクに分割。再投影は不要。地形LOD)
    ↓
  出力アセット(静的ファイル。sim_server/assets/terrain/)
    - base.bin (全タイルの最粗レベルの連結、int16) / tiles/L{k}/*.bin (細かいレベルのタイル別グリッド)
    - tile_index.json (存在するタイルの一覧)
    - metadata.json (緯度経度範囲・楕円体パラメータ・レベル定義等)
    ↓
  HTTP静的配信(sim_serverが/terrain/*で配信。WebSocketの/simとは別ルート)
    ↓
  Rust(WASM)フロント: 起動時にfetch → ENU変換 → CPU側でメッシュ生成 → wgpuで描画
```

- 地形データはリアルタイム更新不要(起動時に1回読み込めば十分)
- シミュレーション状態のWebSocket高頻度チャンネルとは完全に分離する
- 原点(座標系の基準点)はUIからランタイムに入力される値であり、前処理側は原点非依存の生データを出力する
  (詳しい経緯は4節・詳細設計書4節を参照)
- **対象エリアはmap_data内の全タイル**(現在390タイル、北緯20〜50°・東経120〜150°)。単一メッシュではなく、
  1度タイル単位のLOD+近いタイルは6x6チャンクで描画し、広域を自由にパン/ズームできる(4節9番・DETAILED_DESIGN.md 6.10節)。
  外接矩形は前処理が実際のタイル構成から自動計算するので、`map_data/`を増減しても追従する

---

## 3. 技術スタック

### 3.1 C++側

| ライブラリ | 用途 | ライセンス |
|---|---|---|
| uWebSockets(+uSockets) | WebSocketサーバー・HTTP静的ファイル配信 | Apache License 2.0 |
| msgpack-cxx (msgpack-c cpp_masterブランチ) | MessagePackシリアライズ(ヘッダオンリー) | Boost Software License 1.0 |
| GDAL | GeoTIFF前処理(モザイク・ダウンサンプリング) | MIT/X11系ライセンス(ドライバ次第) |
| libuv | uSocketsのイベントループバックエンド(Windows) | MIT |
| zlib | uWebSocketsのpermessage-deflate機能が参照(圧縮自体は無効化) | zlib License |

※ msgpack-c(Boost License)は配布時にライセンス文表示が必要。GDALはビルド構成によっては個別ライセンスの
ドライバを含む場合があるため、使用するフォーマットドライバに応じて要確認。

GDAL依存は前処理ツール(`tools/geotiff_preprocess`)に限定し、常駐サーバーである`sim_server`本体は
GDALをリンクしない。

### 3.2 Rust側

| クレート | 用途 | ライセンス |
|---|---|---|
| leptos (features = ["csr"]) | リアクティブUIフレームワーク | MIT |
| wasm-bindgen / wasm-bindgen-futures | JS⇄WASMバインディング、async連携 | MIT / Apache-2.0 |
| web-sys / js-sys | Web API(WebSocket, Canvas, ResizeObserver等) | MIT / Apache-2.0 |
| rmp-serde | MessagePack(Rust実装) | MIT |
| serde / serde_json | シリアライズフレームワーク | MIT / Apache-2.0 |
| gloo-timers | タイマー(再接続バックオフ、レイアウト安定待ち) | MIT / Apache-2.0 |
| gloo-net | HTTP fetch(metadata.json/tile_index.json/base.bin/タイルグリッド取得) | MIT / Apache-2.0 |
| trunk | WASMビルド・開発サーバー | MIT / Apache-2.0 |
| wgpu | 地形メッシュのGPU描画(WebGPU) | MIT / Apache-2.0 |
| bytemuck | 頂点データのGPUバッファキャスト | MIT / Apache-2.0 |
| glam | ベクトル・行列演算(ENU変換・カメラ) | MIT / Apache-2.0 |
| earcutr | 多角形(作図)の三角形分割 | ISC |
| naga(開発時のみ) | WGSLの構文・型・レイアウトの単体テスト | MIT / Apache-2.0 |

全依存のライセンス・著作権表示は[THIRD_PARTY_NOTICE.md](../THIRD_PARTY_NOTICE.md)(`scripts/gen_third_party_notice.py`で生成)。

---

## 4. 要求仕様と確定した設計方針の一覧

旧指示書には「未確定・実装時に判断/確認が必要な項目」が12項目あり、旧設計書でのユーザーとの対話により
すべて確定した。さらにその確定に伴い実装レベルで新たに生じた5項目も別途確定している。以下は両方を統合した
最終確定事項の一覧である(本書が唯一の正とする)。

| # | 項目 | 確定内容 |
|---|---|---|
| 1 | 再接続処理 | **自動リトライする**(指数バックオフ+ジッター、無制限リトライ、タブ非表示中は一時停止) |
| 2 | マルチクライアント | **全員同一ブロードキャスト**でよい(個別状態は持たない) |
| 3 | VABの行数・列数 | 開発用ダミー`VabConfig`の初期値は**縦6行×横4列**(24ボタン。先頭行=カテゴリ選択、次の4行=中段、最終行=下段の1+4+1構成。DETAILED_DESIGN.md 7.4節) |
| 4 | VABの操作種別 | **単純クリックのみ**(長押し・トグル等は実装しない) |
| 5 | 状況パネル上部「各種情報」の表示項目 | **C++側(サーバー)から動的に設定できる**。`StatusPanelConfig`メッセージで配信 |
| 6 | 左右パネルのレスポンシブ対応 | 左パネルは固定でレスポンシブ不要。**地図(中央)と右パネルはレスポンシブ対応 + ユーザーがUIでサイズ変更可能** |
| 7 | 地形エリアの規模 | `map_data`内の**全タイル**(当初17タイル、現在390タイル)を対象にする。当初は全体を1枚のモザイクにして単一メッシュとしたが、規模の拡大でLOD方式に変えた(9番) |
| 8 | オルソ画像の有無 | RGBオルソ画像なし。パンクロ画像(STK.tif)も**使用せず、標高グラデーション着色のみ**(後から、地形の法線による陰影(ヒルシェード)を表示メニューでON/OFFできるようにした。DETAILED_DESIGN.md 6.8節) |
| 9 | メッシュ解像度 | **1度タイル単位のLOD+近いタイルは6x6チャンク**(全タイルを最粗でも約620m/セルのチャンクで常駐し、カメラに近いチャンクほど185m/62m/31m(元データの30m)のレベルを取得して差し替える。チャンクの頂点は全タイルの下限を含めて合計2500万まで)。経緯: 当初は全域を1枚の単一メッシュにして1024×1024→2048×2048へ引き上げたが、`map_data/`のタイルが増え対象域が30°四方(緯度20〜50・経度120〜150、390タイル)になると1セル約1.6kmまで粗くなり、単一メッシュのままでは頂点バッファがWebGPUの上限(既定256MB)を超えるため、LOD化した(DETAILED_DESIGN.md 2.4節・6.10節) |
| 10 | 垂直誇張の要否 | **不要**。実メートル値のまま描画する(誇張機能は実装しない) |
| 11 | 座標系のすり合わせ | 原点は**UIから緯度経度を入力**して指定。原点=入力地点の海抜0m地点。**東=X、北=Y、鉛直上向き=Z**の局所ENU座標系。**C++側もUIと全く同じ座標系・原点を用いる**。デフォルト原点は**北緯35°21'20"、東経138°51'35"**(35.355556°, 138.859722°) |
| 12 | カメラの自由視点化 | **ズーム・角度切り替え(回転)・視点切り替えを自由に操作できる**カメラとする(実装済み。3D自由視点+2D真上表示。DETAILED_DESIGN.md 6.6・6.9節) |
| 13 | `set_origin`のエラー応答形式 | サーバーは**黙って無視せずエラーを返す**(`CommandError`メッセージ) |
| 14 | 緯度経度入力のバリデーション | 地形データ範囲外の値は**そもそも入力・送信できないようにする**(UI側でブロック、サーバー側でも防御的チェック) |
| 15 | レスポンシブの具体的な方式 | **縦積みへの再レイアウトは行わない**。狭幅時は幅を比例的に縮小、収まらなければ横スクロール |
| 16 | VABの空ラベルボタンの扱い | **ボタン要素自体を描画しない**(DOM上に生成しない。位置は空白のまま保持) |
| 17 | 無制限WebSocket再接続のリソース消費 | バックオフに**ジッター**を加え、**タブが非表示の間は再接続を一時停止**する |

---

## 5. ディレクトリ構成

3D地形描画・レーダー覆域/見通し計算・レーダー観測点管理・汎用UI部品は`sim3dview`ライブラリ
(Rust crate)として切り出してあり、VAB・状況パネル・メニュー・通信プロトコルのような
アプリ固有部分とは別クレートになっている(「本swをライブラリとして使えるように整理したい」
との要望による。詳細な対応関係はDETAILED_DESIGN.md 6節冒頭の表を参照)。C++サーバー・
ライブラリを使うサンプルアプリは、どちらも「サンプルである」ことが分かるよう`sample/`配下に
まとめてある。

```
Sim3dView/
├── README.md                  # セットアップ・起動手順・環境問題の対処
├── CLAUDE.md                  # AIエージェント向けの作業方針・要点(短い索引)
├── THIRD_PARTY_NOTICE.md      # 利用しているサードパーティのライセンス・著作権表示(scripts/gen_third_party_notice.pyで生成)
├── docs/                      # 設計書一式(索引はdocs/README.md)
│   ├── README.md                # 文書体系の索引・読み順・保守ルール
│   ├── BASIC_DESIGN.md          # 基本設計書(本書)
│   ├── DETAILED_DESIGN.md       # 詳細設計書(1〜8節: データ・座標系・プロトコル・C++・ライブラリの設計方針・UML。9節: ライブラリ実装仕様)
│   ├── IMPLEMENTATION_GUIDE.md  # ドキュメントだけでライブラリを再実装するための入口(フェーズ・受け入れ基準・落とし穴)
│   └── DEVELOPMENT_HISTORY.md   # 機能ごとの実装経緯・ハマりどころの記録
├── scripts/                   # gen_third_party_notice.py(ライセンス表記の生成)
├── Cargo.toml                  # ワークスペースルート(members: sim3dview, sample/sim_frontend)
├── map_data/                  # 入力: ALOS DSM GeoTIFFタイル(現在390枚、既存・変更しない。git管理外)
│   └── ALPSMLC30_N###E###_DSM.tif ...
├── tools/                       # ライブラリの一部として提供する開発ツール(サンプルではない)
│   └── geotiff_preprocess/        # GeoTIFF→タイル別多段解像度グリッド/metadata.json前処理CLI(C++, GDAL依存はここに限定)
│       ├── CMakeLists.txt           # sim_serverとは独立したCMakeプロジェクト
│       ├── vcpkg.json               # gdal
│       └── main.cpp
├── sim3dview/                   # ライブラリ本体(Rust/Leptos/WASM)。使い方はsim3dview/README.md参照
│   ├── Cargo.toml
│   ├── README.md                 # 開発者向け使い方ドキュメント
│   ├── style/
│   │   └── sim3dview.css
│   └── src/
│       ├── lib.rs
│       ├── terrain/                # データ取得・座標変換・カメラ・wgpu描画・覆域/見通し計算
│       │   ├── mod.rs
│       │   ├── fetch.rs              # metadata/タイル索引/タイルグリッドのHTTP取得(base_urlは呼び出し側が指定)
│       │   ├── loader.rs             # 取得した地形データの保持(TerrainData)・双線形サンプリング・キャッシュ
│       │   ├── geodesy.rs            # 楕円体(Ellipsoid)・ENU変換(EnuTransform)
│       │   ├── heightmap.rs          # 標高サンプリング・地表点のENU座標(ground_at_enu等)
│       │   ├── mesh.rs                # メッシュ生成(頂点・インデックス・法線・スカート・配色)
│       │   ├── vertex.rs             # 作図・航跡・マーカー共通の頂点(DrawVertex)
│       │   ├── render_bias.rs        # 地表に貼り付くものを持ち上げる高さ(Zファイティング対策)の一覧
│       │   ├── camera.rs              # カメラ(ビュー・射影行列、2D/3D)
│       │   ├── renderer/              # wgpu描画(TerrainRenderer)。pipelines/targets/uniforms/overlay/frustum
│       │   ├── store.rs               # TerrainStore(地形データの共有キャッシュ)
│       │   ├── origin.rs              # OriginState(現在の原点、プロトコル非依存)
│       │   ├── markers.rs             # RadarMarker/RadarMarkersState・マーカー(ピン)・覆域ジオメトリ生成
│       │   ├── drawing.rs             # 作図(図形・線)のデータモデル・DrawingState(6.11節)
│       │   ├── drawing_geometry.rs    # 作図の描画用ジオメトリ生成(純粋関数)
│       │   ├── draw_tool.rs           # 図形の対話作成(ツール・作成中の点・作った図形の一覧/選択/保存)DrawToolState(6.11節)
│       │   ├── tracks.rs              # 航跡(トラック)のデータモデル・TracksState・シンボル/航跡ジオメトリ(6.12節)
│       │   ├── los.rs                 # 見通し/覆域計算
│       │   ├── profile.rs             # 断面図用の地表プロファイル
│       │   ├── lod.rs                 # 地形LODの計画(どのタイル/チャンクをどのレベルで出すか)
│       │   ├── hillshade.rs           # HillshadeState(陰影のON/OFF)
│       │   ├── origin_pick.rs         # OriginPickState(地図クリックで原点を指定)
│       │   ├── recenter.rs            # RecenterRequestState(中心点の移動要求。原点へ/任意の地点へ)
│       │   ├── pick.rs                # 画面クリック→緯度経度のレイキャスト
│       │   ├── terrain.wgsl           # 頂点/フラグメントシェーダ
│       │   └── draw.wgsl              # 作図用シェーダ(太い線の画面px幅への展開を含む)
│       └── ui/                       # 上記を使うLeptosコンポーネント一式
│           ├── mod.rs
│           ├── terrain_view/           # 3D/2D地形描画canvas(mod.rsがコンポーネント。state/frame/lod_driver/overlay/labels/picking)
│           ├── util.rs                 # 小さな共通部品(copy_to_clipboard)
│           ├── los_view.rs             # 見通し範囲タブ(観測点一覧+極座標図)
│           ├── cross_section_view.rs   # 断面図タブ
│           ├── tabbed_panel.rs         # 汎用タブ付きパネル
│           ├── floating_panel.rs       # 汎用フローティングウインドウ(モーダル/非モーダルのウインドウ・ドラッグ移動)
│           ├── context_menu.rs         # 汎用の右クリックメニュー(項目は使う側が渡す)
│           ├── drawing_editor.rs       # 作図エディタ(ツール・一覧・数値編集。DrawToolStateを操作)
│           ├── origin_dialog.rs        # 原点設定フローティングパネル
│           └── coverage_altitude_dialog.rs # 覆域高度設定フローティングパネル
└── sample/                       # 「これはサンプルです」という位置づけのディレクトリ
    ├── sim_server/                 # C++側(sim3dviewのデータ契約を満たす参照実装サーバー)
    │   ├── CMakeLists.txt
    │   ├── vcpkg.json               # libuv/zlib
    │   ├── third_party/
    │   │   ├── uWebSockets/          # git submodule (uSockets含む)
    │   │   └── msgpack-cxx/          # git submodule (msgpack-c cpp_masterブランチ)
    │   ├── include/
    │   │   ├── protocol.hpp           # メッセージ定義・シリアライズ処理
    │   │   ├── simulation.hpp         # シミュレーション本体(状態・コマンドキュー)
    │   │   └── ws_server.hpp
    │   ├── src/
    │   │   ├── main.cpp
    │   │   ├── simulation.cpp
    │   │   └── ws_server.cpp
    │   └── assets/
    │       └── terrain/                 # 前処理ツールの出力(metadata.json/tile_index.json/base.bin/tiles/)
    └── sim_frontend/                # sim3dviewライブラリを使うサンプルアプリ(Rust)
        ├── Cargo.toml                 # sim3dviewをpath依存として使う
        ├── index.html                  # sim3dview.css・app.cssの両方を読み込む
        ├── Trunk.toml
        ├── src/
        │   ├── main.rs
        │   ├── app.rs                   # AppLayout。sim3dviewのcontext類をprovide_contextし、
        │   │                             # protocol::OriginState→sim3dview::terrain::origin::OriginState
        │   │                             # の橋渡しEffectを持つ
        │   ├── protocol.rs              # SimState/VabConfig/ClientCommand等の定義
        │   ├── ws.rs                    # WebSocket接続・再接続・受信処理
        │   ├── track_bridge.rs          # protocol::TrackList → sim3dview::terrain::tracks::TracksState の橋渡し
        │   └── components/
        │       ├── main_panel.rs         # sim3dview::ui::terrain_view::TerrainViewのラッパー
        │       ├── right_panel.rs        # sim3dview::ui::{tabbed_panel,los_view}を使う
        │       ├── menu_bar.rs           # sim3dviewのダイアログ・作図ウインドウの開閉トリガー
        │       ├── operation_panel.rs
        │       ├── vab.rs                # ページ数はmid_pagesプロパティで指定
        │       ├── status_panel.rs
        │       ├── track_detail.rs       # 選択した航跡の詳細(トップステータスパネルのタブ)
        │       ├── drawing_demo.rs       # 「作図デモ」(全種類の図形を置く例)
        │       ├── drawing_window.rs     # 作図エディタを入れる、動かせる非モーダルのウインドウ
        │       └── map_menu.rs           # 地図の右クリックメニューの項目(観測点の追加・原点・中心点・図形の作成…)
        └── style/
            └── app.css                    # アプリ固有(全体レイアウト・VAB・メニュー等)のみ
```

---

## 6. 実装フェーズ計画

以下は**初期の実装計画**(丸括弧内は当時の状況)。各フェーズの詳細な実装内容・確認結果は
DEVELOPMENT_HISTORY.mdおよびコード自体を参照。これ以降に足した機能(地形LOD・水域・陰影・作図・航跡・右クリックメニュー等)と
ライブラリの再実装の手順は[IMPLEMENTATION_GUIDE.md](IMPLEMENTATION_GUIDE.md)のフェーズ表を参照。

1. C++側: `protocol.hpp`定義 + uWebSocketsサーバーの雛形(ダミーデータ送信のみ)
2. Rust側: WebSocket接続 + 受信データのデシリアライズ確認、自動再接続(指数バックオフ+ジッター、タブ非表示時は一時停止)
3. Rust側: 3カラムレイアウトの骨組み(左パネル固定、地図・右パネルはリサイザー付き)
4. Rust側: VABコンポーネント実装(rows=6, cols=4のダミー`VabConfig`、単純クリックのみ)
5. C++側: 実シミュレーションループとの結合、コマンドキュー実装、原点状態の保持・変更ガード実装
6. C++側: GeoTIFF前処理ツール最小実装(1タイルのみで動作検証)
7. C++側: 17タイル全体のモザイク(欠損部0m埋め含む)+ 1024×1024ダウンサンプリング対応
   (のちに2048×2048へ引き上げ、さらにタイル単位のLOD方式へ置き換え。4節9番参照)
8. Rust側: heightmap.bin取得 → ENU変換(原点=デフォルト値)→ メッシュ生成 → 標高グラデーション着色で描画確認
9. Rust側: 原点入力フォーム、状況パネル(StatusPanelConfig駆動)の実装
10. Rust側: 自由視点カメラ(ズーム・回転・視点切り替え、俯瞰・側面の2パネル双方)の実装
11. UI細部の仕上げ: VABの空ラベルボタン非表示、パネルの横スクロール対応、再接続ステータス表示

**全11フェーズ、完了・動作確認済み**(フェーズ9の状況パネルはフェーズ3時点で、フェーズ11の3項目は
それぞれフェーズ2・3・4時点で既に実装済みだったため、実質的な残作業はフェーズ8・9の原点フォーム部分・
フェーズ10のみだった)。詳細な確認内容はDEVELOPMENT_HISTORY.mdの「現在の実装状況」を参照。

---

## 7. スコープ外事項(本書が対象としない範囲)

- オルソ画像(RGBテクスチャ)を用いた地形着色(標高グラデーションのみを採用)
- 垂直誇張(vertical exaggeration)機能
- リサイズ結果・原点設定・観測点等のブラウザ再読み込みをまたぐ永続化(セッション内のみ保持。例外として、作図エディタで作った図形だけはlocalStorageに保存して復元する。DETAILED_DESIGN.md 6.11節)
- 状況パネルのテキスト値表示(v1では数値項目のみ)
- マルチクライアントの個別状態管理(全員同一ブロードキャストのみ)

---

## 8. 関連文書

- [README.md](README.md) — 設計書一式の索引・読み順・保守ルール
- [DETAILED_DESIGN.md](DETAILED_DESIGN.md) — 詳細設計書。データフォーマット・座標変換・通信プロトコルのバイト定義・C++設計・ライブラリの設計方針・UML図、9節にライブラリの定数・アルゴリズム・バイト配置の要点
- [IMPLEMENTATION_GUIDE.md](IMPLEMENTATION_GUIDE.md) — ソースを見ずにライブラリを1から再実装するための実装ガイド(フェーズ・受け入れ基準・落とし穴)
- [DEVELOPMENT_HISTORY.md](DEVELOPMENT_HISTORY.md) — 機能ごとの実装経緯・動作確認済み事項・ハマりどころ
- [../README.md](../README.md) — セットアップ・ビルド・起動手順、既知の環境問題と回避策
- [../CLAUDE.md](../CLAUDE.md) — AIエージェント向けの作業方針・要点(短い索引)
