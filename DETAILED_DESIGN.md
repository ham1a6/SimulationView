# Sim3dView 詳細設計書

## 0. 本書の位置づけ

[BASIC_DESIGN.md](BASIC_DESIGN.md)(基本設計書)の詳細版。データフォーマット・座標変換の数式・
通信プロトコルのバイト単位の定義・クラス図/シーケンス図/状態遷移図(UML、Mermaid記法)を記載する。
旧指示書 `claude_code_instructions.md` と旧設計書 `DESIGN.md` の内容はすべて本書または基本設計書に
統合済みであり、旧2文書は削除後もこの2文書のみで全内容を参照できる。

UML図はMermaid記法で記述している。GitHub/GitLab/VSCode等、Mermaidをネイティブサポートするツールで
プレビューすればそのまま図として描画される。

---

## 1. 対象地形データの実態調査結果

### 1.1 ファイル構成(1タイルあたり)

`map_data/ALPSMLC30_<TILEID>_*` の形式で、17タイル分×6ファイル = 102ファイル。

| サフィックス | 内容 | 本設計での用途 |
|---|---|---|
| `_DSM.tif` | 数値表層モデル(標高、GeoTIFF, 3600×3600px) | **使用**(唯一の入力ラスタ) |
| `_MSK.tif` | 品質マスク(雲・水域等のフラグ) | 不使用(v1) |
| `_STK.tif` | パンクロマチック(白黒)画像 | **不使用**(確定事項) |
| `_HDR.txt` | タイルのヘッダ情報(四隅座標・解像度・楕円体等) | 前処理ツールのテスト・検証用参考情報 |
| `_LST.txt` | 元シーン(観測パス)のリスト | 使用しない |
| `_QAI.txt` | 品質指標(SRTM/ASTERとの差分統計等) | 使用しない |

### 1.2 タイル分布と規模

タイルID命名: `N<緯度2桁>E<経度3桁>` = タイル南西角の整数度。1タイル = 経緯度1°×1°、3600×3600px
(1秒角)。

実在タイル(17枚、対角状に分布。緯度が上がるほど東経範囲が狭まる):

```
lat39:                                              E139
lat38:                                      E138 E139
lat37:                            E136 E137 E138 E139
lat36:      E135 E136 E137 E138 E139
lat35:      E135 E136 E137 E138 E139
```

- 外接矩形: 北緯35°〜40°、東経135°〜140°(5°×5°の格子のうち17/25セルが実在)。
  **この外接矩形はハードコードではなく、`geotiff_preprocess`が実際に見つかったタイルの
  南西角ID(ファイル名から取得)の最小/最大から実行時に自動計算する**(2.3節)。
  そのため`map_data/`に別の場所のタイルを追加/削除しても、コード変更なしに追従する
- 欠損8セル: N037E135, N038E135, N038E136, N038E137, N039E135, N039E136, N039E137, N039E138
- 実距離換算で概ね南北550km×東西450km規模(この緯度帯では経度1°≈90km、緯度1°≈111km)
- 解像度: 1秒角(3600px/度)= 南北方向約30m/px、東西方向は緯度のcos分だけ狭く約24〜27m/px

### 1.3 標高データの実態(全17タイル実測)

- 標高範囲: **-102.0m 〜 3771.0m**(単一ピクセル最大値。位置的に富士山と整合。東経138°/北緯35°タイル内)
- 明示的なnodataセンチネル値(-9999等)は検出されなかった
- データ型: GeoTIFF、符号付き整数(16bit相当。前処理でf32メートルに変換)
- 平均法ダウンサンプリング後(1024×1024)の実測範囲: 約-11.5m 〜 3677.9m
  (平均化により単一ピクセルの極値より穏やかな範囲になる。想定通りの挙動)

### 1.4 欠損タイルへの対応方針

- モザイク時、無データ領域(存在しないタイル + 各タイル内のNODATA画素 + マスクファイルが
  海と示す画素、下記参照)は**f32のNaNで一律埋める**(標高0mとは明確に区別する。標高0m付近の
  実在する陸地と見た目上混同されてしまうため。当初は0mで埋めていたが、複数GeoTIFFタイルの間に
  ある無データ領域を陸地と区別して海として塗り分けたいという要望を受けてNaNへ変更した)
- フロント側は`heightmap`の値が`NaN`の格子点を海と判定し、標高0mの平面として配置した上で
  地形グラデーションとは別の水色で塗る(6.7節)
- `metadata.json`に欠損フラグは設けない(NaN自体が「データなし」の目印になるため)。

**タイル内部の海域はマスクファイル(`*_MSK.tif`)ベースで検出する**: 当初はGDALの
`GetNoDataValue()`(明示的なNODATAセンチネル、1.3節の通りこのデータセットでは検出されない)
だけを見ていたが、これだとタイル境界をまたがない・タイル内部で完結する海域(沿岸部・
内海など)を検出できない。実測したところ、ALOS World 3D-30mのDSMはタイル内部の海域画素に
「標高0m」という一見有効に見える値を格納しており(NODATAセンチネルではない)、対応する
`*_MSK.tif`(DSMと同じ3600×3600グリッド、1バイト/画素)の画素値が**3**の位置が正確に海域に
対応することを、同梱の`*_QAI.txt`の`MASK_NUM_SEA`統計値との突き合わせで確認した(画素値の
内訳: 0=有効データ, 3=海, 4/12=代替データソース[GSI10/PSM]で補完した陸地でありNaN化はしない)。
`load_sea_mask()`(`geotiff_preprocess/main.cpp`)がDSMと同名(拡張子前の`_DSM`を`_MSK`に
置換)の`_MSK.tif`を読み、値が3の画素をNaNへ追加で置換する。マスクファイルが見つからない・
サイズが一致しないなど読めない場合は警告を出してNODATAベースの判定のみにフォールバックし、
処理は止めない。この変更で、モザイク全体に占めるNaN(海)の割合は約32%(NODATAのみ)から
約60%(マスク併用)に増え、沿岸部・内海の描画精度が大幅に改善した。
  `elevation_min`/`elevation_max`はNaNを除いた実データのみで計算する

---

## 2. GeoTIFF前処理ツール仕様

### 2.1 入力

`map_data/*_DSM.tif` を機械的に列挙する(ディレクトリをglobし、ファイル名から`N###E###`パターンを
抽出してタイル位置を決定する。将来タイル追加にも対応できる)。

### 2.2 座標系(再投影は不要)

ALOS DSMタイルは元々EPSG:4326(WGS84/GRS80楕円体)の緯度経度グリッドで、1タイル=1°×1°=3600×3600px
ちょうどに整列している(タイル境界での位置ズレなし)。原点(=座標変換の基準)がUI入力によるランタイム
パラメータであるため、前処理側で特定の投影に固定する意味がない。よって:

- **再投影(reproject)は行わない**。17タイルを緯度経度グリッドのまま単純にモザイクし、ダウンサンプリング
  するだけでよい
- 実際のメートル単位の座標(東/北/上)への変換は、UIで指定された原点をもとに**フロント側が実行時に行う**
  (4節参照)
- GDALの役目は「GeoTIFFの読み取り・モザイク・ダウンサンプリング・書き出し」に限定される

### 2.3 モザイク

タイルを緯度経度グリッド上のピクセル位置(タイルIDから一意に決まる)にそのまま敷き詰める。
外接矩形は固定のハードコード値ではなく、2段階の処理で実行時に決める:

1. `discover_tiles()`: `map_data/`内の`*_DSM.tif`をディレクトリ走査し、ファイル名からタイルID
   (南西角の整数緯度経度)だけを読み取る(GDALでの実データ読み込みはまだ行わない、高速)
2. `compute_mosaic_bounds()`: 見つかった全タイルIDの緯度・経度それぞれの最小値・最大値から
   外接矩形(`MosaicBounds{min_lat, max_lat, min_lon, max_lon}`。max側はタイル南西角+1)を求める。
   現在の17タイル構成では北緯35°〜40°・東経135°〜140° → **18000×18000px**(3600px/度 × 5度)に
   なるが、これは実データから導出された結果であり、`map_data/`に別の場所のタイルを追加/削除
   すれば次回実行時に自動的に変わる
3. `build_mosaic()`: 決まった外接矩形のcanvas上に、各タイルを実際に読み込んで敷き詰める。
   タイルが存在しないセル(欠損タイル)はNaNで埋める(1.4節)

タイル南西角座標`(tile_lat, tile_lon)`から、モザイクcanvas上の配置位置(北→南、西→東の順で格納)を
求める式:

```
row_offset = (mosaic_max_lat - (tile_lat + 1)) * 3600
col_offset = (tile_lon - mosaic_min_lon) * 3600
```

### 2.4 ダウンサンプリング

- 目標解像度: 1024×1024
- リサンプリング手法: **平均法(average)**。18000÷1024≈17.6倍の間引きになるため、最近傍やバイリニアだと
  標高の代表性が落ちる
- 実装は、出力先セル`(dx, dy)`に対応する入力範囲`[sx0,sx1) x [sy0,sy1)`を整数演算で求め、その範囲内の
  全ピクセルの平均を取る
- NaN(1.4節)は平均の対象から除外する。範囲内にNaNでない画素が1つでもあればその平均を採用し
  (陸地の縁で実データを最大限活かす)、範囲全体がNaNの場合のみ出力もNaN(海)にする

### 2.5 出力

| ファイル | 内容 | 備考 |
|---|---|---|
| `heightmap.bin` | f32, row-major, little-endian, 1024×1024, 標高値(海抜メートル) | **行順は南→北**(2.6節の座標復元式に対応させるため、GDAL標準の北→南から反転させて出力する) |
| `texture.png` | **生成しない**(v1) | 標高グラデーション着色を使うため不要 |
| `metadata.json` | 下記2.6参照 | 緯度経度範囲・標高範囲・楕円体パラメータを含む |

> **実装上の注意(重要)**: GDALは標準でラスタを北→南(row 0 = 北端)の順で格納する。しかし
> `metadata.json`の`geodetic_bounds`から緯度経度を復元する式(2.6節)は南→北(row 0 = 南端)を
> 前提としている。この食い違いを埋めるため、前処理ツールは**ダウンサンプリング後・出力直前に
> 行順を反転**させている。これを怠ると、フロント側で地形が南北反転して描画されるバグになる
> (実装時に実際に発生し、修正済み)。

### 2.6 `metadata.json` スキーマ

```json
{
  "width": 1024,
  "height": 1024,
  "elevation_min": -11.5457515716553,
  "elevation_max": 3677.90185546875,
  "geodetic_bounds": {
    "min_lat": 35.0,
    "max_lat": 40.0,
    "min_lon": 135.0,
    "max_lon": 140.0
  },
  "source_crs": "EPSG:4326 (WGS84相当, GRS80楕円体)",
  "height_datum": "orthometric height (EGM96 geoid)",
  "ellipsoid": { "a_m": 6378137.0, "inv_f": 298.257222101 },
  "has_texture": false,
  "default_origin": {
    "lat_dms": "35°21'20\"N",
    "lon_dms": "138°51'35\"E",
    "lat_deg": 35.355556,
    "lon_deg": 138.859722
  }
}
```

- `geodetic_bounds`: 各グリッドセル`(i, j)`の緯度経度は次式で線形補間して求める
  (`j`=行インデックス、`height`=1024。等間隔の緯度経度グリッドのため成立する)

  ```
  lat(j) = min_lat + (j / (height - 1)) * (max_lat - min_lat)
  lon(i) = min_lon + (i / (width  - 1)) * (max_lon - min_lon)
  ```

- `elevation_min`/`elevation_max`: 前処理時に**ダウンサンプリング後の出力データ**から実測する
  (前処理ツール実装上、`std::setprecision(15)`を明示指定して出力する。ostreamの既定精度(有効6桁)の
  ままだと`138.859722`が`138.86`のように丸められてしまうバグが実装時に発生し、修正済み)
- `has_texture`: フロント側が`texture.png`の有無を毎回fetch失敗で判定せずに済むよう明示フラグとして
  持たせている
- `default_origin`: UIの原点入力欄の初期値。実際に使われる原点はサーバー側が保持する状態が正
  (`OriginState`。5節参照)であり、これはあくまで未接続時のUI初期表示用
- `ellipsoid`/`height_datum`: フロント側のENU変換計算(3節)で使うパラメータ

### 2.7 前処理ツール処理フロー(UML: アクティビティ相当のフローチャート)

```mermaid
flowchart TD
    A["map_data/*_DSM.tifを列挙"] --> B["各タイルをGDALで読み込み\n(ファイル名からtile_lat/tile_lonを抽出)"]
    B --> C["18000x18000キャンバスへ配置\n(欠損タイル分は0mのまま)"]
    C --> D["平均法で1024x1024へダウンサンプリング"]
    D --> E["行順を北→南から南→北へ反転"]
    E --> F["elevation_min/maxを実測"]
    F --> G["heightmap.bin書き出し"]
    F --> H["metadata.json書き出し\n(setprecision(15)で精度確保)"]
```

---

## 3. 座標系設計(ENU座標系)

### 3.1 定義

- シミュレーション座標系 = **東=X、北=Y、鉛直上向き=Z** の局所ENU(East-North-Up)座標系
- 原点 = UIで入力された緯度経度地点の、**海抜0m(ジオイド高0m)の点**
- デフォルト原点(試験用): 北緯35°21'20"、東経138°51'35"(10進: 35.355556°, 138.859722°)。
  地形データの範囲内(タイル`N035E138`内、富士山付近)に収まっている

### 3.2 変換式(緯度経度+標高 → ENU)

地形グリッドの各点は`(lat, lon, h)`(hはDSMの標高値=海抜メートル)で表現される。原点`(lat0, lon0)`が
決まれば、標準的な測地座標→ECEF→ENU変換で一意にメートル座標が求まる。GRS80楕円体
(`metadata.json`の`ellipsoid`)を用いる。

```
a  = 6378137.0                      // 長半径
f  = 1 / 298.257222101              // 扁平率
e2 = f * (2 - f)                    // 離心率の2乗

// 1) 測地座標 → ECEF
N(lat) = a / sqrt(1 - e2 * sin(lat)^2)
X = (N(lat) + h) * cos(lat) * cos(lon)
Y = (N(lat) + h) * cos(lat) * sin(lon)
Z = (N(lat) * (1 - e2) + h) * sin(lat)

// 2) 原点(lat0, lon0, h0=0)のECEFを基準に差分を取り、原点でのローカル座標系に回転(ENU)
dX = X - X0,  dY = Y - Y0,  dZ = Z - Z0

East  = -sin(lon0)*dX + cos(lon0)*dY
North = -sin(lat0)*cos(lon0)*dX - sin(lat0)*sin(lon0)*dY + cos(lat0)*dZ
Up    =  cos(lat0)*cos(lon0)*dX + cos(lat0)*sin(lon0)*dY + sin(lat0)*dZ

// シミュレーション座標
sim_x = East
sim_y = North
sim_z = Up
```

- 高さの扱い: DSM値はEGM96ジオイド基準の海抜(orthometric height)であり、上式の`h`にそのまま使う
  (ジオイド高と楕円体高の差は本設計では無視する簡略化。地形の格子間隔が約540mあることを踏まえると
  実用上の影響は小さい)
- 地球の曲率は上式に自然に反映される(遠方の地点ほど`Up`が減少していく)。550km規模の広域では
  原点から離れるほど地表が「下に沈んで」見える効果が正しく表現される
- Rust実装は `sim_frontend/src/terrain/mesh.rs` の `EnuTransform` 構造体。C++側(サーバー)は
  座標変換自体を行わず、緯度経度のみを状態として保持する(5.3節参照)。

### 3.3 計算をどこで行うか

- **前処理(C++/GDAL)側では行わない**。`heightmap.bin`/`metadata.json`は原点非依存の生データのまま
- **フロント側(Rust/WASM)がメッシュ構築時・原点変更時にランタイムで計算する**。1024×1024≈105万頂点分の
  変換は原点ごとに1回で済み、WASM上でも軽量
- 原点を変更したら、地形メッシュの頂点バッファを再計算・再アップロードする
  (`heightmap.bin`/`metadata.json`の再フェッチは不要)

### 3.4 原点の変更タイミングとガード

原点はシミュレーション座標系の定義そのものであり、シミュレーション実行中に変更すると
C++側・フロント側双方の状態(位置、地形メッシュ)がずれるリスクがある。

- **シミュレーション開始前(停止中)のみ原点変更可能**とし、実行中はUIの原点入力をロックする
- 原点はサーバー(C++側)が正とする状態であり、UIはサーバーから配信された値を表示・編集する
- C++側は原点についてUIとは別の内部表現を持たない。サーバーが保持する原点state
  (`OriginState`として配信される値)を唯一の真実とする

### 3.5 原点入力のバリデーション

- UIの原点入力フォームは、`metadata.json`の`geodetic_bounds`の範囲を入力可能な値の上下限として使い、
  範囲外の値は**入力欄への入力段階でブロックする**か、送信ボタンを無効化する
- サーバー側でも同じ範囲チェックを行う(フロントのバリデーションを回避するクライアントに対する防御的
  チェック)。範囲外の`set_origin`が送られてきた場合は`CommandError`を返す

### 3.6 原点状態の状態遷移図

```mermaid
stateDiagram-v2
    [*] --> Stopped: 起動(origin=default_origin, running=false)
    Stopped --> Stopped: set_origin(範囲内) → OriginState再配信
    Stopped --> Running: resume
    Running --> Running: set_origin(拒否) → CommandError
    Running --> Stopped: pause
```

---

## 4. 通信プロトコル詳細

### 4.1 メッセージフレーミング

serdeの`tag`機能をC++側で素朴に再現するのは実装コストが高いため、**先頭1バイトをメッセージタイプ
識別子、残りをMessagePackボディとする自前フレーミング**を採用する(サーバー→クライアント方向のみ)。

```
[1 byte: msg_type] [N bytes: MessagePack body]
```

クライアント→サーバー方向(`ClientCommand`)はメッセージ型が1種類のみのため、プレフィックスバイトを
付けず、MessagePackボディのみを送信する。

msgpackへのシリアライズは、Rust側(rmp-serde)・C++側(msgpack-cxxの`MSGPACK_DEFINE`)ともに
**構造体を配列(フィールド宣言順の位置)としてエンコードする**方式を採る(mapではない)。したがって
両言語の構造体定義は**フィールド宣言順を完全に一致させる必要がある**(片方だけ並び替えるとデータが
壊れる)。

### 4.2 msg_type 一覧

| 値 | 名前 | 方向 | 送信タイミング |
|---|---|---|---|
| 0x01 | SimState | Server→Client | 高頻度(約60Hz) |
| 0x02 | VabConfig | Server→Client | 状態変化時、接続直後にも1回 |
| 0x03 | OriginState | Server→Client | 状態変化時、接続直後にも1回、全クライアントへbroadcast |
| 0x04 | StatusPanelConfig | Server→Client | 状態変化時、接続直後にも1回 |
| 0x05 | CommandError | Server→Client | コマンド拒否時。**要求元クライアントのみ**に送信 |
| 0x06 | AppStatus | Server→Client | 状態変化時(pause/resume)、接続直後にも1回、全クライアントへbroadcast |
| (なし) | ClientCommand | Client→Server | ユーザー操作時 |

### 4.3 メッセージ型定義

**SimState**(サーバー→クライアント、高頻度)
```
t: f64                      // シミュレーション時刻(秒)。running中のみ進む
positions: Vec<f32>         // ダミーの単一点位置([x, y, z])。実際の可視化対象は将来拡張
frame_id: u32               // フレーム番号。running状態に関わらず毎ステップ増加
status_values: Vec<f64>     // StatusPanelConfig.items と同じ順序・同じ数
```

**VabConfig**(サーバー→クライアント、状態変化時のみ)
```
rows: u32
cols: u32
buttons: Vec<VabButton>
  VabButton:
    id: String
    label: String            // 空文字列 = 「未使用の穴」(6.2節)
    enabled: bool
```

**OriginState**(サーバー→クライアント、状態変化時+接続直後)
```
lat_deg: f64
lon_deg: f64
```

**StatusPanelConfig**(サーバー→クライアント、状態変化時+接続直後)
```
items: Vec<StatusItem>
  StatusItem:
    id: String
    label: String
    unit: String              // 単位。なければ空文字列
```

**CommandError**(サーバー→クライアント、要求元のみ)
```
command_type: String          // 拒否されたClientCommand.type
message: String                // エラー内容(人間可読)
```

**AppStatus**(サーバー→クライアント、状態変化時+接続直後。7.7節)
```
text: String                   // シミュレータアプリケーション自体の状態文字列
                                // (例: "シミュレーション実行中" / "一時停止中")
```

**ClientCommand**(クライアント→サーバー)
```
type: String                   // "vab_press" / "pause" / "resume" / "set_param" / "set_origin"
button_id: String              // vab_press時のみ使用
value: f64                     // set_param時のみ使用
lat_deg: f64                   // set_origin時のみ使用
lon_deg: f64                   // set_origin時のみ使用
```

### 4.4 送信頻度

- シミュレーションループ(simスレッド)は約60Hz(16ms間隔)で駆動する
- `VabConfig`/`OriginState`/`StatusPanelConfig`は変化があったときのみ送信(毎フレーム送らない)

### 4.5 WebSocket再接続処理(フロント側)

- 再接続間隔は**指数バックオフ**(初回1秒、以後2倍ずつ、上限30秒でキャップ)
- バックオフ間隔に**ジッター(±300ms)**を加える(サーバー再起動時のサンダリングハード回避)
- **ブラウザタブが非表示の間は再接続の試行を一時停止**する(Page Visibility API)。タブがアクティブに
  戻ったタイミングで即座に再接続を再開する(バックオフの残り時間を待たない)
- リトライ回数の上限は設けない(タブが表示されている間は無制限にリトライ)
- 再接続成功後は、サーバーから`OriginState`/`VabConfig`/`StatusPanelConfig`が接続直後の仕様により
  再送されるため、フロント側の表示状態は自然に復旧する

### 4.6 プロトコルのクラス図

```mermaid
classDiagram
    class MsgType {
        <<enumeration>>
        SimState = 0x01
        VabConfig = 0x02
        OriginState = 0x03
        StatusPanelConfig = 0x04
        CommandError = 0x05
    }
    class SimState {
        +f64 t
        +Vec~f32~ positions
        +u32 frame_id
        +Vec~f64~ status_values
    }
    class VabConfig {
        +u32 rows
        +u32 cols
        +Vec~VabButton~ buttons
    }
    class VabButton {
        +String id
        +String label
        +bool enabled
    }
    class OriginState {
        +f64 lat_deg
        +f64 lon_deg
    }
    class StatusPanelConfig {
        +Vec~StatusItem~ items
    }
    class StatusItem {
        +String id
        +String label
        +String unit
    }
    class CommandError {
        +String command_type
        +String message
    }
    class ClientCommand {
        +String type
        +String button_id
        +f64 value
        +f64 lat_deg
        +f64 lon_deg
    }
    VabConfig "1" *-- "many" VabButton
    StatusPanelConfig "1" *-- "many" StatusItem
```

### 4.7 シーケンス図: 接続確立

```mermaid
sequenceDiagram
    participant C as Client(Rust/WASM)
    participant WS as WsServer(uWSスレッド)
    participant Sim as Simulation(simスレッド)

    C->>WS: WebSocket接続 (ws://.../sim)
    WS->>WS: client_idを採番、clientsマップへ登録
    WS->>Sim: snapshot_origin()
    WS-->>C: OriginState (0x03)
    WS->>Sim: vab_config()
    WS-->>C: VabConfig (0x02)
    WS->>Sim: status_panel_config()
    WS-->>C: StatusPanelConfig (0x04)
    loop 約60Hz
        Sim->>Sim: step(dt)
        Sim-->>WS: SimulationTickResult
        WS-->>C: SimState (0x01, Loop::defer経由)
    end
```

### 4.8 シーケンス図: set_origin(成功/拒否)

```mermaid
sequenceDiagram
    participant C as Client
    participant WS as WsServer(uWSスレッド)
    participant Sim as Simulation(simスレッド)
    participant All as 他の全クライアント

    C->>WS: ClientCommand{type:"set_origin", lat, lon}
    WS->>Sim: enqueue_command(client_id, cmd)
    Note over Sim: 次のstep()呼び出し時にキューを消費

    alt シミュレーション実行中(running=true)
        Sim->>Sim: apply_set_origin() → 拒否
        Sim-->>WS: OutgoingCommandError
        WS-->>C: CommandError (0x05, 要求元のみ)
    else geodetic_bounds範囲外
        Sim->>Sim: apply_set_origin() → 拒否
        Sim-->>WS: OutgoingCommandError
        WS-->>C: CommandError (0x05, 要求元のみ)
    else 停止中 かつ 範囲内
        Sim->>Sim: origin_を更新
        Sim-->>WS: origin_changed = true
        WS-->>C: OriginState (0x03, broadcast)
        WS-->>All: OriginState (0x03, broadcast)
    end
```

### 4.9 シーケンス図: VABボタン押下

```mermaid
sequenceDiagram
    participant U as ユーザー
    participant V as Vabコンポーネント(Rust)
    participant WS as WsServer
    participant Sim as Simulation

    U->>V: クリック(有効なボタン)
    V->>V: ClientCommand::vab_press(button_id)
    V->>WS: WebSocket送信(msgpack, プレフィックスなし)
    WS->>Sim: enqueue_command(client_id, cmd)
    Sim->>Sim: step()内でapply_command()<br/>type=="vab_press" → ログ出力(v1はダミー、実アクチュエーション対象なし)
```

---

## 5. C++側詳細設計

### 5.1 スレッドモデル

- **uWSイベントループスレッド**: `WsServer::run()`を呼んだスレッド。HTTP/WebSocketの送受信を担当。
  `uWS::App`はシングルスレッド前提のため、このスレッド以外から`ws->send()`を直接呼んではならない
- **simスレッド**: `WsServer::run()`内で`std::thread`として起動。`Simulation::step()`を約60Hz
  (16ms間隔)で呼び続ける
- simスレッドからuWSスレッドへ処理を戻す(実際の送信を行わせる)には、必ず`uWS::Loop::defer()`を
  経由する

```mermaid
flowchart LR
    subgraph uWSスレッド
        A[".messageハンドラ"] -->|enqueue_command| B[(コマンドキュー\nSimulation内)]
        F["Loop::defer()で受け取ったコールバック"] --> G["ws->send()"]
    end
    subgraph simスレッド
        C["Simulation::step(dt)"] -->|キューを消費| B
        C --> D["SimulationTickResult"]
        D -->|Loop::defer経由| F
    end
```

### 5.2 コマンド処理フロー

- `.message`ハンドラで受信したコマンドは**直接シミュレーション状態を書き換えず**、
  `Simulation::enqueue_command()`でスレッドセーフなキュー(`std::deque` + `std::mutex`)に積む
- simスレッド側で`step()`の**前半**でキューを消費してから、物理状態(`t_`等)を更新する
- `step()`の戻り値`SimulationTickResult`に、そのステップで発生した`CommandError`と
  `origin_changed`フラグが入っており、uWSスレッド側がこれを見て適切な送信(broadcast/単一送信)を行う

### 5.3 クラス図

```mermaid
classDiagram
    class WsServer {
        -Impl* impl_
        +WsServer(port: uint16_t)
        +~WsServer()
        +run()
    }
    class WsServerImpl {
        -uint16_t port
        -Simulation simulation
        -unordered_map~ClientId,ServerWebSocket*~ clients
        -mutex clients_mutex
        -atomic~ClientId~ next_client_id
        -thread sim_thread
        -atomic~bool~ keep_running
        +broadcast(frame)
        +send_to_client(client_id, frame)
    }
    class Simulation {
        -mutex state_mutex_
        -OriginState origin_
        -bool running_
        -double t_
        -uint32_t frame_id_
        -VabConfig vab_config_
        -StatusPanelConfig status_panel_config_
        -mutex queue_mutex_
        -deque~QueuedCommand~ command_queue_
        +enqueue_command(client_id, cmd)
        +step(dt) SimulationTickResult
        +snapshot_sim_state() SimState
        +snapshot_origin() OriginState
        +vab_config() VabConfig
        +status_panel_config() StatusPanelConfig
        -apply_queued_commands()
        -apply_command(cmd, client_id)
        -apply_set_origin(cmd, client_id)
    }
    class QueuedCommand {
        +ClientId client_id
        +ClientCommand cmd
    }
    class SimulationTickResult {
        +vector~OutgoingCommandError~ errors
        +bool origin_changed
    }
    class OutgoingCommandError {
        +ClientId client_id
        +CommandError error
    }

    WsServer o-- WsServerImpl
    WsServerImpl *-- Simulation
    Simulation ..> QueuedCommand : キューに積む
    Simulation ..> SimulationTickResult : step()の戻り値
    SimulationTickResult *-- OutgoingCommandError
```

### 5.4 HTTP静的配信(地形データ)

`sim_server`は`/sim`(WebSocket)とは別に、以下のHTTP GETルートを持つ:

| パス | Content-Type | 内容 |
|---|---|---|
| `GET /terrain/metadata.json` | application/json | `assets/terrain/metadata.json`をそのまま返す |
| `GET /terrain/heightmap.bin` | application/octet-stream | `assets/terrain/heightmap.bin`をそのまま返す |

フロント(trunk serveでホストされる別オリジン)からfetchされるため、両ルートとも
`Access-Control-Allow-Origin: *`ヘッダーを付与する。想定CWD(カレントディレクトリ)は`sim_server/`
(`assets/terrain/...`という相対パスでファイルを開くため)。

### 5.5 GeoTIFF前処理ツール(geotiff_preprocess)のクラス構成

`tools/geotiff_preprocess/main.cpp`に実装(単一ファイル、`sim_server`本体とは別実行ファイル)。

| 関数/構造体 | 役割 |
|---|---|
| `TileId` | ファイル名から抽出した`(lat, lon)`整数度 |
| `parse_tile_filename()` | `"ALPSMLC30_N035E138_DSM.tif"` → `TileId{35, 138}` |
| `DiscoveredTile` | `TileId` + ファイルパス(まだGDAL読み込みはしていない) |
| `discover_tiles()` | map_dataディレクトリをglobし、ファイル名からタイルIDだけを列挙(1段階目) |
| `MosaicBounds` | モザイクの外接矩形(整数度)。ハードコードではなく実行時に計算する値 |
| `compute_mosaic_bounds()` | 見つかった全タイルIDの緯度・経度の最小/最大から外接矩形を求める |
| `SourceGrid` | GDALで読み込んだ1タイル分の標高グリッド + 地理範囲 |
| `load_sea_mask()` | 対応する`*_MSK.tif`を読み、画素値3(海)の位置をビットマップで返す(1.4節) |
| `load_dsm_tile()` | GDALでGeoTIFFを1枚読み込む(再投影しない)。NODATA画素+マスク海域画素をNaNへ置換 |
| `build_mosaic()` | 外接矩形サイズのキャンバスへ各タイルを配置(2段階目) |
| `downsample_average()` | 平均法によるダウンサンプリング(NaNは平均から除外) |
| `flip_rows_north_to_south()` | GDAL標準の行順(北→南)を南→北へ反転(2.5節の注意参照) |
| `write_heightmap_bin()` / `write_metadata_json()` | 出力ファイル書き出し |

---

## 6. Rust側詳細設計

### 6.1 コンポーネント構成図

```mermaid
graph TD
    App["App (app.rs)<br/>3カラムCSS Gridレイアウト・リサイザー"]
    App --> SimulationStatusPanel["SimulationStatusPanel<br/>(operation_panel.rs) 接続状態・原点・フレーム表示"]
    App --> VabPanel["VabPanel<br/>(vab.rs) 4列ボタングリッド(1+4+1行)"]
    App --> MainPanel["MainPanel<br/>(main_panel.rs) 地形描画canvas(3D, TerrainView)"]
    App --> TopStatusPanel["TopStatusPanel<br/>TabbedPanel: [各種情報]タブ=StatusPanel"]
    App --> BottomStatusPanel["BottomStatusPanel<br/>TabbedPanel: [断面図]=CrossSectionView, [見通し範囲]=LosView"]

    App -.provide_context.-> WsSignals["WsSignals<br/>(接続状態・受信データのシグナル群)"]
    App -.provide_context.-> TerrainStore["TerrainStore<br/>(heightmap/metadataを両パネルで共有)"]
    App -.provide_context.-> RadarMarkersState["RadarMarkersState<br/>(観測点一覧・選択状態、全パネル共有)"]
    App -.propとして渡す.-> WsConnection["WsConnection<br/>(Rc<RefCell<...>>、Send/Sync境界回避のためcontext不使用)"]

    MainPanel --> Loader["terrain::loader<br/>heightmap.bin/metadata.json取得"]
    MainPanel --> Mesh["terrain::mesh<br/>ENU変換・頂点/インデックス生成"]
    MainPanel --> Renderer["terrain::renderer::TerrainRenderer<br/>wgpu Device/Queue/Pipeline(地形)+line_pipeline(観測点/覆域)"]
    MainPanel --> Camera["terrain::camera::Camera<br/>view_proj行列・screen_to_ray"]
    MainPanel --> Pick["terrain::pick::pick_lat_lon<br/>右クリック→レイキャストで緯度経度取得"]
    MainPanel --> Markers["terrain::markers::build_marker_geometry<br/>観測点・覆域リングの3D頂点生成"]
    BottomStatusPanel --> Profile["terrain::profile::build_profile<br/>原点から方位角方向へ地表をサンプリング"]
    BottomStatusPanel --> Los["terrain::los::compute_los / is_visible<br/>全方位角の見通し限界距離・点対点の遮蔽判定"]
    TerrainStore -.共有データ.-> MainPanel
    TerrainStore -.共有データ.-> BottomStatusPanel
    RadarMarkersState -.共有データ.-> MainPanel
    RadarMarkersState -.共有データ.-> BottomStatusPanel
```

### 6.2 WebSocket接続管理のクラス図

```mermaid
classDiagram
    class WsSignals {
        +RwSignal~ConnectionStatus~ status
        +RwSignal~Option~OriginState~~ origin
        +RwSignal~Option~VabConfig~~ vab_config
        +RwSignal~Option~StatusPanelConfig~~ status_panel_config
        +RwSignal~Option~SimState~~ last_sim_state
        +RwSignal~Option~CommandError~~ last_command_error
    }
    class WsConnection {
        -WsSignals signals
        -Rc~RefCell~Inner~~ inner
        +connect_new(url, signals) WsConnection
        +send_command(cmd)
        -open_socket()
        -handle_message(event)
        -schedule_reconnect()
        -setup_visibility_listener()
    }
    class Inner {
        +String url
        +Option~WebSocket~ socket
        +u32 reconnect_attempt
        +bool tab_visible
        +Option~Timeout~ reconnect_timeout
    }
    class ConnectionStatus {
        <<enumeration>>
        Connecting
        Connected
        Reconnecting(attempt: u32)
        PausedHidden
    }

    WsConnection o-- WsSignals
    WsConnection o-- Inner
    WsSignals --> ConnectionStatus
```

`WsConnection`は`Rc<RefCell<Inner>>`を内部に持ちSend/Syncではないため、Leptos 0.8の
`provide_context`(Send+Sync境界を要求する)には乗せられない。そのため`WsSignals`はcontext経由、
`WsConnection`はコンポーネントのpropとして明示的に渡す設計とした
(`WsConnection`自体には`unsafe impl Send/Sync`を付与している。`wasm32-unknown-unknown`は
シングルスレッドのため実質的に安全)。

### 6.3 再接続状態遷移図

```mermaid
stateDiagram-v2
    [*] --> Connecting
    Connecting --> Connected: onopen
    Connecting --> Reconnecting: onclose/onerror (タブ表示中)
    Connected --> Reconnecting: onclose/onerror (タブ表示中)
    Connected --> PausedHidden: タブが非表示になる
    Reconnecting --> Connected: 再接続成功(onopen)
    Reconnecting --> Reconnecting: 再接続失敗(指数バックオフ+ジッターで再試行)
    Reconnecting --> PausedHidden: タブが非表示になる(保留中のタイマーを破棄)
    PausedHidden --> Reconnecting: タブが表示に戻る(即座に再接続を試行)
```

### 6.4 地形描画パイプライン

```mermaid
flowchart LR
    A["loader::load_terrain()<br/>metadata.json + heightmap.bin をfetch"] --> B["mesh::build_mesh()<br/>EnuTransformで各グリッド点をENU変換"]
    B --> C["TerrainVertex配列 + インデックス配列"]
    C --> D["TerrainRenderer::new()<br/>wgpu Instance/Adapter/Device/Surface初期化"]
    D --> E["頂点/インデックスバッファへアップロード"]
    E --> F["TerrainRenderer::render(camera)<br/>Camera::view_proj_matrix()をuniformへ書き込み"]
    F --> G["canvas(WebGPU)に描画"]
```

### 6.5 地形描画クラス図

```mermaid
classDiagram
    class TerrainMetadata {
        +u32 width
        +u32 height
        +f32 elevation_min
        +f32 elevation_max
        +GeodeticBounds geodetic_bounds
        +Ellipsoid ellipsoid
        +bool has_texture
        +DefaultOrigin default_origin
    }
    class TerrainData {
        +TerrainMetadata metadata
        +Vec~f32~ heightmap
    }
    class EnuTransform {
        -f64 origin_lat_rad
        -f64 origin_lon_rad
        -f64 origin_x
        -f64 origin_y
        -f64 origin_z
        -f64 a
        -f64 e2
        +new(origin, ellipsoid) EnuTransform
        +transform(lat_deg, lon_deg, h) [f32; 3]
    }
    class TerrainVertex {
        +[f32; 3] position
        +[f32; 3] color
    }
    class TerrainMesh {
        +Vec~TerrainVertex~ vertices
        +Vec~u32~ indices
    }
    class Camera {
        +Vec3 eye
        +Vec3 target
        +f32 fov_y_radians
        +f32 aspect
        +f32 z_near
        +f32 z_far
        +view_proj_matrix() Mat4
    }
    class TerrainRenderer {
        -Surface surface
        -Device device
        -Queue queue
        -SurfaceConfiguration config
        -RenderPipeline pipeline
        -Buffer vertex_buffer
        -Buffer index_buffer
        -Buffer camera_buffer
        -BindGroup camera_bind_group
        -TextureView depth_view
        +new(canvas, mesh) TerrainRenderer
        +render(camera) Result
        +resize(width, height)
        +aspect_ratio() f32
    }

    TerrainData *-- TerrainMetadata
    TerrainMesh *-- TerrainVertex
    TerrainRenderer ..> TerrainMesh : 構築時に頂点バッファへコピー
    TerrainRenderer ..> Camera : renderで受け取る
    EnuTransform ..> TerrainVertex : transform()の結果をpositionへ
```

### 6.6 座標軸の扱い(ENU→wgpu)

ENU座標系(東=X, 北=Y, 上=Z)は右手系(East×North=Up)。wgpu/glamの一般的な慣習であるY-upとは
軸の意味が異なるが、**頂点データ自体は変換せず**、カメラのビュー行列側で吸収する:

- `Camera::view_proj_matrix()`は`glam::Mat4::look_at_rh(eye, target, Vec3::Z)`のように、
  up方向として`Vec3::Z`を明示的に渡す
- 射影行列は`Mat4::perspective_rh`(wgpu/Vulkan/Metal互換の深度[0,1])を使う。OpenGL互換の
  `perspective_rh_gl`(深度[-1,1])は使わない
- ENUがそもそも右手系であるため、`_rh`系の関数とそのまま整合し、軸の入れ替えによる
  ハンドネス反転を気にする必要がない

### 6.7 標高グラデーション配色

標高を`elevation_min`〜`elevation_max`で正規化し、以下のカラーストップで線形補間する:

| 正規化標高 t | 色(RGB, 0〜1) | 用途 |
|---|---|---|
| 0.0 | (0.05, 0.35, 0.35) | 低地: 深緑がかった青緑 |
| 0.25 | (0.15, 0.5, 0.2) | 緑 |
| 0.5 | (0.55, 0.5, 0.25) | 黄土色 |
| 0.75 | (0.45, 0.32, 0.22) | 茶 |
| 1.0 | (0.95, 0.95, 0.95) | 山頂付近: 白に近い明色 |

**海(データなし)の塗り分け**: `heightmap`の値が`NaN`の格子点(1.4節: 存在しないタイル +
各タイル内のNODATA画素 + マスクファイルが海と示す画素)は、上記のグラデーション計算を行わず
固定色`WATER_COLOR = (0.55, 0.78, 0.92)`(水色)を使う。頂点position自体はNaNだと破綻するため、
標高0mの平面として配置する(`terrain/mesh.rs::build_mesh`)。`terrain::mesh::sample_heightmap`
(カメラ注視点の高さ・断面図のサンプリングで共用)も同様に、双線形補間の結果がNaNなら標高0mへ
丸める。

**背景(実データ範囲の外側)**: メインパネルをズームアウト・回転すると、実データの外接矩形
(5°四方)の外側に出る。ここを「地平線から下は水色・空は黒」に見せるため、2つの仕組みを
組み合わせている(`terrain/mesh.rs::append_background_skirt` + `terrain/renderer.rs`のクリア色):

- `build_mesh()`が、実データの頂点群とは別に、`WATER_COLOR`で塗った巨大な水平の板
  (半径`BACKGROUND_HALF_SIZE_M = 4,000,000m`の正方形、標高は`elevation_min - 500m`)を
  同じ頂点/インデックスバッファの末尾に追加する。半径はカメラの最大ズームアウト距離・
  遠方クリップ距離(`camera.rs`のMAX_DISTANCE=500,000m/Z_FAR=1,500,000m)より十分大きく、
  どの角度・距離から見ても端が視野に入らない。標高を実データの最低標高より確実に低くして
  あるため、実データ(海面=標高0m付近を含む)は常にこの板より手前(深度が浅い)に描画され、
  Zファイティングは起きない
- 板にも覆われない領域(=真上に近い方向、空)は`TerrainRenderer::render()`のクリア色
  (黒 `wgpu::Color{0,0,0,1}`)がそのまま見える。板は別パイプラインではなく地形本体と同じ
  triangle-listパイプライン・同じ描画コールに乗るため、レンダラー側の変更はクリア色のみで済む

この仕組みにより、俯瞰プリセットでズームアウトすると外接矩形の外側全体が水色になり、
側面プリセット(ほぼ水平視点)では地平線を境に上=黒空・下=水色という自然な見た目になる。

### 6.8 頂点シェーダ・フラグメントシェーダ(WGSL概要)

`terrain.wgsl`。カメラのview_proj行列をuniformバッファ(`@group(0) @binding(0)`)として受け取り、
頂点位置を変換するのみのシンプルな構成(ライティング計算なし、頂点色をそのまま出力)。

```wgsl
struct CameraUniform { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> camera: CameraUniform;

struct VertexInput { @location(0) position: vec3<f32>, @location(1) color: vec3<f32> };
struct VertexOutput { @builtin(position) clip_position: vec4<f32>, @location(0) color: vec3<f32> };

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
```

### 6.9 断面図(ボトムステータスパネルの「断面図」タブ、`terrain::profile` / `CrossSectionView`)

ボトムステータスパネルは3D視点ではなく、**原点を起点に方位角スライダーで指定した方向の地表断面を
2D(距離 vs 標高の折れ線)で表示する**。wgpuは使わずSVGで描画するため、中央地図用の
`TerrainRenderer`とは完全に独立(GPUリソースを消費しない)。

```mermaid
flowchart LR
    A["スライダー操作<br/>azimuth: RwSignal&lt;f64&gt;(度、北=0・東=90・時計回り)"] --> B["profile::build_profile(data, origin, azimuth)"]
    B --> C["EnuTransform::inverse(east, north)<br/>ローカル接平面近似でENUオフセット→緯度経度"]
    C --> D["heightmapを双線形補間でサンプリング<br/>範囲外に出たら二分探索で打ち切り距離を確定"]
    D --> E["Vec&lt;ProfilePoint&gt;(distance_m, elevation_m)"]
    E --> F["CrossSectionView: SVG折れ線・塗りつぶしへ変換して描画"]
```

- 方位角0本につき301点(`NUM_SAMPLES=300`)を、原点から「地形データの範囲内にいられる
  最大距離」まで均等にサンプリングする。最大距離は二分探索(30回)で求める
  (`EnuTransform::inverse`で緯度経度に変換し、`geodetic_bounds`内かどうかを判定する)
- `EnuTransform::inverse`は`transform`(緯度経度→ENU)の近似逆変換。原点緯度における
  子午線曲率半径Mと卯酉線曲率半径Nを使ったローカル接平面近似(`lat = lat0 + north/M`,
  `lon = lon0 + east/(N・cos(lat0))`)
- heightmapの双線形補間サンプリング(`mesh::sample_heightmap`)は断面図専用ではなく
  `terrain/mesh.rs`にある共通関数。メインパネルのカメラ注視点の高さ算出(6.5節注記・
  下記コードレビュー節参照)にも使う
- 原点(`WsSignals.origin`)・方位角(`azimuth`)のどちらが変わってもLeptosの反応性により
  自動的に再計算・再描画される(明示的なEffectは不要、`chart`クロージャ内で両方を`.get()`
  しているため)

### 6.10 見通し範囲(レーダー観測点、`terrain::los` / `terrain::markers` / `terrain::pick` / `LosView`)

任意の地点(レーダー観測点)を観測点として、全方位角(1度刻み・360方向)の見通し限界距離を
計算し、(a) メインパネル(3D地形)上に覆域境界の輪郭線、(b) ボトムステータスパネルの
「見通し範囲」タブに2D極座標図(レーダー覆域図)、(c) 断面図タブに覆域区間の色分け、の3か所で
表示する。観測点自体は**メインパネル上での右クリックで追加する**(タブからの手入力ではない)。

**観測点の追加(右クリック→レイキャスト)**: メインパネルのcanvasで右クリックすると
(ブラウザ既定のコンテキストメニューは`prevent_default`で抑止)、クリックされた画面座標から
カメラの逆ビュー射影行列でENU座標系のレイを求め(`Camera::screen_to_ray`)、そのレイを
CPU側のheightmapに対して直接マーチングして地表との交点を探す(`terrain::pick::pick_lat_lon`。
GPU側の読み戻しは行わない)。交点が見つかり、かつ地形データ範囲内であれば、その緯度経度に
既定パラメータ(アンテナ高10m・最大観測範囲50km)の観測点(`RadarMarker`)を追加し、選択状態にする。
観測点は**複数個**追加でき、一覧は`ui_state::RadarMarkersState`(全パネル共有のcontext)が保持する。

**選択・削除・パラメータ編集**: ボトムステータスパネルの「見通し範囲」タブが観測点一覧を
リスト表示する。各行をクリックすると選択状態になり(3D側のハイライト色・極座標図の対象が
連動して切り替わる)、行内の入力欄でアンテナ高・最大観測範囲をその場で編集でき、「削除」
ボタンで一覧から取り除ける(選択中の観測点を削除すると選択状態はクリアされる)。

**3D描画(メインパネル)**: マーカー本体(四角い枠)と覆域ドームは、頂点データ・
描画パイプラインとも別々に扱う(下記)。原点変更時(メッシュ再構築後)・観測点の追加/削除/
選択変更のたびに両方の頂点データを作り直す(`components/terrain_view.rs::rebuild_markers`)。

- マーカー本体: `terrain::markers::build_marker_geometry`が観測点一覧・選択状態から
  LineList用の頂点列を作り、`TerrainRenderer`の`line_pipeline`(地形本体の`pipeline`とは
  別だが、頂点レイアウト・カメラバインドグループ・シェーダーは共用)で描画する。各観測点は
  地表にわずかに(25m)浮かせた四角い枠として描く(選択中は黄色、非選択はオレンジ)
- 覆域ドーム: `terrain::markers::build_dome_surface_geometry`が**選択中の観測点についてのみ**
  (複数観測点の覆域を同時に重ねると見づらいため)TriangleList用の頂点列を作り、専用の
  `dome_pipeline`(アルファブレンド有効・深度書き込み無効。地形やマーカーの奥に半透明で
  透けて見えるようにするため)で描画する

**覆域の3D表示は半球状の面(Surfaceを持つ多面体)**(「ワイヤーフレームではなくSurfaceが
存在する多面体に」という要望により、ワイヤーフレーム表現から変更): `push_dome_surface`が、
`terrain::los::compute_los_dome`(下記)の出力を使い、仰角0°〜80°
(`DOME_RING_ELEVATIONS_DEG = [0,5,10,20,35,55,80]`、地表付近をやや密に)の7段の緯度リング
(計算自体は各360分割)を隣接するリング×隣接する方位角ごとに四角形パッチ(三角形2枚)で
つないで球面状の面を作る。最上段リング(仰角ごとに半径が異なり単一の頂点には収束しない)は、
その高さの平均をアペックス(頂点)として傘状の三角形群で閉じ、開いた穴のない多面体にする。
色は半透明の水色固定(`terrain.wgsl`の`fs_dome`エントリポイントでアルファ0.22を出力)。

三角形化に使う方位角は`DOME_MESH_AZIMUTH_STRIDE`(10)で間引き、360分割→36分割にしている
(`compute_los_dome`自体の計算解像度はそのまま)。間引かずに360分割全てを三角形化すると、
アペックスへ閉じる傘の部分に極端に細い三角形が360枚重なり、アルファブレンド(半透明合成)が
描画順に依存するためカメラ角度が変わるたびに重なり方が変化して明るさがちらつく
(「画面を操作するとマップがチカチカする」との報告を受けて特定・修正)。また、地形に
遮蔽される方角ではドーム境界が定義上ちょうど地形の表面に接するため、そのままだと地形
メッシュとのZファイティングも起きる。これは`DOME_HEIGHT_BIAS_M`(20m)でドーム全体を
一律に持ち上げて回避している。

**`compute_los_dome`は「地形に遮蔽されない方角ではどの仰角でも同じ半径(=滑らかな球面)、
地表付近だけ地形の遮蔽で内側に凹む」という物理的に正しいモデル**(「上方向は、地表面に
よってさえぎられているところ以外は滑らかな球面になるのでは」との指摘を受けて修正。
当初案は`compute_los`(仰角0°の値のみ)をそのまま全仰角のドーム半径として使い回す簡易的な
extrudeだったが、これだと地形に遮蔽されない方角でも仰角によって半径が変わらず、指摘の
通り物理的に不自然だった)。仰角ごとに、観測点からの**直線(仰角一定のレイ)**が地形に
遮蔽されずどこまで届くかを、`compute_los`と同じサンプリングループを複数の仰角しきい値
(`tan`)に対して同時に評価することで求める。直線は一度地形にぶつかったら(直線である以上)
その先で地形が下がっても二度と地形の陰から出てこないため、`compute_los`の「手前の尾根の
陰でも先で再び見える」処理とは異なり「最初に遮蔽された時点で打ち切り」が正しい(これは
簡略化ではなく、レイが地表を這うcompute_losとは異なる幾何学的設定であることによる)。
地形に遮蔽されない方角では、指定した最大観測範囲(スラントレンジ)までそのまま届く。

**観測点は地形メッシュの原点(`OriginState`)とは独立**: 緯度経度の絶対値で保持しており、
`set_origin`コマンドは一切送らない。原点(メッシュ)が変わっても、観測点自体の緯度経度は
変わらず、描画側が新しい原点基準のENU座標へ再変換するだけ。地形メッシュの再計算も発生しない
ため、原点変更とは異なり**シミュレーション実行中でも自由に追加・編集できる**(3.4節の
「原点変更はシミュレーション停止中のみ」という制約は`OriginState`自体の変更にのみ適用され、
観測点には適用されない)。

```mermaid
flowchart LR
    A["メインパネル右クリック<br/>(screen x,y)"] --> B["Camera::screen_to_ray<br/>ENU座標系のレイ"]
    B --> C["pick::pick_lat_lon<br/>heightmapに対してレイマーチング"]
    C --> D["RadarMarkersState::add<br/>(lat,lon)→RadarMarker追加・選択"]
    D --> E["los::compute_los(data, marker, params)"]
    E --> F["markers::build_marker_geometry<br/>LineList頂点(枠+選択中のみ覆域リング)"]
    F --> G["TerrainRenderer::update_markers<br/>→ line_pipelineで描画"]
    E --> H["LosView: SVGの極座標図へ変換して描画"]
```

**等価地球半径(equivalent earth radius)**: 標準大気中では電波が幾何学的な直線よりわずかに
下向きに屈折するため、実際の見通し距離は真球上の幾何学的な地平線より遠くなる。この効果を
「実際の地球半径(平均6,371km)をk倍した仮想的に大きい球面上で電波が直進する」近似で表した
ものが等価地球半径で、標準大気ではk=4/3が広く使われる(レーダー・無線工学の標準的な近似、
`terrain/los.rs::K_FACTOR`に定数として実装。v1では固定値で、UIパラメータ化はしていない)。
距離`d`における地球曲率分の見かけの高度低下量は`d² / (2 * R_eff)`(`R_eff = R_earth * k`)。

**遮蔽判定(マスク角アルゴリズム)**: 方位角ごとに観測点から外側へサンプリングしながら、
各サンプル点の「見かけの仰角」(等価地球半径による曲率低下を差し引いた角度)を計算する。
それまでの最大仰角を`max_angle`として保持し、`angle(d) >= max_angle`を満たす点だけを
「観測点から直接見える点」とみなす(満たさない点は、より手前にある高い地形に遮蔽されて
見えない)。その方位角の見通し限界距離は、この条件を満たした点のうち最も遠いものの距離とする
(手前の尾根の陰でも、その先で地形が十分高くなれば再び見えるケースを許容する。単純な
「最初の遮蔽物で打ち切り」より実際のレーダー覆域に近い近似)。

**パラメータ**:
- 観測点位置(緯度・経度): メインパネル上の右クリック位置から`pick_lat_lon`で決まる
  (`RadarMarker::lat_deg/lon_deg`)。タブ側では読み取り専用表示のみで編集はできない
- アンテナ高(`RadarMarker::height_m`): 観測点の地表(heightmapから取得)からの高さ。
  一覧の行内で編集可、既定10m
- 最大観測範囲(`RadarMarker::max_range_m`): この距離とデータ範囲内の距離の小さい方までを
  計算対象とする。一覧の行内でkm単位表示・編集(内部ではmで保持)、既定50km

**計算コスト**: 観測点1つあたり360方位角 × 200サンプル/方位角 = 72,000回の`heightmap`双線形補間
(3D描画・極座標図とも選択中の観測点のみ計算するため、観測点の総数には比例しない)。
断面図の覆域表示は別経路で、`terrain::los::is_visible`(単一方位への点対点遮蔽判定)を
断面上の各点×配置済み観測点の数だけ呼ぶ(301点×観測点数、1回あたり最大200サンプル)。
いずれもブラウザで実測して体感遅延なく反応的に再計算できることを確認済み。

**断面図タブでの覆域表示**: 断面図(6.9節、`CrossSectionView`)に、地表トラックの覆域(従来通り)
と**上空を含めた覆域**(要望により追加)の2種類を重ねて描く。観測点が1つも配置されていなければ
どちらも描かない。

- 地表トラックの覆域: 断面上の各点が「配置済み観測点のいずれか1つからでも見える(=遮蔽
  されない)」区間だけをつないだ緑色のオーバーレイ線(`cs-coverage`)。判定は断面上の各点への
  **点対点の遮蔽判定**(`terrain::los::is_visible`。`compute_los`と同じマスク角アルゴリズムを
  対象点までの1本のレイに絞って適用)
- **上空を含めた覆域**: 断面上の各点(距離)ごとに「これ以上の高度(標高)なら、配置済み
  観測点のいずれかから見える」という下限高度を求め(`terrain::los::min_visible_altitude`。
  仰角の式が対象の高度について線形であることを利用し、`is_visible`のように対象の実標高と
  比較するのではなく、可視となる最小高度を直接逆算する)、その下限高度から表示上の上限
  (地表断面の最高標高+`SKY_MARGIN_M`=10,000m、v1では固定値)までを緑の半透明な塗りつぶし
  (`cs-airspace`)で示す。複数の観測点があるときは各点ごとの下限高度の最小値(=最も緩い
  条件、いずれか1つでも見えれば覆域内)を採用する。地表トラックの覆域(`is_visible`)より
  緩やかな条件になりうるため(例: 地表そのものはわずかに遮蔽されていても、少し上空なら
  見える)、上空を含めた覆域の方が地表トラックの覆域より広く出ることがある(実機で確認済み。
  地表トラックの緑線が途切れていても塗りつぶしだけは連続している区間がある)

---

## 7. UI詳細設計

### 7.1 レイアウト

```
┌──────────────────────────────────────────────────────────────┐
│ ファイル  設定  表示  ヘルプ                    ← メニューバー    │
├───────────────────────┬───────────────────┬───────────────────┤
│ シミュレーション          │                   │ トップステータスパネル │
│ ステータスパネル(上)      │                   │  [各種情報]タブ      │
├───────────────────────┤     メインパネル     ├───────────────────┤
│ VABパネル(下)            │    (3D地形)        │ ボトムステータスパネル │
│                         │                   │ [断面図][見通し範囲]  │
└───────────────────────┴───────────────────┴───────────────────┘
```

画面最上部にメニューバー(`components/menu_bar.rs`)を固定高さで配置し、その下に
既存の3カラムレイアウト(`.app-shell`)を残り高さいっぱいで敷く(`.app-root`が
`display:flex; flex-direction:column`で両者を縦に並べる)。

パネル名は全て位置ベースの汎用名で統一している(シミュレーションステータスパネル/VABパネル/
メインパネル/トップステータスパネル/ボトムステータスパネル)。表示内容そのものを指す旧称
(「操作パネル」「VAB」「地図」「各種情報パネル」「側面図パネル」)は、トップ/ボトムステータス
パネルではタブラベルとして残るのみで、パネル自体の名前としては使わない。

- CSS Gridで4列(左パネル固定 / メインパネル / リサイザー(6px) / 右パネル)を構成する
- 左パネルは`display:grid; grid-template-rows: 1fr auto;`で上下2分割
  (シミュレーションステータスパネル/VABパネル)。VABパネル側は`auto`で内容の高さに
  ぴったり合わせ、余った分はシミュレーションステータスパネル側(`1fr`)が吸収する
  (固定`1fr 1fr`だと、VABパネルの実寸と半分の高さがずれた際に一方に余白/スクロールが
  生じるため)
- 右パネルは`grid-template-rows: 1fr 1fr;`で上下2分割(トップ/ボトムステータスパネル、
  こちらは両方とも内容量の変動が小さいため固定分割のままでよい)。両パネルとも
  `TabbedPanel`(7.6節)で実装しており、現状は1タブのみだが後から同じ枠に別タブを追加できる
- 左パネル幅は`--panel-width`(CSS変数、既定320px)で固定
- 地図・右パネルの幅は`grid-template-columns`の`minmax(下限px, Nfr)`で指定し、`N`(fr値)を
  Leptosの`RwSignal<f64>`で保持する。ドラッグ量(スクリーン座標のpx)をそのままfr値に加減算する
  設計のため、fr値の初期値もpxスケールの数値にしておく(小さい値だと1回のドラッグで下限に
  張り付いてしまう不具合が実装時に発生し、修正済み)

### 7.2 レスポンシブ方式

- 3カラムの横並びレイアウトは崩さない(縦積みへの再レイアウトは行わない)
- 画面幅が狭くなった場合は、`minmax()`の`fr`部分により中央・右パネルの幅が比例的に縮小する
- `minmax()`の下限を下回る場合は、外側コンテナ(`.app-shell`)に`overflow-x: auto`を設定してあるため
  横スクロールで対応する
- 各パネル内(`.panel-section`)は`overflow-y: auto`で縦スクロールに対応する

### 7.3 リサイザー(splitter)の実装

- 地図・右パネルの境界にドラッグ可能な6px幅の要素を配置する
- `on:pointerdown`でドラッグ開始位置を記録し、`element.set_pointer_capture(pointer_id)`で
  ドラッグ中にカーソルが要素外に出てもイベントを受け取り続けるようにする
- `on:pointermove`でドラッグ量(dx)を計算し、center_fr/right_frシグナルを更新する
  (center_fr += dx, right_fr -= dx)
- `on:pointerup`/`on:pointercancel`でドラッグ終了

### 7.4 VAB仕様

- 開発用ダミー`VabConfig`の初期値: rows=6, cols=4(24ボタン)
- ボタンの操作種別: 単純クリックのみ
- ラベルが空文字の`VabButton`は「未使用の穴」として、**DOM要素自体を生成しない**
- 実装上の注意: 穴を`filter()`で除外すると自動配置(auto-placement)がずれるため、先頭行の
  各ボタンには`grid-row`/`grid-column`を配列インデックスから明示的に計算して指定する
  (`row = 1; col = i + 1;`。先頭行のみを扱うため`row`は常に1)

**カテゴリ選択タブ + 動的コンテンツ**(要望により追加): VabConfigの**先頭行(`cols`個)だけ**を
「カテゴリ選択タブ」として扱い、サーバーが決めたラベル・有効/無効のまま描画する(押すと従来
通り`vab_press`コマンドを送る)。**先頭行より後ろ(中段・下段、元のB5〜B24相当)は、サーバーの
VabConfigの内容を使わず、選択中カテゴリに応じて内容が切り替わるフロント側だけのダミー
コンテンツに置き換える**(`components/vab.rs`):

- 中段: `MID_ROWS`(4)行×`MID_TOTAL_COLS`(8)列のダミーボタン。1ページあたり`cols`列だけを表示し、
  残りは**「◀ 1/2 ▶」のページ送りボタン(`.vab-pager`)で切り替える**(横スクロールバー方式は
  「ボタンによってページを切り替える感じにしたい」との要望により不採用にした)。現在ページ
  (`current_page`、ローカル状態)×`cols`列目から`cols`個ぶんの列だけをグリッドに描画し、
  ◀/▶は端のページで無効化する。カテゴリ(先頭行)を切り替えたら現在ページは先頭に戻す
- 下段: `BOTTOM_COLS`(4)列の固定ダミーボタン(横スクロールなし)
- ラベルは選択中カテゴリの先頭行ボタンのラベルを使い、中段は`"{カテゴリ}-{n}"`、下段は
  `"{カテゴリ}A{n}"`という形式にして両者を区別している。クリックすると
  `vab_dummy_mid_{i}`/`vab_dummy_bottom_{i}`というローカル生成idで通常通り`vab_press`を送る
  (サーバー側は汎用的に`button_id`をログするだけなので、未知のidでも問題なく動作する)

VABはまだ実ハードウェア非連動の開発用ダミー段階であるため、**「カテゴリごとに実際に何を
表示・操作すべきか」はまだフロント側だけの試作**であり、サーバー(C++)側はカテゴリという
概念自体を持たない(先頭行の4ボタンを含め、VabConfig自体は今まで通り単一の固定24ボタン
グリッドを1回だけ送る)。サーバー側を本当にカテゴリ対応させる(カテゴリ選択に応じて
別のVabConfigを配信する等)のは将来の課題(CLAUDE.md参照)。

### 7.5 状況パネル仕様

- 表示項目(ラベル・単位・並び順)は`StatusPanelConfig`によりC++側が動的に決定する
- フロント側は表示項目をハードコードせず、`items`の定義通りに`SimState.status_values`を
  並べて表示する
- v1では数値項目のみを対象とする

### 7.6 タブ付きパネル(`components/tabbed_panel.rs`)

右パネル上下段(トップ/ボトムステータスパネル、`components/right_panel.rs`)は、
汎用の`TabbedPanel`コンポーネントで実装する。パネル固有の名前(「各種情報」「側面図」等)は
**タブのラベル**であり、パネル自体の見出し(タイトル)は位置に基づく汎用名
(「トップステータスパネル」「ボトムステータスパネル」)にすることで、後から同じ枠に
別内容のタブを追加できるようにしてある。

```mermaid
classDiagram
    class TabbedPanel {
        +String title
        +Vec~Tab~ tabs
    }
    class Tab {
        -label: &str
        -view: AnyView
    }
    class TopStatusPanel {
        title = "トップステータスパネル"
        tabs = [("各種情報", StatusPanel)]
    }
    class BottomStatusPanel {
        title = "ボトムステータスパネル"
        tabs = [("断面図", CrossSectionView), ("見通し範囲", LosView)]
    }
    TabbedPanel o-- Tab
    TopStatusPanel ..> TabbedPanel : 使う
    BottomStatusPanel ..> TabbedPanel : 使う
```

ボトムステータスパネルへ「見通し範囲」タブ(6.10節)を追加したのが、複数タブ構成の最初の
実例(それまではどちらのパネルも1タブのみだった)。タブ配列に`tab(...)`のエントリを
1行足すだけで、`TabbedPanel`側の変更は一切不要だった(設計意図通りの拡張性)。

- `active: RwSignal<usize>`で選択中タブのインデックスを保持する
- 各タブの中身(`AnyView`)は初回描画時に全タブぶん一度だけ生成してDOMに残し、
  非選択タブは`style:display="none"`で隠すだけにする(タブ切り替えのたびに
  作り直さない。Leptosの再マウントコストを避けるための一般的なパターン)
- タブが1個しかない場合でもタブバー自体は表示する(見た目の一貫性のため、
  タブ数によって表示/非表示を切り替えるような分岐は入れていない)

### 7.7 メニューバー・原点設定フローティングパネル

画面最上部のメニューバー(`components/menu_bar.rs`)は「ファイル」「設定」「表示」
「ヘルプ」の4項目。「設定」→「原点設定...」を選ぶと、原点入力フォーム(緯度・経度・
`設定`ボタン、DETAILED_DESIGN.md 3.5節のバリデーション込み)をフローティングパネル
(`components/origin_dialog.rs`)として画面中央に表示する。フォーム自体の中身は
実装当初シミュレーションステータスパネルに直接埋め込まれていたものをそのまま
移設したもので、ロジックに変更はない。

```mermaid
stateDiagram-v2
    [*] --> 閉: 初期状態
    閉 --> 開: 設定→原点設定...をクリック
    開 --> 閉: ✕ / 背景クリック
```

- 開閉状態は`ui_state::OriginDialogState`(`RwSignal<bool>`のラップ)を`provide_context`
  で共有し、`MenuBar`(トリガー)・`OriginDialog`(表示)の双方が`use_context`で参照する
- メニューのドロップダウン・原点設定パネルとも、背景の透明な`.menu-backdrop`/
  半透明の`.origin-dialog-backdrop`をクリックすると閉じる(パネル本体のクリックは
  `ev.stop_propagation()`でバックドロップまで伝播させない)
- 「ファイル」「表示」「ヘルプ」は現時点では項目未定のため、クリックすると
  「(準備中)」のプレースホルダのみ表示する(実装の骨組みだけ用意し、後から
  項目を追加できるようにしてある)
- 実装上の注意: メニュー項目のクリックハンドラで`WsConnection`(非`Copy`)を
  ムーブするクロージャ(`on_submit`)を、開閉のたびに何度も呼ばれる`Fn`/`FnMut`
  クロージャの外側で1回だけ作ると「2回目以降の呼び出しでムーブ済みエラー」に
  なる(Leptosの`{move || ...}`は再実行される前提のため`FnMut`である必要がある)。
  `origin_dialog.rs`では、開閉のたびに実行される内側のクロージャの中で
  `conn.clone()`してから`on_submit`を作ることで回避している

### 7.8 シミュレーションステータスパネルの状態表示(AppStatus)

シミュレーションステータスパネルのバッジは、**接続そのものの状態(`ConnectionStatus`、
フロント側が自前で計算)**と**シミュレータアプリケーション自体の状態(`AppStatus.text`、
C++側から配信)**という2つの独立したレイヤーの情報を、1つの文字列に出し分けて表示する:

- `ConnectionStatus::Connected`のとき: `AppStatus.text`をそのまま表示
  (例: "シミュレーション実行中" / "一時停止中")
- それ以外(`Connecting`/`Reconnecting`/`PausedHidden`)のとき: 詳細を出し分けず
  一律「接続中」と表示する(バッジの背景色は`ConnectionStatus`ごとに従来通り
  色分けされたまま)

`AppStatus.text`の実体はC++側`Simulation::running_`(pause/resumeコマンドで変化)に
連動しており、`OriginState`の`origin_changed`と同じパターンで
`SimulationTickResult::app_status_changed`フラグを介して、値が変化した時と
接続直後にのみ配信する(毎フレームは送らない)。

---

## 8. UML図一覧(索引)

| 図 | 節 | 種別 |
|---|---|---|
| 前処理ツール処理フロー | 2.7 | フローチャート |
| 原点状態の状態遷移図 | 3.6 | ステートマシン図 |
| プロトコルのクラス図 | 4.6 | クラス図 |
| シーケンス図: 接続確立 | 4.7 | シーケンス図 |
| シーケンス図: set_origin | 4.8 | シーケンス図 |
| シーケンス図: VABボタン押下 | 4.9 | シーケンス図 |
| C++側スレッドモデル | 5.1 | フローチャート |
| C++側クラス図 | 5.3 | クラス図 |
| Rustコンポーネント構成図 | 6.1 | コンポーネント図 |
| WebSocket接続管理クラス図 | 6.2 | クラス図 |
| 再接続状態遷移図 | 6.3 | ステートマシン図 |
| 地形描画パイプライン | 6.4 | フローチャート |
| 地形描画クラス図 | 6.5 | クラス図 |
