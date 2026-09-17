# C++シミュレータ Web UI 実装指示書

## 背景・目的
既存のC++シミュレータに、Rust(WASM/Leptos)製のWeb UIを追加する。
シミュレータ本体のコアロジックは変更せず、WebSocketサーバー機能を追加する形で連携させる。
中央の地図は、複数のGeoTIFFから生成した3D地形として表示する。

---

## アーキテクチャ概要

### シミュレーション連携(WebSocket, 高頻度)
```
┌───────────────────────────────────────────┐
│         C++ プロセス(1プロセス完結)          │
│                                             │
│  [Simスレッド]  →(Loop::defer)→ [uWSイベントループ] │
│  simulate_step()                 WS送受信    │
│                                             │
└─────────────────┬───────────────────────────┘
                   │ WebSocket (ws://localhost:9001/sim)
                   │ バイナリフレーム(MessagePack)
┌─────────────────▼───────────────────────────┐
│      ブラウザ (Rust → WASM, Leptos)            │
│  web_sys::WebSocket → rmp_serde → Signal更新 → 再描画(Canvas) │
└─────────────────────────────────────────────┘
```

- C++側とRust(WASM)側の1プロセス構成(中間サーバーなし)
- 通信はWebSocketのバイナリフレーム、シリアライズはMessagePack
- コマンド伝達(UI→シミュレータ)も同じWebSocket経由の双方向通信

### 地形データ連携(HTTP静的配信, 起動時1回)
```
[複数GeoTIFF(DEM、オルソ画像がある場合はそれも)]
    ↓
  前処理ツール(C++, GDAL使用。オフライン or 起動時バッチ実行)
    - 座標系統一(再投影)
    - モザイク(複数枚の結合)
    - ダウンサンプリング(メッシュ解像度に合わせる)
    ↓
  出力アセット(静的ファイル)
    - heightmap.bin (標高の生バイナリ)
    - texture.png   (オルソ画像テクスチャ、任意)
    - metadata.json (座標範囲・解像度等)
    ↓
  HTTP静的配信(WebSocketとは別チャンネル)
    ↓
  Rust(WASM)フロント:起動時にfetch → CPU側でメッシュ生成 → wgpuで描画
```

- 地形データはリアルタイム更新不要(起動時に1回読み込めば十分)
- シミュレーション状態のWebSocket高頻度チャンネルとは完全に分離する
- **対象エリアは固定範囲**(シミュレーション対象範囲のみ)と仮定し、単一メッシュでの描画を基本設計とする。広域を自由にパン/ズームして見て回る要件が後から出た場合は、クアッドツリー分割+タイル単位LODへの再設計が必要になる(本ドキュメントのスコープ外)

---

## 技術スタック

### C++側

| ライブラリ | 用途 | ライセンス |
|---|---|---|
| uWebSockets | WebSocketサーバー・静的ファイル配信 | Apache License 2.0 |
| msgpack-c | MessagePackシリアライズ | Boost Software License 1.0 |
| GDAL | GeoTIFF前処理(再投影・モザイク・ダウンサンプリング) | MIT/X11系ライセンス(一部ドライバは個別ライセンスの場合あり、要確認) |

### Rust側

| クレート | 用途 | ライセンス |
|---|---|---|
| leptos (features = ["csr"]) | リアクティブUIフレームワーク | MIT |
| wasm-bindgen | JS⇄WASMバインディング | MIT / Apache-2.0 |
| web-sys | Web API(WebSocket, Canvas等) | MIT / Apache-2.0 |
| js-sys | JS標準オブジェクトバインディング | MIT / Apache-2.0 |
| rmp-serde | MessagePack(Rust実装) | MIT |
| serde (features = ["derive"]) | シリアライズフレームワーク | MIT / Apache-2.0 |
| trunk | WASMビルド・開発サーバー | MIT / Apache-2.0 |
| wgpu | 地形メッシュのGPU描画(WebGPU/WebGL2) | MIT / Apache-2.0 |
| bytemuck | 頂点データのGPUバッファキャスト | MIT / Apache-2.0 |
| glam | ベクトル・行列演算(カメラ変換等) | MIT / Apache-2.0 |
| gloo-net(または web_sys の fetch) | heightmap.bin / texture.png / metadata.json の取得 | MIT / Apache-2.0 |

※ msgpack-c(Boost License)は配布時にライセンス文表示が必要。GDALはビルド構成によっては個別ライセンスのドライバを含む場合があるため、使用するフォーマットドライバに応じて要確認。それ以外は表示義務のない緩いライセンス。

※ 2026年時点でWebGPUはChrome/Edge/Firefox/Safariいずれも安定サポート済みのため、wgpuはWebGPUバックエンドを第一候補としてよい。ただし対象ユーザー環境によっては要確認。

---

## 通信プロトコル設計(シミュレーション連携)

### メッセージフレーミング
serdeの`tag`機能をC++側で素朴に再現するのは実装コストが高いため、**先頭1バイトをメッセージタイプ識別子、残りをMessagePackボディとする自前フレーミング**を採用する。

```
[1 byte: msg_type] [N bytes: MessagePack body]

msg_type:
  0x01 = SimState (シミュレーション状態、高頻度)
  0x02 = VabConfig (VABボタン設定、変化時のみ)
```

### メッセージ型定義(C++側 / Rust側で内容を一致させる)

**SimState** — シミュレーション状態(サーバー→クライアント、高頻度)
```
t: f64
positions: Vec<f32>
frame_id: u32
```

**VabConfig** — VABボタン設定(サーバー→クライアント、状態変化時のみ送信)
```
rows: u32
cols: u32
buttons: Vec<VabButton>
  VabButton:
    id: String
    label: String
    enabled: bool
```

**ClientCommand** — クライアント→サーバーへの操作コマンド
```
type: String  ("vab_press" / "pause" / "resume" / "set_param" など)
button_id: String  (vab_press時のみ使用)
value: f64  (set_param時のみ使用)
```

### 送信頻度
- シミュレーションループは既存の周期のまま(間引かない)
- WebSocket送信(SimState)は約16ms間隔(60Hz相当)に間引く
- VabConfigは変化があったときのみ送信(毎フレーム送らない)

---

## 地形データフォーマット仕様

前処理の出力は3ファイル1セットとする。**ハイトマップはPNGではなく生のf32バイナリにする**(PNG量子化による標高精度の劣化を避けるため)。

### heightmap.bin
- 標高値を row-major, little-endian の `f32` で並べた生バイナリ
- サイズ = `width * height * 4` バイト
- 単位はメートル(前処理側で統一)

### texture.png(オルソ画像がある場合のみ)
- 標準的なRGB PNG(またはJPEG)
- heightmap.binと同じ地理範囲・アスペクト比にリサンプリング済みであること
- ブラウザ側は`createImageBitmap`等のネイティブデコーダでそのまま読める形式にする(自前デコード実装は不要)

### metadata.json
```json
{
  "width": 1024,
  "height": 1024,
  "elevation_min": 120.5,
  "elevation_max": 890.2,
  "bounds": {
    "min_x": 0.0,
    "min_y": 0.0,
    "max_x": 5000.0,
    "max_y": 5000.0
  },
  "crs": "EPSG:xxxx"
}
```
- `bounds`はシミュレーション座標系と対応付けられる単位(メートル等、要すり合わせ)で記述する
- `crs`は前処理時にGeoTIFFから読み取った元の座標系情報を記録用に残す(描画自体には`bounds`を使う)

---

## UIレイアウト

3カラム構成。左右パネルはそれぞれ上下2分割。

```
┌─────────────┬───────────────────┬─────────────┐
│ 操作パネル(上) │                   │ 状況パネル(上) │
│  ステータス    │                   │  各種情報     │
├─────────────┤     地図(3D地形)    ├─────────────┤
│ 操作パネル(下) │                   │ 状況パネル(下) │
│    VAB       │                   │  側面図       │
└─────────────┴───────────────────┴─────────────┘
```

- CSS Gridの`grid-template-areas`で3カラムレイアウトを構成する
- 左パネル = 操作パネル(上:シミュレータステータス表示 / 下:VAB)
- 中央 = 地図(GeoTIFF由来の3D地形、wgpu描画)
- 右パネル = 状況パネル(上:各種情報表示 / 下:地図の側面図)
- 左右パネル幅は固定(`--panel-width: 320px`程度、CSS変数化しておく)
- 上下分割比率は初期値 `1fr 1fr`(均等)。コンテンツ確定後に調整可能な形にしておく
- **中央の地図と右パネル下部の側面図は、同一の地形メッシュに対して異なるカメラ(ビュー行列)を適用する形で実現する**(メッシュを2重に持つ必要はない)

### VAB(ボタングリッド)仕様
- 正方形ボタンを格子状(rows × cols)に配置
- ボタンのラベル・有効/無効状態はハードコードせず、サーバーから配信される`VabConfig`に応じて動的に決まる
- ボタン押下時は`ClientCommand{type: "vab_press", button_id: ...}`をWebSocket経由でサーバーに送信する
- ボタンの正方形維持には`aspect-ratio: 1/1`を使用する
- ラベルが空文字のボタンは「未使用の穴」として扱う(表示要件は実装時に確認)

### WebSocket接続処理
- `web_sys::WebSocket`を使用し、`set_binary_type(BinaryType::Arraybuffer)`を必ず設定する(デフォルトのBlobのままだと受信データの型が期待と異なる)
- 受信メッセージは先頭1バイトで`msg_type`を判別し、`SimState`/`VabConfig`それぞれに`rmp_serde::from_slice`でデシリアライズする
- 受信データはLeptosの`Signal`で保持し、UIへリアクティブに反映する

---

## C++側 実装要件

### スレッドモデル(シミュレーション連携)
- シミュレーションスレッドとuWebSocketsイベントループスレッドを分離する
- 別スレッドからWebSocket送信する際は必ず `uWS::Loop::defer()` を経由する(uWebSocketsはシングルスレッド前提のため、直接`ws->send()`を呼ぶと未定義動作になる)
- クライアントリスト(`std::vector<uWS::WebSocket*>`)へのアクセスは`std::mutex`で保護する

### コマンド処理
- `.message`ハンドラで受信したコマンドは直接シミュレーション状態を書き換えず、スレッドセーフなキュー(`std::queue` + `std::mutex`)に積む
- シミュレーションスレッド側でstep()の前後にキューを消費して適用する

### GeoTIFF前処理ツール
シミュレーション本体(`sim_server`)とは別の、単発実行のCLIツールとして実装する。ビルド時 or 地形データ更新時に手動実行する想定。常駐サーバーには含めない。

処理ステップ:
1. 入力GeoTIFF群を読み込み、共通の座標系に再投影する
2. 再投影後の複数GeoTIFFをモザイク(1枚に結合)する
3. 目標メッシュ解像度(例: 1024×1024)にダウンサンプリングする
4. 標高バンドを抽出し、`heightmap.bin`(f32生バイナリ)として出力する
5. RGBバンドがあれば、同じ地理範囲でリサンプリングして`texture.png`として出力する
6. 座標範囲・解像度・標高min/maxを`metadata.json`に出力する

出力先(`assets/terrain/`)は、`sim_server`(uWebSockets または `cpp-httplib`)の静的ファイル配信機能でHTTP配信する。WebSocketの`/sim`エンドポイントとは独立したルートにする。

### ディレクトリ構成(想定)
```
sim_server/
├── CMakeLists.txt
├── third_party/
│   ├── uWebSockets/      # git submodule
│   └── msgpack-c/        # git submodule
├── include/
│   ├── simulation.hpp     # 既存シミュレーションコア(変更最小限)
│   ├── ws_server.hpp
│   └── protocol.hpp        # メッセージ定義・シリアライズ処理
├── src/
│   ├── main.cpp
│   ├── simulation.cpp
│   ├── ws_server.cpp
│   └── protocol.cpp
├── tools/
│   └── geotiff_preprocess/
│       ├── CMakeLists.txt   # GDAL依存はこのツール限定にする
│       └── main.cpp
├── assets/
│   └── terrain/
│       ├── heightmap.bin
│       ├── texture.png
│       └── metadata.json
```

---

## Rust(Leptos/WASM)側 実装要件

### 地形描画の処理フロー
1. 起動時に`metadata.json`をfetchしてパース
2. `heightmap.bin`をfetchし、`width * height`個の`f32`標高値として読み込む
3. CPU側で平面グリッドメッシュを構築し、各頂点のY座標に標高値を直接ベイクする(頂点シェーダでのテクスチャサンプリングによる変位ではなく、静的地形なのでCPU側で確定させる方針)
4. `texture.png`があれば`createImageBitmap`でデコードし、wgpuのテクスチャとしてアップロード。UV座標はグリッド位置から計算する
5. `bounds`情報を使って、メッシュのワールド座標とシミュレーション座標系の対応を取る
6. カメラ(中央パネルの俯瞰視点、右パネル下部の側面視点)は同一メッシュに対して異なるビュー行列を適用する形で実現する

### 頂点構造(例)
```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TerrainVertex {
    position: [f32; 3], // x, y(標高), z
    uv: [f32; 2],
}
```

### メッシュ解像度についての注意
DEMの生解像度が非常に高い場合、頂点数がそのまま描画負荷に直結する。前処理側のダウンサンプリング解像度(`metadata.json`の`width`/`height`)でメッシュの頂点密度が決まる設計にしているため、**パフォーマンスの調整は基本的に前処理側の出力解像度で行う**(フロント側で間引く実装は本設計には含めない)。

### ディレクトリ構成(想定)
```
sim_frontend/
├── Cargo.toml
├── index.html
├── src/
│   ├── main.rs
│   ├── app.rs          # AppLayout(全体レイアウト)
│   ├── protocol.rs      # SimState/VabConfig/ClientCommand定義
│   ├── ws.rs            # WebSocket接続・受信処理
│   ├── components/
│   │   ├── operation_panel.rs
│   │   ├── vab.rs
│   │   ├── map_view.rs        # 地形描画の呼び出し・カメラ切り替え
│   │   └── status_panel.rs
│   ├── terrain/
│   │   ├── mod.rs
│   │   ├── loader.rs          # heightmap.bin / texture.png / metadata.json 取得
│   │   ├── mesh.rs            # メッシュ生成
│   │   └── camera.rs          # 俯瞰・側面ビューのカメラ変換
├── style/
│   └── app.css
```

---

## 未確定・実装時に判断/確認が必要な項目

以下はこれまでの検討で結論が出ていない、または実装者の判断に委ねられている点。Claude Codeに実装を依頼する際は、これらについて質問するか、妥当なデフォルトを仮置きして進めること。

1. **再接続処理**: WebSocket切断検知時のフロント側自動リトライ処理(未実装)
2. **マルチクライアント**: 複数クライアント接続時、全員に同一ブロードキャストでよいか、個別状態が必要か
3. **VABの行数・列数**: 固定か、画面サイズに応じた可変か
4. **VABの操作種別**: 単純クリックのみか、長押し・トグル等の操作も必要か
5. **状況パネル上部「各種情報」の具体的な表示項目**: 未定義
6. **左右パネルのレスポンシブ対応**: 固定幅のままでよいか
7. **地形エリアの規模**: 固定範囲(単一メッシュ)前提で設計。広域パン/ズームが必要になった場合はタイル化(LOD)の再設計が別途必要
8. **オルソ画像の有無**: GeoTIFFが標高データのみか、RGB画像も含むか未確認。含まない場合、テクスチャは単色 or 等高線的な着色(標高に応じたグラデーション)で代替する必要がある
9. **地形メッシュ解像度の上限**: パフォーマンス要件(フレームレート目標、対象デバイス)次第で前処理側の出力解像度を決める
10. **垂直誇張(vertical exaggeration)の要否**: 実際の標高差が地形の水平スケールに対して小さい場合、見た目がほぼ平坦になることがある。標高値に係数を掛けて強調表示する機能が必要か確認
11. **座標系のすり合わせ**: `metadata.json`の`bounds`とシミュレーション本体の座標系(単位・原点)の対応関係を明確にする必要あり
12. **地形描画のカメラ操作**: マウスでの自由視点操作が必要か、固定アングルの俯瞰・側面表示のみで十分か

---

## 実装フェーズ(推奨順序)

1. C++側: `protocol.hpp`定義 + uWebSocketsサーバーの雛形(ダミーデータ送信のみ)
2. Rust側: WebSocket接続 + 受信データのデシリアライズ確認(コンソール出力のみでUI未着手)
3. Rust側: 3カラムレイアウトの骨組み(CSS Grid、中身は空のプレースホルダー)
4. Rust側: VABコンポーネント実装(ダミーVabConfigで表示確認)
5. C++側: 実シミュレーションループとの結合、コマンドキュー実装
6. C++側: GeoTIFF前処理ツール実装(単一GeoTIFF入力→`heightmap.bin`/`metadata.json`出力の最小実装)
7. C++側: 複数GeoTIFFの再投影・モザイク対応、静的ファイル配信の追加
8. Rust側: `heightmap.bin`取得→CPU側メッシュ生成→wgpuで単色描画(テクスチャなし)確認
9. Rust側: `texture.png`のオルソ画像貼り付け対応
10. Rust側: 俯瞰・側面の2カメラ対応、垂直誇張などの調整機能追加
11. 再接続処理・エラーハンドリングの追加
