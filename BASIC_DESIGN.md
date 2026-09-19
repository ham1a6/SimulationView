# Sim3dView 基本設計書

## 0. 本書の位置づけ

本書は、旧指示書 `claude_code_instructions.md`(以下「旧指示書」)および旧設計書
`DESIGN.md`(以下「旧設計書」)の内容を全て統合・整理した**基本設計書**である。
詳細な数式・プロトコルのバイト定義・UML図は [DETAILED_DESIGN.md](DETAILED_DESIGN.md)(詳細設計書)を参照する。
本書と詳細設計書の完成後、旧指示書・旧設計書は削除される予定であり、両者に記載されていた内容は
本書または詳細設計書のいずれかに漏れなく引き継がれている。

新しいセッション・新しい開発者が本プロジェクトに参加する際は、まず本書を読み、詳細な仕様が必要になった箇所で
詳細設計書を参照する、という使い方を想定する。

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
[map_data/ 内のALOS DSM GeoTIFF ×17]
    ↓
  前処理ツール(C++, GDAL使用。geotiff_preprocessという単発CLI)
    - モザイク(17タイルを緯度経度グリッドのまま敷き詰め。再投影は不要)
    - ダウンサンプリング(平均法で2048×2048へ)
    ↓
  出力アセット(静的ファイル。sim_server/assets/terrain/)
    - heightmap.bin (標高の生バイナリ、f32)
    - metadata.json (緯度経度範囲・楕円体パラメータ等)
    ↓
  HTTP静的配信(sim_serverが/terrain/*で配信。WebSocketの/simとは別ルート)
    ↓
  Rust(WASM)フロント: 起動時にfetch → ENU変換 → CPU側でメッシュ生成 → wgpuで描画
```

- 地形データはリアルタイム更新不要(起動時に1回読み込めば十分)
- シミュレーション状態のWebSocket高頻度チャンネルとは完全に分離する
- 原点(座標系の基準点)はUIからランタイムに入力される値であり、前処理側は原点非依存の生データを出力する
  (詳しい経緯は4節・詳細設計書4節を参照)
- **対象エリアは固定範囲**(map_data内17タイルの外接矩形)を単一メッシュとして描画する。将来、広域を
  自由にパン/ズームして見て回る要件が出た場合は、クアッドツリー分割+タイル単位LODへの再設計が必要になる
  (本書のスコープ外)

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
| gloo-net | HTTP fetch(heightmap.bin/metadata.json取得) | MIT / Apache-2.0 |
| trunk | WASMビルド・開発サーバー | MIT / Apache-2.0 |
| wgpu | 地形メッシュのGPU描画(WebGPU) | MIT / Apache-2.0 |
| bytemuck | 頂点データのGPUバッファキャスト | MIT / Apache-2.0 |
| glam | ベクトル・行列演算(ENU変換・カメラ) | MIT / Apache-2.0 |

---

## 4. 要求仕様と確定した設計方針の一覧

旧指示書には「未確定・実装時に判断/確認が必要な項目」が12項目あり、旧設計書でのユーザーとの対話により
すべて確定した。さらにその確定に伴い実装レベルで新たに生じた5項目も別途確定している。以下は両方を統合した
最終確定事項の一覧である(本書が唯一の正とする)。

| # | 項目 | 確定内容 |
|---|---|---|
| 1 | 再接続処理 | **自動リトライする**(指数バックオフ+ジッター、無制限リトライ、タブ非表示中は一時停止) |
| 2 | マルチクライアント | **全員同一ブロードキャスト**でよい(個別状態は持たない) |
| 3 | VABの行数・列数 | 開発用ダミー`VabConfig`の初期値は**縦4行×横6列**(24ボタン) |
| 4 | VABの操作種別 | **単純クリックのみ**(長押し・トグル等は実装しない) |
| 5 | 状況パネル上部「各種情報」の表示項目 | **C++側(サーバー)から動的に設定できる**。`StatusPanelConfig`メッセージで配信 |
| 6 | 左右パネルのレスポンシブ対応 | 左パネルは固定でレスポンシブ不要。**地図(中央)と右パネルはレスポンシブ対応 + ユーザーがUIでサイズ変更可能** |
| 7 | 地形エリアの規模 | `map_data`内**17タイル全体をモザイク**し、単一の広域メッシュとする |
| 8 | オルソ画像の有無 | RGBオルソ画像なし。パンクロ画像(STK.tif)も**使用せず、標高グラデーション着色のみ** |
| 9 | メッシュ解像度 | **2048×2048**(約419万頂点。当初1024×1024だったが、「マップの解像度を上げてほしい」との要望により2048×2048へ引き上げた。5°四方≒555kmに対し1グリッドセルは約271m。単一の固定メッシュ全体を丸ごとGPUに載せる設計のままなので、これ以上の大幅な引き上げにはLOD化等の設計変更が必要) |
| 10 | 垂直誇張の要否 | **不要**。実メートル値のまま描画する(誇張機能は実装しない) |
| 11 | 座標系のすり合わせ | 原点は**UIから緯度経度を入力**して指定。原点=入力地点の海抜0m地点。**東=X、北=Y、鉛直上向き=Z**の局所ENU座標系。**C++側もUIと全く同じ座標系・原点を用いる**。デフォルト原点は**北緯35°21'20"、東経138°51'35"**(35.355556°, 138.859722°) |
| 12 | カメラの自由視点化 | **ズーム・角度切り替え(回転)・視点切り替えを自由に操作できる**カメラとする(実装中。詳細設計書参照) |
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
├── BASIC_DESIGN.md            # 本書
├── DETAILED_DESIGN.md         # 詳細設計書(数式・プロトコル・UML)
├── README.md                  # セットアップ・起動手順・環境問題の対処
├── DEVELOPMENT_HISTORY.md     # 機能ごとの実装経緯・ハマりどころの記録
├── CLAUDE.md                  # AIエージェント向けの作業方針・要点(短い索引)
├── Cargo.toml                  # ワークスペースルート(members: sim3dview, sample/sim_frontend)
├── map_data/                  # 入力: ALOS DSM GeoTIFFタイル(17枚、既存・変更しない)
│   └── ALPSMLC30_N###E###_DSM.tif ...
├── sim3dview/                   # ライブラリ本体(Rust/Leptos/WASM)。使い方はsim3dview/README.md参照
│   ├── Cargo.toml
│   ├── README.md                 # 開発者向け使い方ドキュメント
│   ├── style/
│   │   └── sim3dview.css
│   └── src/
│       ├── lib.rs
│       ├── terrain/                # データ取得・座標変換・カメラ・wgpu描画・覆域/見通し計算
│       │   ├── mod.rs
│       │   ├── loader.rs             # heightmap.bin/metadata.json取得(base_urlは呼び出し側が指定)
│       │   ├── mesh.rs                # ENU変換・メッシュ生成
│       │   ├── camera.rs              # カメラ(ビュー・射影行列、2D/3D)
│       │   ├── renderer.rs            # wgpu描画パイプライン
│       │   ├── store.rs               # TerrainStore(地形データの共有キャッシュ)
│       │   ├── origin.rs              # OriginState(現在の原点、プロトコル非依存)
│       │   ├── markers.rs             # RadarMarker/RadarMarkersState・覆域ジオメトリ生成
│       │   ├── los.rs                 # 見通し/覆域計算
│       │   ├── pick.rs                # 画面クリック→緯度経度のレイキャスト
│       │   └── terrain.wgsl           # 頂点/フラグメントシェーダ
│       └── ui/                       # 上記を使うLeptosコンポーネント一式
│           ├── mod.rs
│           ├── terrain_view.rs         # 3D/2D地形描画canvas
│           ├── los_view.rs             # 見通し範囲タブ(観測点一覧+極座標図)
│           ├── tabbed_panel.rs         # 汎用タブ付きパネル
│           ├── floating_panel.rs       # 汎用フローティングウインドウ
│           ├── origin_dialog.rs        # 原点設定フローティングパネル
│           └── coverage_altitude_dialog.rs # 覆域高度設定フローティングパネル
└── sample/                       # 「これはサンプルです」という位置づけのディレクトリ
    ├── sim_server/                 # C++側(sim3dviewのデータ契約を満たす参照実装サーバー)
    │   ├── CMakeLists.txt
    │   ├── vcpkg.json               # libuv/zlib/gdal(前処理ツール専用)
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
    │   ├── tools/
    │   │   └── geotiff_preprocess/     # GDAL依存はここに限定
    │   │       ├── CMakeLists.txt
    │   │       └── main.cpp
    │   └── assets/
    │       └── terrain/                 # 前処理ツールの出力(heightmap.bin/metadata.json)
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
        │   └── components/
        │       ├── main_panel.rs         # sim3dview::ui::terrain_view::TerrainViewのラッパー
        │       ├── right_panel.rs        # sim3dview::ui::{tabbed_panel,los_view}を使う
        │       ├── menu_bar.rs           # sim3dviewのダイアログ開閉トリガー
        │       ├── operation_panel.rs
        │       ├── vab.rs
        │       └── status_panel.rs
        └── style/
            └── app.css                    # アプリ固有(全体レイアウト・VAB・メニュー等)のみ
```

---

## 6. 実装フェーズ計画

以下の順序で実装を進める(丸括弧内は本書執筆時点の状況)。各フェーズの詳細な実装内容・確認結果は
開発セッションのログおよびコード自体を参照。

1. C++側: `protocol.hpp`定義 + uWebSocketsサーバーの雛形(ダミーデータ送信のみ)
2. Rust側: WebSocket接続 + 受信データのデシリアライズ確認、自動再接続(指数バックオフ+ジッター、タブ非表示時は一時停止)
3. Rust側: 3カラムレイアウトの骨組み(左パネル固定、地図・右パネルはリサイザー付き)
4. Rust側: VABコンポーネント実装(rows=4, cols=6のダミー`VabConfig`、単純クリックのみ)
5. C++側: 実シミュレーションループとの結合、コマンドキュー実装、原点状態の保持・変更ガード実装
6. C++側: GeoTIFF前処理ツール最小実装(1タイルのみで動作検証)
7. C++側: 17タイル全体のモザイク(欠損部0m埋め含む)+ 1024×1024ダウンサンプリング対応
   (のちに2048×2048へ引き上げ。4節9番参照)
8. Rust側: heightmap.bin取得 → ENU変換(原点=デフォルト値)→ メッシュ生成 → 標高グラデーション着色で描画確認
9. Rust側: 原点入力フォーム、状況パネル(StatusPanelConfig駆動)の実装
10. Rust側: 自由視点カメラ(ズーム・回転・視点切り替え、俯瞰・側面の2パネル双方)の実装
11. UI細部の仕上げ: VABの空ラベルボタン非表示、パネルの横スクロール対応、再接続ステータス表示

**全11フェーズ、完了・動作確認済み**(フェーズ9の状況パネルはフェーズ3時点で、フェーズ11の3項目は
それぞれフェーズ2・3・4時点で既に実装済みだったため、実質的な残作業はフェーズ8・9の原点フォーム部分・
フェーズ10のみだった)。詳細な確認内容はDEVELOPMENT_HISTORY.mdの「現在の実装状況」を参照。

---

## 7. スコープ外事項(本書が対象としない範囲)

- 広域パン/ズームに対応するタイル分割・LOD機構(2.3節参照)
- オルソ画像(RGBテクスチャ)を用いた地形着色(標高グラデーションのみを採用)
- 垂直誇張(vertical exaggeration)機能
- リサイズ結果・原点設定等のブラウザ再読み込みをまたぐ永続化(localStorage等は使用しない。セッション内のみ保持)
- 状況パネルのテキスト値表示(v1では数値項目のみ)
- マルチクライアントの個別状態管理(全員同一ブロードキャストのみ)

---

## 8. 関連文書

- [DETAILED_DESIGN.md](DETAILED_DESIGN.md) — 詳細設計書。データフォーマット・座標変換の数式・通信プロトコルの
  バイト定義・クラス図/シーケンス図/状態遷移図(UML)を含む
- [README.md](README.md) — セットアップ・ビルド・起動手順、既知の環境問題と回避策
- [DEVELOPMENT_HISTORY.md](DEVELOPMENT_HISTORY.md) — 機能ごとの実装経緯・動作確認済み事項・ハマりどころ
- [CLAUDE.md](CLAUDE.md) — AIエージェント向けの作業方針・要点(短い索引)
