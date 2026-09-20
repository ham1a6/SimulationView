# Sim3dView 詳細設計書

## 0. 本書の位置づけ

設計書は次の3層で構成する(全体の索引は[README.md](README.md))。

| 層 | 文書 | 内容 |
|---|---|---|
| 基本設計 | [BASIC_DESIGN.md](BASIC_DESIGN.md) | 背景・全体構成・技術スタック・確定した設計方針(17項目)・ディレクトリ構成 |
| 詳細設計(システム) | **本書** | 地形データの実態・前処理の方針・座標系・通信プロトコル・C++サーバー・ライブラリの設計方針(モジュール構成・図・「なぜそうしたか」)・サンプルアプリのUI |
| 詳細設計(ライブラリ実装仕様) | [impl/](impl/)の第1〜5部 | 定数・アルゴリズム・バイト配置・シェーダー全文・テストを、実装コードから起こした正確な仕様 |

**役割分担(同じ事実を2か所に書かないための約束)**

- 数値・定数・アルゴリズム・バイト配置・WGSLの**正は実装仕様(`impl/`)**。コードから起こしているので、本書と食い違って見えたら実装仕様が正しい(本書を直す)。
  本書はそれらを書き写さず、**方針と理由(なぜ)・全体像・図**を持ち、詳細は各節から`impl/`の該当節へ案内する
- 機能ごとの経緯(要望→調査→原因→修正→実機確認)は[DEVELOPMENT_HISTORY.md](DEVELOPMENT_HISTORY.md)。本書には結論だけを書く
- **節番号はソースコードのコメントから参照されている**(例: `DETAILED_DESIGN.md 6.10節`)。節の追加は枝番で行い、番号を詰め直さない
- 通信プロトコル(4節)・C++サーバー(5節)・サンプルアプリのUI(7節)は`sample/`(アプリ固有)の設計で、ライブラリの実装仕様(`impl/`)の対象外。こちらは本書が正

UML図はMermaid記法で記述している。GitHub/GitLab/VSCode等、Mermaidをネイティブサポートするツールでプレビューすればそのまま図として描画される。

---

## 1. 対象地形データの実態調査結果

入力はALOS全球数値地表モデル(ALOS World 3D-30m)のGeoTIFFタイル。当初は17タイル(北緯35〜40°・東経135〜140°)で調査し、その後`map_data/`が増えて
現在は**390タイル**(北緯20〜50°・東経120〜150°の30°四方)になっている。以下の性質はタイルが増えても変わらない。

### 1.1 ファイル構成(1タイルあたり)

`map_data/ALPSMLC30_<TILEID>_*` の形式で、1タイルあたり最大6ファイル。

| サフィックス | 内容 | 本設計での用途 |
|---|---|---|
| `_DSM.tif` | 数値表層モデル(標高、GeoTIFF, 3600×3600px) | **使用**(入力ラスタ) |
| `_MSK.tif` | 品質マスク(海・雲・代替データ補完等のフラグ) | **使用**(画素値3=海の判定のみ。1.4節) |
| `_STK.tif` | パンクロマチック(白黒)画像 | **不使用**(確定事項。標高グラデーション着色のみ) |
| `_HDR.txt` | タイルのヘッダ情報(四隅座標・解像度・楕円体等) | 前処理ツールのテスト・検証用の参考情報 |
| `_LST.txt` | 元シーン(観測パス)のリスト | 使用しない |
| `_QAI.txt` | 品質指標(SRTM/ASTERとの差分統計等) | 使用しない(1.4節の調査でMASK統計値との突き合わせにだけ使った) |

### 1.2 タイル分布と規模

- タイルID命名: `N<緯度2桁>E<経度3桁>` = タイル**南西角**の整数度。1タイル = 経緯度1°×1°、3600×3600px(1秒角)
- 現在の範囲は`metadata.json`の`geodetic_bounds`(北緯20〜50°・東経120〜150°)。30×30=900セルのうち、陸のある390枚が存在する
  (海だけのセルは元データに無いか、前処理が「陸なし」として出力しない。フロントでは存在しないタイルとして扱う)
- **外接矩形はハードコードではなく、`geotiff_preprocess`が見つかったタイルIDの最小/最大から実行時に決める**(2.3節)。
  `map_data/`に別の場所のタイルを増減しても、ツールを再実行するだけで追従する(サーバーは起動時に`metadata.json`を読むので、sim_serverの再起動も必要)
- 解像度: 1秒角(3600px/度)= 南北方向約30m/px、東西方向は緯度のcos分だけ狭い(北緯35°で約25m/px)

### 1.3 標高データの実態

- 標高範囲(全390タイル、単一画素の実測): **-330m 〜 3937m**(`metadata.json`の`elevation_min`/`elevation_max`)。17タイルの調査時は-102m〜3771m(富士山)
- 明示的なnodataセンチネル値(-9999等)は検出されなかった。海はDSM上では標高0mで格納されている(1.4節)
- データ型は符号付き整数(16bit相当)。前処理でf32メートルに変換し、出力はint16に四捨五入する
- 水面ノイズ由来の大きな負値が一部にあるため、標高の色の正規化では下限を`elevation_min`ではなく**0m固定**にしている(実装仕様 [第1部](impl/1_data_and_geodesy.md) 6.2)

### 1.4 欠損・海域の扱い(方針)

前処理は、無データ領域を**f32のNaNで一律に表し**(標高0mの実在する陸地と区別するため)、出力グリッドでは`i16::MIN`(-32768、`NO_DATA`)にする。
フロントは`NO_DATA`のノードを海(=地形なし)と判定し、そのノードを含む三角形は描画しない(海岸線は最大1セル分陸側に退く)。
存在しないタイル・タイル内のNODATA・海マスクの水域は、すべてこの「地形メッシュが無いところ」になり、そこに水域レイヤーが描かれる(6.10節)。
`metadata.json`に欠損フラグは設けない(`NO_DATA`自体が目印になる)。`elevation_min`/`elevation_max`はNaNを除いた実データだけで計算する。

**タイル内部の海域は`*_MSK.tif`(マスクファイル)で検出する**: GDALの`GetNoDataValue()`だけでは、タイル内部で完結する海域(沿岸・内海)を検出できなかった。
実測で、DSMは海域画素に「標高0m」という一見有効な値を格納しており、対応する`*_MSK.tif`(DSMと同じ3600×3600、1バイト/画素)の画素値**3**が海に正確に対応することを、
`*_QAI.txt`の`MASK_NUM_SEA`統計との突き合わせで確認した(0=有効、3=海、4/12=代替データ[GSI10/PSM]で補完した陸地で、NaN化しない)。
マスクが読めないときは警告してNODATAだけの判定に落とし、処理は止めない(17タイルでの実測: モザイク全体のNaN率は約32%→約60%になり、沿岸・内海の精度が大きく改善した)。

**タイル内の小さなNODATA穴は周囲から補間して埋める**(「範囲内の欠損は周りから補間できる?」→「タイル内の小さな穴だけ補完で大丈夫」というやり取りによる仕様):
タイルが丸ごと無い大きな欠損は、補間しても実際の地形とは無関係な架空の起伏になるだけなので対象外(NaNのまま)。海は「欠損」ではなく「実際に海」なので補間の材料にも対象にもしない。
連結成分が2000画素(ネイティブ解像度で直径約1.5kmの円)以下の穴だけを、境界から内側へ波及させながら確定済み近傍の平均で埋める
(アルゴリズムは実装仕様 [第1部](impl/1_data_and_geodesy.md) 7.2)。現在の`map_data/`にはGDALが検出するNODATAが無いため、このパスは実際には発火しない(別のDSMソースを使う場合に備えた対応)。

---

## 2. GeoTIFF前処理ツール仕様

`tools/geotiff_preprocess`(C++/GDAL、独立したCMakeプロジェクト)。バイト単位の出力契約とアルゴリズムの正は実装仕様 [第1部](impl/1_data_and_geodesy.md) の1章・7章で、
本節は方針と理由を述べる。

### 2.1 入力

`map_data/*_DSM.tif` を機械的に列挙する(ディレクトリを走査し、ファイル名から`N###E###`パターンを抽出してタイル位置を決定する。タイルの追加にそのまま対応できる)。

### 2.2 座標系(再投影は不要)

ALOS DSMタイルは元々EPSG:4326(WGS84/GRS80楕円体)の緯度経度グリッドで、1タイル=1°×1°=3600×3600pxちょうどに整列している(タイル境界での位置ズレなし)。
原点(=座標変換の基準)がUI入力によるランタイムパラメータであるため、前処理側で特定の投影に固定する意味がない。よって:

- **再投影(reproject)は行わない**。緯度経度グリッドのまま、タイルごとにダウンサンプリングするだけ
- 実際のメートル単位の座標(東/北/上)への変換は、UIで指定された原点をもとに**フロント側が実行時に行う**(3節)
- GDALの役目は「GeoTIFFの読み取り・ダウンサンプリング・書き出し」に限定される(`sim_server`本体はGDALをリンクしない)

### 2.3 外接矩形と処理単位(全域モザイクは作らない)

1. `discover_tiles()`: `map_data/`内の`*_DSM.tif`を走査し、ファイル名からタイルID(南西角の整数緯度経度)だけを読み取る(GDALでの実データ読み込みはまだ行わない、高速)
2. `compute_mosaic_bounds()`: 見つかった全タイルIDの最小/最大から外接矩形(max側は南西角+1)を求め、`metadata.json`の`geodetic_bounds`に書く
3. 各タイルを**独立に**読み込み、その場でレベルごとのグリッドを作って書き出す(ストリーミング。並列処理)

> **経緯**: 当初は全タイルを1枚のモザイク(18000×18000px等)にして2048×2048へ縮小していたが、対象域が390タイルになると全域モザイクは約46GBでメモリが限界になり、
> 1セルも約1.6kmまで粗くなったため、タイル単位の処理(下記のタイルLOD)に変えた。モザイクは作らないので、メモリは1スレッドあたり数百MB。

### 2.4 解像度レベル(タイルLOD)

> **経緯**: 単一メッシュのまま解像度を上げると頂点バッファがWebGPUの上限(既定256MB)を超えるため、**1度タイル単位の複数解像度(LOD)方式**にした。
> さらに最細レベルを元データの解像度(30m)にするため、**1度タイルを6×6のチャンクに分割**した(DEVELOPMENT_HISTORY.md「地形の高解像度化」「拡大時に元データの解像度(30m)まで表示」)。

タイル(1度x1度)ごとに、一辺を`N`セルに分割した`(N+1)x(N+1)`ノードのグリッドを複数のレベルで出力する。現在は5レベル(`kLevelCells`):

| レベル | 一辺のセル数N | セルの大きさ | 単位 |
|---|---|---|---|
| 0 | 60 | 約1.85km | タイル全体で1枚 |
| 1 | 180 | 約620m | チャンク(N/6=30セル) |
| 2 | 600 | 約185m | チャンク(100セル) |
| 3 | 1800 | 約62m | チャンク(300セル) |
| 4 | 3600 | 約31m(元データ=1画素) | チャンク(600セル、約36万頂点) |

- 各ノードの値は、そのノード位置を中心とする1セル幅の窓内の有効画素の**平均**(NaN除外。窓内に有効画素が1つでもあれば陸、全部NaNなら海=データなし)
- **チャンク**: レベル1以上は、1度タイルを6×6のチャンク(緯度経度とも1/6度、約18km×15km)に分け、チャンクごとにグリッドを持つ。隣のチャンクと縁のノードを共有するので、
  同じレベルのチャンク同士は縁が一致する。1タイル全体の30mグリッドは約1300万頂点でGPUバッファ上限を超えるため、カメラのすぐ近くのチャンクだけを最細にする
- 陸のないタイル(海のみ)は出力せず、存在しないタイルとして扱う(フロントでは標高0mの海と同じ扱い)

### 2.5 出力

| ファイル | 内容 | 備考 |
|---|---|---|
| `metadata.json` | 2.6節 | 全体の緯度経度範囲・標高範囲・楕円体・レベル定義(`tile_levels`)・チャンク分割数 |
| `tile_index.json` | 存在するタイルの一覧(`lat`,`lon`,`elevation_min`,`elevation_max`) | 緯度→経度の昇順 |
| `base.bin` | レベル0(タイル全体で1枚)を全タイル分、`tile_index.json`の順に連結 | 約2.9MB。フロントが起動時に全部取得して常時保持する |
| `tiles/L{k}/N035E138.bin` | レベルk(1以上)のタイル別ファイル。チャンクのレコードを連結 | 1タイルあたりレベル1が約69KB、2が約735KB、3が約6.5MB、4が約26MB(全体で約12GB) |
| `texture.png` | **生成しない** | 標高グラデーション着色を使うため不要 |

グリッドは`int16`(標高メートル、四捨五入)・リトルエンディアン・row-major。**行は南→北、列は西→東**、データなしは`-32768`。
GDALはラスタを北→南で格納するので、前処理はグリッド化の際に行順を南→北へ反転する(しないと地形が南北反転する。実装時に実際に発生し修正済み)。
`tiles/L{k}/*.bin`は、チャンク(番号=行(南→北)×6+列(西→東))を番号順に並べた**固定サイズのレコード**の連結なので、
フロントは**HTTP Range**で必要なチャンク1個分だけ取得できる。バイト位置・サイズの計算式は実装仕様 [第1部](impl/1_data_and_geodesy.md) 1.4・1.5。

### 2.6 `metadata.json`

フィールドの定義・検証・書式(`setprecision(15)`等)の正は実装仕様 [第1部](impl/1_data_and_geodesy.md) 1.2。各フィールドを置いた理由:

| フィールド | 理由・用途 |
|---|---|
| `tile_levels` | レベルごとの1度タイル1辺あたりのセル数(先頭がレベル0=最粗)。フロントはこの配列からレベル数・各グリッドの大きさを決めるので、前処理側で値を変えてもフロントの変更は不要(レベル1以上は`chunks_per_tile`で割り切れること) |
| `chunks_per_tile` | 1度タイルを何×何のチャンクに分けるか(レベル1以上) |
| `geodetic_bounds` | 全タイルの外接矩形(整数度)。原点入力のバリデーション(3.5節)・タイル索引の大きさ・範囲チェックに使う。サーバーも起動時にこれを読む |
| `elevation_min`/`elevation_max` | ダウンサンプリング前の全画素から実測。色の正規化は上限だけを使う(下限は0m固定。1.3節) |
| `ellipsoid`/`height_datum` | フロント側のENU変換(3節)のパラメータ(WGS84相当。DSMの標高はEGM96海抜) |
| `has_texture` | `texture.png`の有無を毎回fetch失敗で判定せずに済むよう明示フラグとして持つ(現在は常にfalse) |
| `default_origin` | UIの原点入力欄の初期値。実際に使われる原点はサーバーが保持する状態(`OriginState`。5節)が正で、これは未接続時のUI初期表示用 |

### 2.7 前処理ツール処理フロー(UML: アクティビティ相当のフローチャート)

```mermaid
flowchart TD
    A["map_data/*_DSM.tifを列挙\n(外接矩形を計算)"] --> B["タイルを並列に処理"]
    B --> C["1タイルをGDALで読み込み\n(海マスク→NaN、小さな穴は補間)"]
    C --> D["レベルごとに(N+1)x(N+1)ノードの\n窓平均グリッドを作る(南→北)"]
    D --> E["レベル1以上: 6x6のチャンクに分けて\ntiles/Lk/名前.binへ書き出し"]
    D --> F["レベル0: メインスレッドへ返す"]
    F --> G["base.bin / tile_index.json / metadata.json書き出し"]
```

---

## 3. 座標系設計(ENU座標系)

### 3.1 定義

- シミュレーション座標系 = **東=X、北=Y、鉛直上向き=Z** の局所ENU(East-North-Up)座標系
- 原点 = UIで入力された緯度経度地点の、**海抜0mの点**(実装上は、DSMの海抜をそのまま楕円体高として扱うので、その地点の楕円体高0mの点。3.2節)
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
  (ジオイド高と楕円体高の差は本設計では無視する簡略化。日本周辺のジオイド高は数十m規模で最細セル(約30m)と同程度以上のため、
  標高の絶対値の厳密さが要る用途では見直しが必要になる)
- 地球の曲率は上式に自然に反映される(遠方の地点ほど`Up`が減少していく。原点から1,000kmで約80km)。広域では
  原点から離れるほど地表が「下に沈んで」見える効果が正しく表現される
- Rust実装は `sim3dview/src/terrain/geodesy.rs` の `EnuTransform` 構造体(前計算するもの・逆変換・水域シェーダー用の係数は実装仕様 [第1部](impl/1_data_and_geodesy.md) 4章)。C++側(サーバー)は
  座標変換自体を行わず、緯度経度のみを状態として保持する(5.3節参照)。

### 3.3 計算をどこで行うか

- **前処理(C++/GDAL)側では行わない**。タイルのグリッドと`metadata.json`は原点非依存の生データのまま
- **フロント側(Rust/WASM)がメッシュ構築時・原点変更時にランタイムで計算する**。常駐する全タイル
  (数百万頂点)の変換を原点ごとに1回行う。行(緯度)・列(経度)ごとの三角関数を前計算して1頂点あたりの
  計算を軽くしてあり、WASM上でも実用上問題ない速度で完了する
- 原点を変更したら、常駐している全タイル(各自の解像度レベル)の頂点バッファを再計算・再アップロードする
  (タイルデータの再フェッチは不要。インデックスは原点に依存しないので作り直さない)

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
  チェック)。範囲外の`set_origin`が送られてきた場合は`CommandError`を返す。範囲は`Simulation`の
  コンストラクタが起動時に`assets/terrain/metadata.json`の`geodetic_bounds`から読む(ハードコードしない。
  読めなかった場合はチェックを無効にして警告を出す)

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
| 0x07 | TrackList | Server→Client | シミュレーション進行中は約20Hz(SimStateの3フレームに1回)、接続直後にも1回。停止中は送らない |
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

**TrackList**(サーバー→クライアント、進行中は約20Hz+接続直後。6.12節)
```
t: f64                         // シミュレーション時刻(秒)
tracks: Vec<Track>             // 全トラックの最新状態(消えたトラックは次の一覧から抜ける)
  Track:
    id: u32                    // 同じ実体には常に同じID(フロントの航跡・ラベルの対応づけ)
    kind: u8                   // 0=不明 1=固定翼機 2=ヘリ 3=艦船 4=地上車両 5=ミサイル
    affiliation: u8            // 0=不明 1=友軍 2=敵 3=中立
    label: String              // 表示名(コールサイン等)
    lat_deg: f64
    lon_deg: f64
    alt_m: f64
    alt_ref: u8                // 0=alt_mは海抜 1=地表からの高さ(サーバーが地形の高さを持たない車両など)
    heading_deg: f64           // 進行方向(北から時計回り)
    speed_mps: f64             // 対地速度
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
- `TrackList`は全トラックの最新状態をまとめて、シミュレーション進行中だけ約20Hzで送る(3フレームに1回)。位置は
  シミュレーション時刻の関数で、停止中は変わらないので送らない(新規接続には接続直後に1回)。フロントの描画は全体の再描画になるので、
  60Hzで送らずに表示に十分な頻度に抑えている

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
        AppStatus = 0x06
        TrackList = 0x07
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
    class AppStatus {
        +String text
    }
    class TrackList {
        +f64 t
        +Vec~Track~ tracks
    }
    class Track {
        +u32 id
        +u8 kind
        +u8 affiliation
        +String label
        +f64 lat_deg
        +f64 lon_deg
        +f64 alt_m
        +u8 alt_ref
        +f64 heading_deg
        +f64 speed_mps
    }
    VabConfig "1" *-- "many" VabButton
    StatusPanelConfig "1" *-- "many" StatusItem
    TrackList "1" *-- "many" Track
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
    WS->>Sim: snapshot_app_status()
    WS-->>C: AppStatus (0x06)
    WS->>Sim: snapshot_tracks()
    WS-->>C: TrackList (0x07)
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
        WS-->>C: OriginState (0x03, broadcast。要求元も含む全クライアントへ)
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
        +snapshot_app_status() AppStatus
        +snapshot_tracks() TrackList
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
        +bool app_status_changed
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
| `GET /terrain/tile_index.json` | application/json | `assets/terrain/tile_index.json`をそのまま返す |
| `GET /terrain/base.bin` | application/octet-stream | `assets/terrain/base.bin`(全タイルの最粗レベルの連結)をそのまま返す |
| `GET /terrain/tiles/:level/:name` | application/octet-stream | `assets/terrain/tiles/{level}/{name}`(例: `L2/N035E138.bin`)。`level`・`name`は英数字・`_`・`.`のみ許可し、`..`を含むものは400(パストラバーサル対策)。**HTTP Range(単一範囲`bytes=a-b`)に対応**し、大きいファイル(最細レベルで1タイル約26MB)からチャンク1個分だけを206で返せる |

**キャッシュ**: 4つのルートとも、応答に`ETag`(ファイルの大きさ+更新時刻)と`Cache-Control: no-cache`を付ける。ブラウザは保存した
応答を使う前に毎回`If-None-Match`で確認し、変わっていなければ本体なしの**304**が返る(`*`・`W/`付き・カンマ区切りにも対応)。
2回目以降の表示で数十MBを取り直さずに済み、`map_data/`を作り直したときはETagが変わるので必ず新しいものになる
(有効期限(`max-age`)は付けない。古い地形を使い続ける事故を避けるため)。

フロント(trunk serveでホストされる別オリジン)からfetchされるため、全ルートとも
`Access-Control-Allow-Origin: *`ヘッダーを付与する。想定CWD(カレントディレクトリ)は`sim_server/`
(`assets/terrain/...`という相対パスでファイルを開くため)。

### 5.5 GeoTIFF前処理ツール(geotiff_preprocess)のクラス構成

`tools/geotiff_preprocess/main.cpp`に実装(単一ファイル、`sim_server`本体とは別のCMakeプロジェクト・別実行ファイル。sim3dviewライブラリの一部としてリポジトリ直下の`tools/`に置く)。

| 関数/構造体 | 役割 |
|---|---|
| `GeodeticBounds` | 出力`metadata.json`の`geodetic_bounds`(整数度の外接矩形) |
| `TileId` | ファイル名から抽出した`(lat, lon)`整数度 |
| `parse_tile_filename()` | `"ALPSMLC30_N035E138_DSM.tif"` → `TileId{35, 138}` |
| `DiscoveredTile` | `TileId` + ファイルパス(まだGDAL読み込みはしていない) |
| `discover_tiles()` | map_dataディレクトリをglobし、ファイル名からタイルIDだけを列挙(1段階目) |
| `MosaicBounds` | モザイクの外接矩形(整数度)。ハードコードではなく実行時に計算する値 |
| `compute_mosaic_bounds()` | 見つかった全タイルIDの緯度・経度の最小/最大から外接矩形を求める |
| `SourceGrid` | GDALで読み込んだ1タイル分の標高グリッド + 地理範囲 |
| `load_sea_mask()` | 対応する`*_MSK.tif`を読み、画素値3(海)の位置をビットマップで返す(1.4節) |
| `fill_small_nodata_holes()` | 海ではない小さなNODATA穴(連結成分が`kMaxFillableHolePixels`以下)を周囲の有効画素からBFSで補間して埋める(1.4節) |
| `load_dsm_tile()` | GDALでGeoTIFFを1枚読み込む(再投影しない)。小さなNODATA穴は補間、残ったNODATA画素+マスク海域画素はNaNへ置換 |
| `tile_name()` | タイルIDから`"N035E138"`形式の名前を作る(フロントのURLと一致させる) |
| `build_level_grid()` | 1タイルの標高から、レベルごとの`(N+1)x(N+1)`ノードの窓平均グリッド(南→北・int16)を作る(2.4節) |
| `write_chunked_level_file()` | タイル全体のグリッドを6x6のチャンク(縁のノードは隣と共有)に分け、固定サイズのレコードとして連結して書き出す(2.5節) |
| `process_tile()` | 1タイルを読み込み、全レベルのグリッドを作る。レベル1以上はその場でチャンク形式のファイルへ書き出し、レベル0はメインへ返す |
| `TileResult` | `process_tile()`の結果(タイル位置・標高の最小最大・レベル0のグリッド) |
| `write_int16_file()` / `write_tile_index_json()` / `write_metadata_json()` | 出力ファイル書き出し |
| `main()` | タイル列挙→並列処理→`base.bin`連結→索引・メタデータ出力 |

---

## 6. Rust側詳細設計

> **注記(ライブラリ/サンプル分離後の対応関係)**: Rust側は`sim3dview`ライブラリcrateと`sample/sim_frontend`サンプルアプリcrateに分かれている
> (経緯はDEVELOPMENT_HISTORY.md「sim_frontendをライブラリとサンプルアプリに分離」)。本節は**ライブラリ(`sim3dview`)の設計**が中心で、
> 6.1〜6.3の図には`sample/sim_frontend`側(通信・全体レイアウト)も含む。数値・アルゴリズム・シェーダーの正確な仕様は実装仕様
> [impl/](impl/)(下表の「実装仕様」列)。
>
> - **`sim3dview`ライブラリ**: 地形描画パイプライン・カメラ・座標軸・標高配色・シェーダ・LOS/覆域計算・markers・pick・作図・航跡(6.2を除く本節)、
>   7.6節のTabbedPanel、7.7節のフローティングパネル・右クリックメニュー・原点設定/覆域高度設定ダイアログの実装本体
> - **`sample/sim_frontend`サンプルアプリ**: 4節の通信プロトコル(`protocol.rs`/`ws.rs`)、6.2・6.3の接続管理、`app.rs`(全体レイアウト)、7.4節のVAB、7.5節の状況パネル、7.7節のメニューバー(トリガーのみ)
> - ライブラリは通信プロトコルもサーバーのURLも知らない。原点は`terrain::origin::OriginState`(プロトコル非依存)で受け、`app.rs`が`protocol::OriginState`⇔ライブラリの橋渡しEffectを持つ。
>   地形の取得先も、呼び出し側(`app.rs`)が`base_url`として明示的に渡す

### 6.0 ライブラリ(`sim3dview`)のモジュール構成

公開(`pub mod`)は、アプリが使う`camera`・`draw_tool`・`drawing`・`hillshade`・`markers`・`origin`・`origin_pick`・`recenter`・`store`・`tracks`と`ui`だけで、
それ以外は`pub(crate)`(ライブラリの内部。詳細は実装仕様 [第5部](impl/5_ui_and_integration.md) 1.2)。

| 領域 | モジュール | 役割 | 実装仕様 |
|---|---|---|---|
| データ | `terrain::fetch` | metadata.json・tile_index.json・base.bin・タイルのHTTP取得(Range含む) | 第1部 2章 |
| | `terrain::loader` | `TerrainData`(取得済みグリッドの保持・双線形サンプリング・キャッシュの追い出し)・メタデータ型 | 第1部 3章 |
| 測地 | `terrain::geodesy` | `Ellipsoid`(`WGS84`)・`EnuTransform`(緯度経度⇔ENU。回転は`DMat3`) | 第1部 4章 |
| | `terrain::heightmap` | 標高サンプリング(`sample_heightmap`)・地表点のENU座標(`ground_at_enu`/`ground_at_geodetic`) | 第1部 5章 |
| | `terrain::origin` | `Origin`・`OriginState` | 第5部 2.1 |
| メッシュ・LOD | `terrain::mesh` | 頂点・インデックス・法線・スカート・配色 | 第1部 6章 |
| | `terrain::lod` | LOD計画(`plan_levels` = `evaluate_tile` → 並べ替え → `allocate_levels`)。`TileLayout` | 第2部 2章 |
| 描画 | `terrain::renderer` | `TerrainRenderer`(状態と3つの描画パス)。`pipelines`・`targets`(深度/MSAA/縮小)・`uniforms`・`overlay`(頂点バッチ)・`frustum` | 第3部 |
| | `terrain::vertex`・`terrain::render_bias` | 作図・航跡・マーカー共通の頂点`DrawVertex`(種類`KIND_*`)・Zバイアスの一覧 | 第3部 2章・第4部 1章 |
| 図形・航跡 | `terrain::drawing`・`drawing_geometry`・`draw_tool`・`tracks`・`markers` | 6.9・6.11・6.12節 | 第4部 |
| 計算 | `terrain::los`・`profile`・`pick`・`camera` | 見通し(`RayContext`)・断面・ピッキング・カメラ | 第2部 |
| UI | `ui::terrain_view`(`mod.rs`=コンポーネント、`state`・`frame`・`lod_driver`・`overlay`・`labels`・`picking`) | 地図canvas | 第5部 3・4章 |
| | `ui::context_menu`(`MapMenuState`を含む)・`floating_panel`・`tabbed_panel`・`origin_dialog`・`drawing_editor`・`util` ほか | 汎用部品・ダイアログ | 第5部 5章 |

**テスト**: `cargo test -p sim3dview`(ネイティブ)。`TerrainData::synthetic`(`cfg(test)`)で合成地形を作り、標高サンプリング・丸み込みの`ground_at_enu`・LODの予算配分・反転Z・`screen_to_ray`・
電波の地平線(見通し)・視錐台カリングなどを検証する。WGSL(`terrain.wgsl`・`draw.wgsl`)は`naga`で構文・型を検証し、uniform・頂点のレイアウトがRust側の構造体と一致することを確かめる
(方針は[IMPLEMENTATION_GUIDE.md](IMPLEMENTATION_GUIDE.md) 4.3、テスト一覧は各実装仕様の「検証」節)。

### 6.1 コンポーネント構成図

```mermaid
graph TD
    App["App (app.rs)<br/>3カラムCSS Gridレイアウト・リサイザー"]
    App --> SimulationStatusPanel["SimulationStatusPanel<br/>(operation_panel.rs) 接続状態・原点・フレーム・航跡数・開始/一時停止"]
    App --> VabPanel["VabPanel<br/>(vab.rs) 先頭行=カテゴリタブ、中段・下段=フロント側ダミー"]
    App --> MainPanel["MainPanel<br/>(main_panel.rs) 地形描画canvas(3D/2D, TerrainView)"]
    App --> TopStatusPanel["TopStatusPanel<br/>TabbedPanel: [各種情報]=StatusPanel / [航跡情報]=TrackDetail"]
    App --> BottomStatusPanel["BottomStatusPanel<br/>TabbedPanel: [断面図]=CrossSectionView / [見通し範囲]=LosView"]
    App --> DrawingWindow["DrawingWindow<br/>(drawing_window.rs) 非モーダルFloatingPanel+DrawingEditor(7.7節)"]
    App --> ContextMenu["ContextMenu<br/>右クリックメニュー本体(項目はmap_menu.rsが決める。7.7節)"]

    App -.provide_context.-> WsSignals["WsSignals<br/>(接続状態・受信データのシグナル群)"]
    App -.provide_context.-> TerrainStore["TerrainStore<br/>(地形データを全パネルで共有)"]
    App -.provide_context.-> RadarMarkersState["RadarMarkersState<br/>(観測点一覧・選択状態、全パネル共有)"]
    App -.provide_context.-> DrawToolState["DrawingState / DrawToolState<br/>(作図の一覧・図形の対話作成。6.11節)"]
    App -.provide_context.-> TracksState["TracksState<br/>(航跡の一覧・選択・表示設定。6.12節)"]
    App -.provide_context.-> MenuStates["ContextMenuState / MapMenuState<br/>(右クリックメニューの状態と、地図の項目を作る関数)"]
    App -.propとして渡す.-> WsConnection["WsConnection<br/>(Rc<RefCell<...>>、Send/Sync境界回避のためcontext不使用)"]

    MainPanel --> Loader["terrain::fetch / terrain::loader<br/>metadata.json・base.bin・タイル取得"]
    MainPanel --> Mesh["terrain::mesh<br/>ENU変換・頂点/インデックス生成"]
    MainPanel --> Renderer["terrain::renderer::TerrainRenderer<br/>wgpu Device/Queue/Pipeline(地形・水域)+draw系パイプライン(観測点ピン/2D覆域/作図/航跡)"]
    MainPanel --> Camera["terrain::camera::OrbitCamera<br/>view_proj行列・screen_to_ray"]
    MainPanel --> Pick["terrain::pick::pick_lat_lon<br/>クリック→レイキャストで緯度経度取得(右クリックメニュー・作図・原点指定)"]
    MainPanel --> Markers["terrain::markers::build_marker_geometry<br/>観測点・覆域の3D頂点生成"]
    BottomStatusPanel --> Los["terrain::los::compute_los<br/>全方位角の見通し限界距離"]
    TerrainStore -.共有データ.-> MainPanel
    TerrainStore -.共有データ.-> BottomStatusPanel
    RadarMarkersState -.共有データ.-> MainPanel
    RadarMarkersState -.共有データ.-> BottomStatusPanel
```

### 6.2 WebSocket接続管理のクラス図(サンプルアプリ側)

```mermaid
classDiagram
    class WsSignals {
        +RwSignal~ConnectionStatus~ status
        +RwSignal~Option~OriginState~~ origin
        +RwSignal~Option~VabConfig~~ vab_config
        +RwSignal~Option~StatusPanelConfig~~ status_panel_config
        +RwSignal~Option~SimState~~ last_sim_state
        +RwSignal~Option~CommandError~~ last_command_error
        +RwSignal~Option~AppStatus~~ app_status
        +RwSignal~Option~TrackList~~ track_list
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

`WsConnection`は`Rc<RefCell<Inner>>`を内部に持ちSend/Syncではないため、Leptos 0.8の`provide_context`(Send+Sync境界を要求する)には乗せられない。
そのため`WsSignals`はcontext経由、`WsConnection`は`WsHandle`(`StoredValue::new_local`で包んだ`Copy`のハンドル。`Send`+`Sync`を満たす)に包み、
コンポーネントのpropとして明示的に渡す設計とした(以前は`unsafe impl Send/Sync`を付与していたが、ハンドル化して`unsafe`を無くした)。

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
    A["fetch::load_terrain()<br/>metadata.json + tile_index.json + base.bin をfetch"] --> B["mesh::build_whole_tile_mesh()<br/>全タイルをレベル0でENU変換"]
    B --> C["TerrainVertex配列 + インデックス配列(タイルごと)"]
    C --> D["TerrainRenderer::new()<br/>wgpu Instance/Adapter/Device/Surface初期化"]
    D --> E["set_mesh()でメッシュごとに頂点/インデックスバッファへアップロード"]
    E --> F["TerrainRenderer::render(camera)<br/>view_projをuniformへ書き込み、水域→地形→重ね描きを描画"]
    F --> G["canvas(WebGPU)に描画"]
    E -.カメラ操作後.-> H["lod::plan_levels()でタイル/チャンクごとのレベルを決定<br/>(6.10節)"]
    H --> I["細かいレベルをfetch→メッシュ再生成→set_mesh()で差し替え"]
    I --> F
```

3つの描画パス(①地形・水域・絶対座標の重ね描き ②カメラ固定の重ね描き ③縮小)の構成と`TerrainRenderer`の公開APIは実装仕様 [第3部](impl/3_renderer_and_shaders.md) 1章・4章。

### 6.5 地形描画クラス図

```mermaid
classDiagram
    class TerrainMetadata {
        +Vec~u32~ tile_levels
        +u32 chunks_per_tile
        +f32 elevation_min
        +f32 elevation_max
        +GeodeticBounds geodetic_bounds
        +Ellipsoid ellipsoid
        +bool has_texture
        +DefaultOrigin default_origin
    }
    class TerrainData {
        +TerrainMetadata metadata
        +String base_url
        -Vec~i16~ base
        -Vec~TileEntry~ tiles
        +whole_grid(tile) [i16]
        +chunk_grid(tile, level, chunk) Rc~Vec~i16~~
        +insert_chunk_grid(key, level, chunk, data)
        +set_chunk_level(key, chunk, level)
        +sample_bilinear(tile, u, v) f32
        +evict_unused(keep, limit_bytes)
    }
    class EnuTransform {
        -DVec3 origin_ecef
        -DMat3 enu_from_ecef
        -f64 a
        -f64 e2
        +new(origin, ellipsoid) EnuTransform
        +transform(lat_deg, lon_deg, h) [f32; 3]
        +enu_to_geodetic(east, north, up) (lat, lon, h)
        +inverse(east, north) (lat, lon)
    }
    class TerrainVertex {
        +[f32; 3] position
        +[f32; 3] color
        +[i16; 2] normal_xy
    }
    class TerrainMesh {
        +Vec~TerrainVertex~ vertices
        +Vec~u32~ indices
    }
    class OrbitCamera {
        +Vec3 target
        +f32 distance
        +f32 yaw
        +f32 pitch
        +ViewMode mode
        +to_camera(aspect) Camera
        +keep_above_ground(ground_up)
    }
    class Camera {
        +Vec3 eye
        +Vec3 target
        +Vec3 up
        +Projection projection
        +view_proj_matrix() Mat4
        +screen_to_ray(x, y, w, h)
    }
    class TerrainRenderer {
        -Surface surface
        -Device device
        -Queue queue
        -Pipelines pipelines
        -RenderTargets targets
        -HashMap~MeshKey, MeshGpu~ meshes
        +new(canvas) TerrainRenderer
        +set_mesh(key, mesh)
        +render(camera) Result
        +resize(width, height)
        +update_markers(vertices)
        +update_drawings(batches)
        +update_tracks(vertices)
    }

    TerrainData *-- TerrainMetadata
    TerrainMesh *-- TerrainVertex
    OrbitCamera ..> Camera : to_camera
    TerrainRenderer ..> TerrainMesh : set_meshで頂点・インデックスバッファへコピー
    TerrainRenderer ..> Camera : renderで受け取る
    EnuTransform ..> TerrainVertex : transform()の結果をpositionへ
```

### 6.6 座標軸の扱い(ENU→wgpu)

ENU座標系(東=X, 北=Y, 上=Z)は右手系(East×North=Up)。wgpu/glamの一般的な慣習であるY-upとは軸の意味が異なるが、**頂点データ自体は変換せず**、カメラのビュー行列側で吸収する:

- ビュー行列は`look_at`のup方向として`Vec3::Z`を明示的に渡す(2D=真上からの表示だけは視線がZ軸と平行になり特異点になるので、北(Y軸)を上にする)
- 射影行列はwgpu/Vulkan/Metal互換の深度範囲0〜1(glamの`directx`系)を使い、**反転Z+有限far**にしている(理由は[第2部](impl/2_lod_camera_los.md) 1.4。
  `screen_to_ray`(ピッキング)・水域の視線も反転Zと整合させる)。OpenGL互換の[-1,1]は使わない
- ENUがそもそも右手系であるため、`_rh`系の関数とそのまま整合し、軸の入れ替えによるハンドネス反転を気にする必要がない

**カメラが地面の下にもぐらない制限**(3Dモード): `OrbitCamera::keep_above_ground`が、視点(カメラ位置)を、その真下の地面の高さ+`MIN_EYE_CLEARANCE_M`(30m。最細の地形のセルと同程度)以上に保つ。
地面の高さは、呼び出し側(`ui::terrain_view::keep_camera_above_ground`)がいま画面に出している地形から`heightmap::ground_at_enu`で引く(地球の丸み込み、海・データ範囲外は海抜0m)。
カメラ操作・原点変更・LOD切り替えのあとの描画の前に必ず通る。方針:

- まず**距離を保ったまま仰角を上げる**(ドラッグで下へ回したとき、地面の高さで止まる操作感)。仰角は視点の水平位置と一緒に変わって真下の地面の高さも変わるので、数回繰り返して収束させる
- 仰角を最大まで上げても届かない場合(近すぎるズームイン・注視点の周りが高い地形)は、視点を真上に持ち上げ、注視点との距離・仰角をそこから求め直す(水平位置は変えない)
- 仰角は水平より下向き(注視点を見上げる)にもなりうる(谷底から山頂を見上げるのは地面の上なので許す)。以前は注視点の高さを基準に`MIN_PITCH`だけで制限していたため、下へ回すと視点が地面の下に入り、地形の裏側が見えていた
- 2Dモード(正射影)は視点の高さが見た目に関係しないので何もしない

正確な式は[第2部](impl/2_lod_camera_los.md) 1.3。

### 6.7 標高グラデーション配色と海の扱い

標高を`elevation_max`で正規化(下限は0m固定)し、5点のカラーストップ(低地=深緑 → 緑 → 黄土 → 茶 → 山頂=白に近い明色)で線形補間する。色の値・式は[第1部](impl/1_data_and_geodesy.md) 6.2。

**海(データなし)の扱い**: `NO_DATA`のノード(1.4節)は描画しない。頂点positionはNaNだと破綻するため標高0mで配置するが、`mesh.rs::grid_indices`は
この頂点を1つでも含む三角形をインデックスに加えない(海岸線は最大1グリッドセル分陸側に退く。細かいレベルほど小さい)。
海と実データ範囲の外側には、地形メッシュより先に描く水域レイヤー(6.10節)が水色で見える(視線が楕円体に当たらない空は`TerrainRenderer::render()`のクリア色の黒)。
三角形の有無はグリッドだけで決まり原点に依存しないため、原点変更で再構築してもインデックス数は変わらない(`update_vertices`は頂点バッファだけを書き換える)。
`heightmap::sample_heightmap`(カメラ注視点の高さ・見通し範囲・クリック位置の判定で共用)は、`NO_DATA`を含むセルの双線形補間を標高0mへ丸める。

> 経緯: かつては海を水色で塗り、実データ外側を覆う背景スカートと「海を表示」切り替えがあったが、要望で一度撤去し、その後楕円体の水域レイヤー(6.10節)として再実装した(DEVELOPMENT_HISTORY.md参照)。

### 6.8 頂点シェーダ・フラグメントシェーダ・アンチエイリアス

シェーダー(`terrain.wgsl`・`draw.wgsl`)の**全文と、uniform・頂点のバイトレイアウトは実装仕様 [第3部](impl/3_renderer_and_shaders.md)**(実ファイルと`scripts/sync_impl_wgsl.py`で同期している)。
`terrain.wgsl`は、カメラの`view_proj`と陰影のON/OFFフラグをuniform(`@group(0) @binding(0)`)で受け取り、頂点位置を変換して、頂点色に陰影を掛けて出力する。
エントリポイントは、地形本体(`vs_main`/`fs_main`)・覆域ドーム用の固定半透明(`fs_dome`)・水域(`vs_fullscreen`/`fs_water`)・縮小(`fs_downsample`)。

**陰影(ヒルシェード)**: 色が標高のグラデーションだけだと、30mの細かい起伏(尾根・谷・斜面の向き)が見分けにくいため、地形の法線と固定の光源から明るさを求めて頂点色に掛ける
(表示メニューの「陰影表示」でON/OFF、既定ON)。

- **法線**: 頂点に`normal_xy`(単位法線のEast・North成分、snorm16x2の4バイト。Up成分はシェーダーが復元)を持たせる。ノードの東西・南北の隣のノードの位置の差(中心差分)の外積から求める
  ([第1部](impl/1_data_and_geodesy.md) 6.4)。位置は丸める前のf64を使う(原点から遠いタイルでf32だと隣との差が誤差に埋もれてざらつくため)。
  海のノード・覆域ドームなど陰影を付けない頂点は`TerrainVertex::UNLIT_NORMAL`(単位法線ではありえない値。x=y=0は真上=平地なので別の値)にし、シェーダーが見て陰影を掛けない
  (マーカー・作図・航跡は別のシェーダー`draw.wgsl`)
- **光源**: ENU座標で固定(北西から仰角45度)。カメラの向きに依らず、地図の陰影の一般的な向きになる。2D表示でも同じ光源で、陸地測量図のような見た目になる
- **明るさ**: 環境光(0.35)と`n・L`の和を、平地(法線が真上)で1になるよう正規化する。平地はON/OFFで色が変わらず、光源側の斜面は最大約1.25倍、反対側は最小約0.43倍
- **切り替え**: ON/OFFはuniformの値で、メッシュの作り直しは不要。状態は`terrain::hillshade::HillshadeState`(context。`TerrainView`が購読して`TerrainRenderer::set_hillshade`)
- 頂点は24→28バイトになった(常駐頂点が約2500万なので約100MB増)。陰影は頂点ごとに求めて補間する(明るさは法線について線形なので、フラグメントごとに求めるのとほぼ同じ)

**MSAA(4x)+スーパーサンプリング(2x)**: メッシュの細分化後、遠景で多数の細かい三角形が1画素に収まりきらず、エイリアシング(市松状の斑点・ちらつき)を起こしたため導入した
(「地表面上に水色の点がいっぱい」「ズーム・カメラ操作時にちかちかする」という報告への対処)。

- すべてのパイプライン(地形・水域・覆域ドーム、および作図・マーカー・航跡が使う`draw.wgsl`系)で`multisample.count = 4`にし、深度テクスチャも同じサンプル数にする
- WebGPUで仕様上必須なのは1と4のみで、8倍は実装依存(実機で`createTexture`が明示的にエラーになることを確認)。そのため倍率は4のまま、**内部解像度をcanvasの`SUPERSAMPLE_FACTOR`(2)倍にして描画し、最後に線形フィルタで実際のcanvas解像度へ縮小する2パス構成**にした
  (ちょうど2倍なのでバイリニア補間がそのまま2×2画素の平均になる。`TextureBlitter`は使わず自前の縮小パス)
- 内部テクスチャの一辺は`SUPERSAMPLE_MAX_DIMENSION`(4096px)で頭打ち(`maxTextureDimension2D`の最小値8192に対し、非常に大きなcanvasで2倍すると際どいため)
- 効果: 修正前に陸地内で水色と完全一致する画素が複数あった領域を、ピクセルサンプリングで再検証して1200画素中0画素まで減少。**ただしカメラがほぼ水平に近い極端に浅い角度では、2倍でも弱い縞模様が残る**
  (完全な解消にはLOD的な仕組みかより高い倍率が必要。未対応。CLAUDE.md「既知の技術的負債」)

ハマりどころ: `trunk serve`はpath依存先(`sim3dview`)のソース変更を自動では検知しない。ライブラリ側だけを編集したら`trunk serve`を再起動する。

### 6.9 見通し範囲(レーダー観測点、`terrain::los` / `terrain::markers` / `terrain::pick` / `LosView`)

任意の地点(レーダー観測点)を観測点として、全方位角(1mil刻み・6400方向。1周=6400mil、NATO式)の見通し限界距離を計算し、
(a) メインパネル(3D地形)上に覆域(3Dはドーム、2Dは塗り+輪郭線)、(b) ボトムステータスパネルの「見通し範囲」タブに2D極座標図(レーダー覆域図)、(c) 「断面図」タブに断面+覆域、の3か所で表示する。
アルゴリズム・定数・数式の正は実装仕様 [第2部](impl/2_lod_camera_los.md) 5章(計算)・[第4部](impl/4_overlays.md) 2章(描画ジオメトリ)。

**観測点の追加・管理**: 観測点は**メインパネル上の右クリックで追加する**(タブからの手入力ではない)。右クリックメニュー(7.7節)があればその項目「ここにレーダー観測点を追加」、
無ければ右クリックそのもので追加する。クリック位置の求め方は、画面座標からカメラの`screen_to_ray`でレイを求め、CPU側のheightmapに対してマーチングして地表との交点を探す(`terrain::pick::pick_lat_lon`。GPUの読み戻しはしない)。
観測点は**複数個**追加でき、一覧は`markers::RadarMarkersState`(全パネル共有のcontext)が保持する。「見通し範囲」タブの一覧で、選択(3Dのハイライト色・極座標図の対象が連動)・アンテナ高(既定10m)と最大観測範囲(既定50km)のその場編集・削除ができる。

**観測点は地形メッシュの原点(`OriginState`)とは独立**: 緯度経度の絶対値で保持しており、`set_origin`コマンドは一切送らない。原点(メッシュ)が変わっても観測点の緯度経度は変わらず、
描画側が新しい原点基準のENU座標へ再変換するだけで、地形メッシュの再計算も発生しない。したがって、原点変更とは異なり**シミュレーション実行中でも自由に追加・編集できる**
(3.4節の「原点変更はシミュレーション停止中のみ」は`OriginState`自体の変更にのみ適用される)。

```mermaid
flowchart LR
    A["メインパネル右クリック<br/>(screen x,y)"] --> B["Camera::screen_to_ray<br/>ENU座標系のレイ"]
    B --> C["pick::pick_lat_lon<br/>heightmapに対してレイマーチング"]
    C --> D["RadarMarkersState::add<br/>(lat,lon)→RadarMarker追加・選択"]
    D --> E["los::compute_los(data, marker, params)"]
    E --> F["markers::build_marker_geometry<br/>ピン(ビルボード)の頂点"]
    F --> G["TerrainRenderer::update_markers<br/>→ draw_blend_pipelineで描画"]
    E --> H["LosView: SVGの極座標図へ変換して描画"]
```

**計算モデル**

- **等価地球半径**: 標準大気中では電波が幾何学的な直線よりわずかに下向きに屈折し、実際の見通し距離は真球上の地平線より遠くなる。これを「地球半径をk倍した仮想の球面上で電波が直進する」近似で表す(標準大気でk=4/3、
  レーダー・無線工学の標準的な近似。v1では固定値でUIパラメータ化していない)。距離`d`での曲率による見かけの高度低下は`d² / (2 * R_eff)`
- **遮蔽判定(マスク角アルゴリズム、`compute_los`)**: 方位角ごとに観測点から外側へサンプリングし、各点の「見かけの仰角」(曲率低下を差し引いた角度)を求め、それまでの最大仰角以上の点だけを「観測点から直接見える」とする。
  見通し限界距離は見える点のうち最も遠いもの(手前の尾根の陰でも、その先で地形が十分高くなれば再び見えるケースを許容。単純な「最初の遮蔽物で打ち切り」より実際のレーダー覆域に近い)
- **3D覆域は半球状の面**(`compute_los_dome`): 仰角ごとに、観測点からの**直線(仰角一定のレイ)**が地形に遮蔽されずどこまで届くかを求める。直線は一度地形にぶつかったら、その先で地形が下がっても二度と陰から出てこないため、
  `compute_los`とは違って**最初に遮蔽された時点で打ち切る**のが正しい(簡略化ではなく、レイが地表を這う`compute_los`とは幾何学的設定が違うため)。遮蔽されない方角では、どの仰角でも同じ半径=滑らかな球面で、地表付近だけ地形の遮蔽で内側に凹む
  (当初案の「`compute_los`の値を全仰角に使い回す」簡易的なextrudeは、遮蔽されない方角でも仰角で半径が変わらず物理的に不自然だったので改めた)
- **2D覆域は「指定した海抜高度での探知可能領域」**(`compute_coverage_area`): `compute_los_dome`と対になり、仰角一定ではなく**高度一定の直線**を対象の高度について走査する。選択中の観測点についてのみ表示する点は3Dと共通。
  表示する海抜高度はメニュー「設定」→「覆域高度設定...」で指定する(既定1000m。`RadarMarkersState::coverage_altitude_m`、7.7節)
- **性能**: 観測点1つあたり6400方位×1000サンプル=640万回の`heightmap`補間(選択中の1つだけ計算。目標は再計算+再描画が1秒以内、リリースビルドのブラウザ実測で3Dが約470ms・2Dが約230ms)。
  速度対策(いずれも結果は変えない): `EnuTransform`が原点の三角関数・曲率半径を`new`で前計算する、2D覆域の塗りと輪郭線で計算結果を共有する、`compute_los_dome`が走査中の最大仰角の単調性を使って
  全リング×全サンプルの走査を避ける(素朴実装と完全に同じ結果になることを比較テストで確認)

**3D描画とジオメトリの方針**(定数・頂点構成は[第4部](impl/4_overlays.md) 2章)

- **マーカー**: 画面サイズ固定のピン(縁取り+本体+中の点。選択中は黄色、非選択はオレンジ)。観測点の位置(地表+25m)をアンカーに、画面のpxでずらすビルボード(`DrawVertex::billboard`)なので、拡大・縮小・回転しても同じ大きさで正面を向く。
  作図(6.11節)と同じ`draw_blend_pipeline`・絶対座標のuniformで、深度テストあり(アンカーの深度をクリップ空間で距離の0.2%手前へ寄せ、粗いLODの地形に埋まって消えないようにしている。遠くの山の陰には隠れる)。
  以前は地表の四角い枠(`LineList`)だったが、ズームで大きさが変わって位置が分かりにくいため置き換えた
- **覆域ドーム**: 選択中の観測点についてのみ(複数を重ねると見づらいため)、仰角0°〜87°の38段の緯度リングを、隣接リング×隣接方位ごとの四角形パッチでつないだ半透明の面(ワイヤーフレームではなくSurfaceを持つ多面体)。
  専用の`dome_pipeline`(アルファブレンド有効・深度書き込み無効。地形やマーカーの奥に透けて見えるように)で描く。色は半透明の水色固定(`fs_dome`)
- **滑らかにするための処理**: (1) 遮蔽されなかったリングの半径をサンプル位置に丸めず上限ちょうどにする(丸めると高仰角ほど半径が不揃いで球にならなかった)、
  (2) 半径を方位角方向に平滑化する(3方位メディアン→6方位平均。1方位だけ遮蔽される所が細い三角形になって放射状の筋に見えるのを防ぐ。遮蔽されない方角の半径は変わらない)、
  (3) リング数を増やし間隔を最上部まで細かくする。計算は1mil刻み(6400方位)のままで、平滑化後の描画は2方位に1つ(3200方位)に間引く
- **ちらつき対策**: 最上段リングを閉じる傘の部分は48分割に間引く(全方位で三角形化するとアペックスへ極端に細い三角形が大量に重なり、アルファブレンドの描画順依存でカメラ角度ごとに明るさがちらつく)。
  リング間の四角形は重ならないので間引かない。また、遮蔽される方角ではドーム境界が定義上ちょうど地形表面に接してZファイティングするため、ドーム全体を一律に20m持ち上げる(`render_bias::DOME_M`)
- **2D覆域の塗り**: 星形(観測点中心の極座標の境界なので自己交差しない)の塗り(半透明、アルファ0.32)+外周の輪郭線(不透明・太さ2.5px)。**深度テストなし**(`draw_screen_pipeline`)で描く。
  以前は深度テストありで描いていたが、塗りの三角形は観測点から境界への長い平面で間の起伏(数百m)に埋まり、場所によって塗りが欠けていた。2Dは真上からの正射影で地形に隠れることがないので、深度テストを外して均一に塗られるようにした。
  3Dドームとはバッファ・パイプラインが別で、`ui/terrain_view/overlay.rs::rebuild_markers`がモードに応じてどちらを作るか切り替える(使わない方は空)

**メインパネルの2D/3D表示切り替え**: メインパネル右上の「2D表示に切替」/「3D表示に切替」ボタンで、`terrain::camera::ViewMode`(`ThreeD`/`TwoD`)を切り替える。
3Dは自由視点(透視投影、ドラッグで回転)。2Dは真上からの正射影で、北を上に固定した地図のような見た目になり、ドラッグは回転ではなく`OrbitCamera::pan`による平行移動になる
(`up`が北=Yなので回転操作自体が意味を持たない)。ホイールズームは`OrbitCamera::distance`を正射影の画面縦幅(m)として再利用する。
かつて俯瞰/側面のプリセットボタンがあったが、要望で削除した(2D↔3Dはこのボタンだけ)。

### 6.10 地形LOD(タイルとチャンクごとの解像度レベル)と水域レイヤー

対象域が30度四方に広がったため、単一メッシュでは解像度が足りず(1セル約1.6km)、頂点数を増やすとWebGPUのバッファ上限を超える。
そこで**1度タイル単位でLODを切り替え、近いタイルは6×6のチャンクに分けてチャンクごとに解像度を選ぶ**(2.4節のレベル定義。最細は元データの30m)。
計画の正確なアルゴリズム・定数は[第2部](impl/2_lod_camera_los.md) 2章、適用ループ(取得・メッシュ生成・差し替えのスケジューリング)は[第5部](impl/5_ui_and_integration.md) 4章。

- **描き方**: 全タイルを**6×6のチャンクのメッシュ**で描き、チャンクごとに独立してレベル(1〜4)を持つ。**最も粗くてもレベル1(約620m/セル)**で、遠くのタイルもこれより粗くはしない
  (以前は遠いタイルをレベル0=約1.85km/セルのタイル全体1枚で描いていた)。起動直後だけは全タイルのレベル0(合計約155万頂点)を`base.bin`から作って全体1枚で描き、
  レベル1のグリッド(1タイル約69KB、390個)が取得できたタイルから近い順にチャンク表示へ切り替える(取得は約20秒、メッシュ生成を含めた全タイルの切り替えは約40秒。その間はレベル0のタイルが混じる)。
  状態は`terrain::lod::TileLayout`(`Whole` / `Chunks(チャンクごとのレベル)`)
- **計画**(`terrain::lod::plan_levels`。GPU・ネットワークに触れない純粋関数): タイル・チャンクごとに、視点(2Dでは注視点)から最も近い点までの距離と、視錐台に入るかを求める。
  理想のレベルは、画面(CSSピクセル)上で**1セルが1ピクセル以下**になる最も粗いレベル。30mのレベル4が使われるのは、1ピクセルが約62m未満になる距離(canvas高さ700pxで視点から約46km以内)。
  理想より1つだけ細かい現状のレベルは下げない(ヒステリシス。ちらつき防止)、視野の外は現状維持。
  遠い多数のタイルは、タイルの一番近い点でも理想がレベル1なら、チャンクごとの計算を省く(毎回計算すると重いため)
- **頂点予算**: チャンクの頂点の合計は2500万以下。まず全タイルに全チャンクをレベル1で載せる下限の分(約1520万頂点)を近い順に確保し(足りなければそのタイルは全体表示のまま)、
  次に、見えていて近いチャンクから順に、目標レベルまで残り(約980万頂点)の許す限り上げる。最細のチャンクは約36万頂点なので、同時に最細にできるのは約27個まで
  (GPUメモリは頂点28バイトとインデックスで合計約1.2GB)。予算が足りない遠い側は目標より粗いままになる(近い側が優先)。予算は300万→600万→2500万(全タイルの下限を含む)と増やしてきた
- **視錐台カリング**: 全タイルを常駐させると画面外の頂点が大量になるため、`TerrainRenderer::render`がメッシュごとの頂点位置を囲む直方体(`MeshGpu::bounds`。原点変更時に更新)が
  クリップ空間の6面のどれかの外に完全に出ているものは描かない(GPUのクリッピングでも何も描かれないので見た目は変わらない)
- **適用**(`ui::terrain_view`の`update_lod`): カメラ操作からの予約は150ms待ってまとめ、ドラッグ中は後回しにする。取得の完了・メッシュ反映の続きからの予約は8msだけ待って続ける
  (150msを待つと取得の補充が周期的に間延びし、全タイルのレベル1の取得だけで約4.7秒かかっていた)。必要なグリッドが取得済みならメッシュ(頂点+インデックス+縁のスカート)を作って`set_mesh`で差し替える。
  取得済みの範囲でより細かければ先にそこまで上げ、目標のグリッドが届いたらさらに上げる(レベル1→2→3→4と段階的)。1回の更新でメッシュを作る量は時間の目安(12ms)で区切り、残りは次の更新へ回す(速い端末では1回で多く進み、遅い端末では自動的に細かく刻まれる)
- **取得**: レベル2以下はタイル1枚分のファイルをまとめて取得してチャンクごとに分割し、レベル3以上はHTTP Rangeでチャンク1個分だけ取得する(最細で約0.7MB)。同時16件まで
  (ブラウザの同一ホストへのHTTP/1.1接続は通常6本だが、全タイルのレベル1を起動後に取得するので6件だと1分ほどかかった)。失敗は再試行しない。取得済みグリッドは合計300MBまで残し、超えたら画面に出していないものから古い順に捨てる
- **標高サンプリングとの一致**: 各チャンクの「いま画面に出しているレベル」(`TerrainData::set_chunk_level`)のグリッドで`sample_heightmap`が標高を引く。描画されている地形と、
  観測点・見通し計算・クリック判定・注視点の高さが一致する。細かいレベルに切り替わった直後は、注視点の高さと観測点・覆域を合わせ直す
- **継ぎ目**: 解像度の違う隣のメッシュ同士は縁のノードの高さが食い違い、隙間から背景の黒が見える。各メッシュの縁から下向きの壁(スカート。深さはレベル0が800m、以降400/250/150/100m)を付けて隠す。
  同じレベルのチャンク同士は縁のノードを共有するので継ぎ目は一致する

**水域レイヤー**(「マスクファイルの水域と地形データの範囲外を水色で表示。標高は海抜0m。WGS84の丸みを考慮し、地形を隠さないように。標高が0m以下の地形の上にも水域が来ないように」との要望で追加。
以前はNaNセルの水色塗り・背景スカートというメッシュ方式で実装したが、z-fightingや縞状の透けが問題になり撤去した。DEVELOPMENT_HISTORY.md参照):

- **水域の範囲**: 「地形メッシュが張られない所」がそのまま水域になる(マスクの水域・存在しないタイル・タイル内のNODATA・地形データの範囲外)。マスクの水域を別に持つ必要はない
- **面はWGS84楕円体の海抜0m**(メッシュではなく解析的な楕円体): メッシュ(緯度経度の格子)だと、セルが弦になって面がセルの中央で凹み(1度で約240m、0.25度で約15m)、地形との継ぎ目や範囲の限り(遠景で途切れる)が問題になる。楕円体なら丸みが厳密で、範囲の限りもない
- **描き方**: 画面いっぱいの三角形1枚を、地形メッシュより**先に**描く(`water_pipeline`。自身は深度テストせず常に描く)。フラグメント(`fs_water`)が各画素の視線と楕円体の交点の有無を求め、あれば水色、なければ`discard`(空は黒のまま)
- **水域の深度と、地形が隠れない余裕**: 水域が深度を書かないと、水面の下・地球の丸みの向こう側にある地形まで水面越しに描かれてしまう(海面すれすれの低い視点から遠くの陸地を見ると、丸みで本来は隠れるはずの陸地が海の中に透けて見えた)。
  そこで、楕円体(地球本体)を不透明な物体として、交点の深度を書く。ただし書く深度は、交点から視線に沿って`WATER_DEPTH_MARGIN_M`(1,000m)奥へずらした点のもの。
  地形は交点から1km以内の奥までは水域より手前として描かれるので、**標高が0m以下(DSMの負の値)の地形が水域に隠れることはない**(標高-Hの地形と視線が角度θで交わるとき、交点との視線方向の距離はH/sinθ。-10mでもθ≧約0.6度まで隠れない)。
  地球の丸みの向こう側の地形は水平線から数km以上奥になるので隠れる。楕円体に当たらない視線(水平線より上を通って見える山頂など)は`discard`されるので、地形はそのまま描かれる。ドーム・マーカーは地形と同じく深度でテストされる
- **地形の縁の壁(スカート)は海抜0mまで届かせる**: 水域の深度が隠せるのは海抜0mより下を通る視線だけ。データ範囲の端など標高の高い縁の下に隙間が残ると、縁の外の低い視点から水域の面より上を通ってその下(地形の裏側)が見える。
  そのためスカートの底は、`skirt_depth`だけ下げた高さと海抜0mの**低い方**にした(`mesh.rs::grid_vertices`)
- **視線と楕円体の式**: 視線は`Camera::water_ray_basis`(`screen_to_ray`と同じ式。逆VP行列はf32の丸め誤差で向きが大きくずれるため使わない)。楕円体は陰関数`f(p) = c0 + 2 g・(M p) + |M p|²`で、
  原点が楕円体上(h=0)なので定数項が(丸め誤差の)`c0`だけになり、絶対座標約6.4e6mのままf32で二乗する場合のような桁落ちがない(係数は[第1部](impl/1_data_and_geodesy.md) 4.4)。
  `f≒2h/a`なので、原点から遠い点(石垣島・約2,000km)でも海抜0mで誤差0.1m以下、-30m・500m・3,000mも一致することを合成テストで確認した
- **原点変更**: 係数は原点基準なので、頂点バッファを作り直す場面(初期化・原点変更)で`TerrainRenderer::set_ellipsoid_origin`を呼ぶ
- 色は`WATER_COLOR`(0.25, 0.55, 0.85)。地形の低地の緑と区別できる水色の単色

### 6.11 作図(図形・線、`terrain::drawing` / `terrain::drawing_geometry` / `draw.wgsl`)

アプリが任意の図形・線を地図上に出すための汎用機能。レーダー観測点(6.9節)のような用途固定のものとは別に、2D図形・3D図形・折れ線を、塗り・枠線・不透明度つきで、**絶対座標に固定**するか**カメラに固定**するかを選んで描ける。
通信プロトコルは一切知らない(アプリが`DrawingState`を`provide_context`して出し入れする)。データモデルは[第4部](impl/4_overlays.md) 3章、ジオメトリ生成は4章、対話作成は5章。

**データモデル**(`terrain/drawing.rs`)

- `Drawing { id, shape, style, visible }`の一覧を`DrawingState.items`(`RwSignal<Vec<Drawing>>`)が持つ(`add`/`update`/`remove`/`clear`)。`TerrainView`が購読して描き直す(未提供なら作図なし)
- `Shape`: 2D図形=`Circle`/`Rect`(回転可)/`Polygon`(凹も可)/`Sector`(扇形)、3D図形=`Sphere`/`Cuboid`/`Cylinder`/`Cone`、線=`Polyline`。回転角・方位は時計回り・0度が上(北)
- `Style { fill, stroke, stroke_width_px }`。色はRGBA(アルファ<1で半透明)。2D図形は塗り+輪郭線、3D図形は面の塗り(陰影つき)+稜線、折れ線は`stroke`だけ
- 位置は`Position`の3種類(**1つの図形の中では同じ種類にそろえる**。混ぜた図形・`Screen`に置いた3D図形は`Shape::validate`が弾いて描かず、警告ログを出す):

| 種類 | 座標 | 固定先 | 奥行きの扱い | 置ける図形 |
|---|---|---|---|---|
| `World` | 緯度経度+`Altitude` | 絶対座標(原点変更・LOD切替に追従) | 地形と同じパスで深度テスト(山の陰に隠れる) | すべて |
| `View` | カメラからの右・上・前方(m) | カメラ | 地形の手前に別パスで描き、図形どうしは深度で隠し合う | すべて |
| `Screen` | 角(`Corner`)からのpx | 画面(canvas) | 地形の手前、追加順に重ねる(深度なし) | 2D図形・線 |

- `Altitude::Msl(h)`: 海抜(地形データの標高と同じ基準)。2D図形は**その高さの水平な面**(地球の丸みには沿う)。`Altitude::AboveGround(o)`: 地表から。2D図形・線は**地形の起伏に沿って貼り付く**(細分化して描く)。
  3D図形は位置の真下の地表を基準に置くだけで変形しない。地表に貼り付くものは、地形メッシュとのZファイティングを避けるため15m持ち上げる(`render_bias::DRAWING_M`。観測点・覆域と同じ考え方で、値の一覧は`terrain::render_bias`。[第4部](impl/4_overlays.md) 1章)
- 大きさ(半径・幅等)の単位は`World`/`View`ではm、`Screen`ではpx

**ジオメトリ生成の方針**(`terrain/drawing_geometry.rs`、純粋関数。`build(ctx, drawings) -> DrawingBatches`)

- 面も太い線も`DrawVertex`(位置・RGBA・aux・params、56バイト)のTriangleListで表す。頂点列は座標の種類ごと(`world`/`view`/`screen`)、さらに`world`/`view`は不透明(深度を書く)と半透明(書かない)に分ける
- 2D図形は、置いた位置を中心とするローカル平面(x=右/東, y=上/北)で三角形と輪郭線を作り、種類ごとの変換で出力座標にする。`World`では、ローカル座標を基準点からの方位・距離(方位角等距離図法、球面の直接解)とみなして緯度経度へ戻し、
  `Altitude`から高さを決めて`EnuTransform`でメッシュ原点のENUへ変換する。多角形は三角形分割(`earcutr`)+辺の長さ上限での分割。分割の細かさは海抜の水平面で辺20km・輪郭2km、地表貼り付けで辺500m・輪郭250m
- 3D図形は位置における局所ENU(位置を通る鉛直線が+z)で作り、`enu_to_geodetic`→`EnuTransform::transform`で厳密に変換する(遠方でも地球の丸みで傾いた上向きが正しい)
- `World`の折れ線は点の間を大円に沿って分割し、地表基準は「地表からの高さ」を補間する
- 標高は`ctx.ground`クロージャで受ける(`ui::terrain_view`は`heightmap::sample_heightmap`を渡す)ので、地形なしで単体テストできる

**太い線**(`terrain/draw.wgsl`): WebGPUの線プリミティブは太さ1pxしかないため、線分1本を四角形(三角形2枚)にして頂点シェーダーで**画面のピクセル幅**へ広げる。
各頂点は「この端点(`position`)」と「反対側の端点(`aux`)」を持ち、クリップ座標→ピクセル座標で向きと法線を求めてから`params.x`(px)の半分だけ左右へ、端点は線の向きへ半幅だけ延ばす(折れ線のつなぎ目の隙間を埋める)。
視点の後ろの端点は線分をそこで打ち切る(透視投影で向きが反転して帯が壊れるのを避ける)。線は`LINE_DEPTH_BIAS`だけ手前に寄せて深度テストし、同じ位置の面とのZファイティングを避ける。
3D図形の面は法線からLambert風の陰影(絶対座標は地形と同じ北西・仰角45度、視点空間はカメラの左上手前)を掛ける。半透明の線は、折れ線のつなぎ目の重なり部分だけアルファが二重に掛かって濃く見える(不透明なら出ない)。

**描画パス**: 1つ目のパス(地形)は 地形→マーカー→`World`不透明→覆域ドーム→`World`半透明。カメラ固定(`View`/`Screen`)が1つでもあるときは**2つ目のパス**で描く:
1つ目のMSAAカラーを`Load`で引き継ぎ、深度だけ`Clear`し直して地形と隠し合わないようにする(`View`は`Camera::projection_matrix`=ビュー行列なしの射影、`Screen`はピクセル→クリップの行列で深度テストなし)。
パイプラインは不透明(深度書き込みあり)・半透明(なし)・画面(深度テストなし)の3本で、シェーダー・bind groupは地形とは別。詳細は[第3部](impl/3_renderer_and_shaders.md) 3〜4章。

**再構築のタイミング**(`ui/terrain_view/overlay.rs::rebuild_drawings`): 一覧の変更・原点変更・canvasのリサイズ(`Screen`の角の位置が変わる)、および地表基準の図形があるときの地形LOD切替。

**図形の対話作成**(`terrain/draw_tool.rs` / `ui/drawing_editor.rs`。仕様は[第4部](impl/4_overlays.md) 5章、[第5部](impl/5_ui_and_integration.md) 5.8)

UIから図形を作る・編集する層。`DrawingState`の上に載る別のcontext`DrawToolState`で、ライブラリは通信もアプリ固有のUIも持たない。作る図形はすべて絶対座標(`Position::World`)。

- **作り方**: パネルでツール(円・矩形・多角形・扇形・折れ線・球・直方体・円柱・円錐)を選び、地図を左クリックして点を置く。クリック数は図形ごとに固定(円・球・円柱・円錐=中心→円周上の点、矩形・直方体=対角の2点、扇形=中心→開始側の縁→終了側の縁)で、
  多角形・折れ線だけ何点でも置いてダブルクリック/Enter(`確定`ボタン)で確定する。確定後もツールは選んだままで、続けて置ける。右クリック/Backspace=置いた点を1つ戻す(点が無ければツール解除)、Esc=ツール解除。地形データ範囲外のクリックは無視する
- **仮の図形**: 作成中はカーソル位置までを含めた図形(作れなければ点をつないだ折れ線)を`DrawingState`に**仮の1要素**として置き、クリック・カーソル移動のたびに更新する(確定・取り消しで消す)。
  カーソル移動は`TerrainView`が1フレームに1回へまとめる(更新のたびに作図全体の再構築+描画が走るため)
- **一覧と選択**: `DrawToolState.shapes`(作った順の`UserShape { id, name }`)が、このエディタで作った図形を区別する(アプリが`DrawingState`へ直接足したデモ等は一覧に出ず、保存もされない)。
  一覧で選ぶと地図上で**黄色い太線の枠**になり、下に数値編集フォームが開く(位置・大きさ・高度・見た目・名前・表示/非表示・削除)。地図上のドラッグでの頂点移動やシンボルのクリック選択は未実装
- **保存**: `DrawToolState::persist(key)`で、作った図形をJSON(`{version, shapes}`)にしてブラウザのlocalStorageへ保存し、次回起動時に復元する(内容が変わったときだけ書く)。版が違う・壊れたデータは警告ログを出して読み込まない
- **UI**: `ui::drawing_editor::DrawingEditor`。地図をクリックできるよう、モーダルではなく非モーダルの`FloatingPanel`(`modal=false`、7.7節)かタブの中身として置く(サンプルは表示メニューの「作図...」で開く移動可能なウインドウ)。
  作成中の案内と`確定`/`1つ戻す`/`終了`は、地図の左上に重ねるヒントバーに出す

### 6.12 航跡(トラック)表示(`terrain::tracks` / `draw.wgsl`の向きつきビルボード)

シミュレーションなどから受け取った航空機・艦船・車両等の現在位置を、シンボル・ラベル・航跡(軌跡)・高度線で表示する。ライブラリは通信プロトコルを知らず、アプリが`TracksState`(context)へ最新の一覧を`set`する
(サンプルでは`sample/sim_frontend/src/track_bridge.rs`が`protocol::TrackList`を変換して反映する。プロトコルは4.3節)。モデル・ジオメトリ・当たり判定の仕様は[第4部](impl/4_overlays.md) 6章。

**データモデル**(`terrain/tracks.rs`)

- `Track { id, kind: SymbolKind, affiliation: Affiliation, label, lat_deg, lon_deg, altitude: Altitude, heading_deg, speed_mps }`。`SymbolKind`=不明/固定翼機/ヘリ/艦船/地上車両/ミサイル(形が変わる)、
  `Affiliation`=不明(黄)/友軍(青)/敵(赤)/中立(緑)(色が変わる)。高度は作図(6.11節)と同じ`Altitude`(サーバーが地形の高さを持たない車両などは`AboveGround`)
- `TracksState { entries, show_labels, show_trails, show_altitude_lines, selected }`。`set(Vec<Track>)`は受信のたびに全件を渡す(前回に無いIDは新規、今回に無いIDは航跡ごと消える)。
  航跡は同じIDの間だけ、前回の位置が最後の点から250m以上離れていれば足していく(上限400点)

**シンボル**(向きつきビルボード): 種別ごとの形を、進行方向が+y・右が+xのポリゴンで持ち、三角形分割(`earcutr`)して縁取り(暗色)→本体(所属の色)の順に積む。
位置(高度込み)をアンカーに、頂点は画面のpxでずらす**画面サイズ固定のビルボード**(マーカーのピンと同じ仕組み)。さらに`draw.wgsl`の**向きつき**(`params.z=2`)は、アンカーとアンカーから進行方向(ENUの水平)へ200m進んだ点を射影して、
進行方向が**画面上で実際に指す向き**を求め、その向きへ形を回す。3Dでカメラを回しても、2Dの地図でも、シンボルが実際の進行方向を向く(真上・真下から見て向きが画面に現れないときは画面の上向き)。深度は覆域マーカーと同じ扱い(アンカーの深度を距離の0.2%手前へ寄せる)。

**航跡・高度線・ラベル**

- 航跡: 過去の位置→現在位置の折れ線(太さ1.5px、所属の色、アルファ0.55。6.11節の太い線)。高度線: 現在位置から真下の地表へ下ろす細い線(1px)。地表から30m以下は出さず、3Dのみ(2Dは真上から見て点になる)
- ラベル: 名前+「高度 m 速度 km/h」(地表基準は`AGL `つき)。WebGPUには文字を描く機能がないので、`TerrainView`が`canvas`の上に重ねるHTML要素(`.terrain-track-labels`内の`.track-label`)で、
  毎フレーム(`render_frame`→`update_labels`)アンカーを画面へ射影して`transform`で動かす。カメラの後ろ・画面の外は非表示。トラック数が変わったときだけ要素を作り直し、文字は変わったときだけ更新する

**選択(クリックで詳細を出す)**

- `TracksState.selected`が選択中のID。`select(Option<id>)`で選択・解除、`set`で選択中のトラックが一覧から消えたら自動で解除する
- 当たり判定: `TerrainView`の`pointerup`で、ドラッグではない左クリック(移動が5px未満)のとき、「原点クリック指定」モード中なら従来どおり原点指定、そうでなければ`tracks::pick_track`
  (各シンボルのアンカーを画面へ射影し、クリック位置に最も近く半径20px以内のもの。地形の陰に隠れたシンボルも対象)で選ぶ。**何もない所のクリックは選択解除**
- 強調: 選択中のシンボルの後ろに白い輪を描き、ラベルに`.selected`クラスを付ける
- 詳細の表示はアプリの役目。ライブラリは`selected`と`selected_track()`、種別・所属の日本語名だけを渡す。サンプルは、トップステータスパネルの**「航跡情報」タブ**(`components/track_detail.rs`)に、
  名前・識別番号・種別・所属・位置・高度・針路・速度・原点からの距離と方位・「選択を解除」ボタンを出し、`TabbedPanel`の`active`(任意のprop)で、航跡が選択されたら自動でこのタブへ移る

**描画・再構築**: `TerrainRenderer::update_tracks`(専用バッファ、`draw_blend_pipeline`・絶対座標のuniform)。再構築(`ui/terrain_view/overlay.rs::rebuild_tracks`)は、トラックの受信・表示設定の変更・原点変更・2D/3D切替・地形LOD切替(地表基準・高度線があるとき)。
トラックは高頻度で更新されるので、受信のたびに`render_frame`だけ呼び、LODの更新は予約しない。

**サンプル(デモ)**: `sample/sim_server`の`Simulation::make_demo_scenario`が、デフォルト原点(富士山の近く)のまわりに7つのトラック(友軍機・敵機・ヘリ(地表基準)・中立の艦船(駿河湾)・車両(地表基準)・不明機・ミサイル)を楕円軌道で周回させ、
`TrackList`として配信する。フロントは左パネルの「開始/一時停止」ボタン(`resume`/`pause`コマンド)でシミュレーションを進め、表示メニューの「航跡ラベル/航跡(軌跡)/高度線」で表示を切り替える。
自分のシミュレータへつなぐときは、シナリオの部分を自分のシミュレーション結果から`Track`を作る処理に置き換える。

---

## 7. UI詳細設計

サンプルアプリ(`sample/sim_frontend`)のUIの設計。本節が`components/xxx.rs`と書くものは`sample/sim_frontend/src/components/`のファイルで、
`ui/xxx.rs`と書くものは`sim3dview`ライブラリの汎用部品(実装仕様 [第5部](impl/5_ui_and_integration.md) 5章)。

### 7.1 レイアウト

```
┌──────────────────────────────────────────────────────────────┐
│ ファイル  設定  表示  ヘルプ                    ← メニューバー    │
├───────────────────────┬───────────────────┬───────────────────┤
│ シミュレーション          │                   │ トップステータスパネル │
│ ステータスパネル(上)      │                   │  [各種情報][航跡情報]  │
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

- 中段: `MID_ROWS`(4)行×(`mid_pages`×`cols`)列のダミーボタン。**ページ数は`VabPanel`の`mid_pages`プロパティで、パネルを置く側が決める**
  (`Signal<usize>`なので実行中に変えられる。省略時は2、0は1扱い。サンプルは`app.rs`で`mid_pages=2usize`)。**1なら単一ページで、ページ送りを出さない**。
  1ページあたり`cols`列だけを表示し、2以上のときは
  **「◀ 1/N ▶」のページ送りボタン(`.vab-pager`)で切り替える**(横スクロールバー方式は
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

**有効化状態(色反転)**(「vabについて通常状態と色を反転した有効化状態を用意したい」との
要望により追加): 全VABボタン共通の`.vab-button-active`(背景を`var(--accent)`、文字色を
濃紺に反転する見た目)を、区画ごとに異なるソースで切り替える。

- 先頭行(B1〜B4相当): 「フロント側で現在の選択状況を有効化表示する」との要望通り、
  既存の`selected_category`(カテゴリ選択の選択中インデックス)をそのまま使う
  (元々`.vab-button-selected`という名前だったものを、有効化状態の汎用クラスとして
  `.vab-button-active`に統合した)
- 中段・下段: 当初の要望は「それ以外はC++側からのステータスをもって切り替える」だったが、
  この2区画は選択中カテゴリに応じてフロント側だけで生成するダミーボタン(`vab_dummy_mid_{i}`/
  `vab_dummy_bottom_{i}`)であり、C++側はその存在自体を知らない(VabConfigは先頭行の
  `cols`個しか使っていない、上記参照)。C++からステータスを紐づけようがないため、
  「フロントエンド側で制御する方針に変更する」との回答を受け、区画ごとに直近クリックした
  ボタンのインデックスをローカルに保持(`selected_mid`/`selected_bottom`、いずれも
  `RwSignal<Option<usize>>`)して有効化表示するようにした。カテゴリ切り替え時
  (先頭行クリック時)はどちらも`None`にリセットする(前カテゴリでの押下状態を
  引き継がない)
- 実装上の注意: `.vab-button-active`と`.vab-button-dummy`は詳細度が同じ単一クラス
  セレクタのため、CSS上の宣言順で後ろにある方が勝つ。中段・下段では両クラスが同時に
  付与されるため、`.vab-button-active`を`.vab-button-dummy`より**後ろ**に定義する
  必要がある(`style/app.css`)

### 7.5 状況パネル仕様

- 表示項目(ラベル・単位・並び順)は`StatusPanelConfig`によりC++側が動的に決定する
- フロント側は表示項目をハードコードせず、`items`の定義通りに`SimState.status_values`を
  並べて表示する
- v1では数値項目のみを対象とする

### 7.6 タブ付きパネル(`ui/tabbed_panel.rs`)

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

ボトムステータスパネルへ「見通し範囲」タブ(6.9節)を追加した際に、「断面図」
「見通し範囲」の2タブ構成になった(タブ配列に`tab(...)`のエントリを1行足すだけで、
`TabbedPanel`側の変更は一切不要だった。設計意図通りの拡張性)。その後「ボトムステータス
パネルの側面図もいらない」との要望を受けて断面図タブ(`CrossSectionView`、旧称「側面図」)
自体を一時削除したが、後日「断面図機能自体を復活してほしい」との要望を受けて再度追加し、
現在は再び2タブ構成になっている(git履歴参照。復活時の実装は`sim3dview::ui::
cross_section_view::CrossSectionView`としてライブラリ側に置いている)。

- `active: RwSignal<usize>`で選択中タブのインデックスを保持する
- 各タブの中身(`AnyView`)は初回描画時に全タブぶん一度だけ生成してDOMに残し、
  非選択タブは`style:display="none"`で隠すだけにする(タブ切り替えのたびに
  作り直さない。Leptosの再マウントコストを避けるための一般的なパターン)
- タブが1個しかない場合でもタブバー自体は表示する(見た目の一貫性のため、
  タブ数によって表示/非表示を切り替えるような分岐は入れていない)

### 7.7 メニューバー・フローティングパネル(原点設定/覆域高度設定)・右クリックメニュー

画面最上部のメニューバー(`components/menu_bar.rs`)は「ファイル」「設定」「表示」
「ヘルプ」の4項目。「表示」配下には、地形の陰影のON/OFFを切り替える「陰影表示」(ONのとき項目の頭に✓、
6.8節)、カメラの中心点を原点へ戻す「中心点を原点に戻す」、航跡表示(6.12節)の「航跡ラベル」「航跡(軌跡)」「高度線」の
ON/OFF(`TracksState`の各`show_*`。ONのとき✓)、作図(6.11節)のデモ図形を出し入れする「作図デモ」(`components/drawing_demo.rs`。
消すときは自分が追加した図形のIDだけを消すので、ユーザーが作った図形は残る)がある。図形を自分で作る操作は、「作図...」(移動できる非モーダルのウインドウ。6.11節「図形の対話作成」)。
「設定」配下に2つのフローティングパネルを開く項目がある:

- 「原点設定...」: 原点入力フォーム(緯度・経度・`設定`ボタン、DETAILED_DESIGN.md
  3.5節のバリデーション込み)を`ui/origin_dialog.rs`(ライブラリ)として画面中央に表示する。
  フォーム自体の中身は実装当初シミュレーションステータスパネルに直接埋め込まれて
  いたものをそのまま移設したもので、ロジックに変更はない。サーバーへ`set_origin`
  コマンドを送るため`WsConnection`を必要とする
- 「覆域高度設定...」: メインパネルの2D表示モードで使う覆域表示の対象海抜高度
  (`terrain::markers::RadarMarkersState::coverage_altitude_m`、6.9節)を編集する
  `ui/coverage_altitude_dialog.rs`(ライブラリ)を画面中央に表示する。当初はメインパネル
  右上のインライン入力欄(2Dモード時のみ表示)だったが、「高度はメニューから
  フローティングウインドウで入力できるようにして」との要望を受けてこちらへ移設した。
  サーバーへは何も送らないフロント側だけのローカル表示設定のため`WsConnection`は
  不要で、`origin_dialog.rs`と見た目(`.origin-dialog*`のCSSクラスを共用)は同じだが
  実装ははるかに単純(バリデーションも送信ボタンもない、数値入力欄1つだけ)

```mermaid
stateDiagram-v2
    [*] --> 閉: 初期状態
    閉 --> 開: 設定→(原点設定/覆域高度設定)...をクリック
    開 --> 閉: ✕ / 背景クリック
```

- 開閉状態はそれぞれ`ui::origin_dialog::OriginDialogState`/`ui::coverage_altitude_dialog::CoverageAltitudeDialogState`
  (どちらも`RwSignal<bool>`の単純なラップ)を`provide_context`で共有し、
  `MenuBar`(トリガー)・各ダイアログ本体(表示)の双方が`use_context`で参照する
- メニューのドロップダウン・フローティングパネルとも、背景の透明な`.menu-backdrop`/
  半透明の`.origin-dialog-backdrop`をクリックすると閉じる(パネル本体のクリックは
  `ev.stop_propagation()`でバックドロップまで伝播させない)
- **`FloatingPanel`の種類**(`ui/floating_panel.rs`、ライブラリの汎用部品。上の2つはどちらも既定のモーダル):
  - `modal`(既定`true`): 半透明バックドロップ(`.floating-panel-backdrop`)が画面を覆い、中央にパネルを出す。背景クリックか✕で閉じる。
  - `modal=false`(**ウインドウ**): バックドロップなし。画面全体を覆う透明な層(`.floating-window-layer`、`pointer-events: none`)の上に、ウインドウ
    (`.floating-panel--window`、`pointer-events: auto`)だけがクリックを受けるので、背後(地図など)を操作したまま出しておける。✕でだけ閉じる。
    位置は`initial_position`(画面左上からの(x, y)、既定(80, 60))で決め、パネルが大きいときは本体(`.floating-panel-body`)がスクロールする。
  - `draggable`(既定`false`、モーダルにも使える): タイトルバー(✕以外)のポインタ操作で動かす。位置は`left`/`top`(初期位置。モーダルは中央)に対する
    `transform: translate`の移動量で持ち、ドラッグ開始時のパネルの矩形から「右端が80px以上・左端が(画面幅-80px)以下・上端が0以上・上端が(画面高さ-40px)以下」
    になる範囲へ移動量を制限する(タイトルバーを画面外へ出してしまい、つかみ直せなくなるのを防ぐ)。動かした位置は、閉じて開き直しても保つ(中身を作り直さないため)。
    ウインドウのリサイズ・最小化・複数ウインドウの重なり順・位置の永続化は未実装。
  - サンプルの「作図...」(`components/drawing_window.rs`。`DrawingWindowState`の開閉状態をメニューから立てる)は`modal=false`+`draggable`で、
    作図エディタ(6.11節)を地図の上に浮かせる。
- **右クリックメニュー**(`ui/context_menu.rs`、ライブラリの汎用部品。`FloatingPanel`と同じく、本体だけをライブラリが持ち、中身は使う側が決める):
  - アプリは`ContextMenuState`を`provide_context`し、`<ContextMenu/>`を1つだけ置く。出したい所から`ContextMenuState::show(x, y, items)`(client座標)を呼ぶ。
    項目`MenuItem`は、操作(`action`。`enabled`で無効にもできる)・サブメニュー(`submenu`。入れ子可)・見出し(`label`。押せない)・区切り線(`separator`)。
  - 画面全体を覆う透明な背景(`.context-menu-backdrop`、z-index 30)の上にメニューを描く。メニュー外の左/右クリック(そのクリックは背後へ通さない)・Esc・項目の選択で閉じ、
    項目は**先に閉じてから**`on_select`を呼ぶ(呼んだ先で別のメニューを出せる)。画面の右端・下端にはみ出すときは、描画後に測って収まる位置へずらす
    (それまでは`visibility: hidden`で、指定位置から一瞬ずれて見えるのを防ぐ)。サブメニューは項目にカーソルを乗せる/押すと右へ開き、右に収まらなければ左へ開く(`.flip`)。
    `copy_to_clipboard(text)`(`navigator.clipboard`。https/localhostのみ)も付けてある。
  - **地図の右クリック**: `TerrainView`が`MapMenuState(UnsyncCallback<MapMenuTarget, Vec<MenuItem>>)`と`ContextMenuState`の**両方**を`use_context`できれば、
    右クリックで`MapMenuTarget { position: 地表の(緯度, 経度)(範囲外・空ならNone), track: 右クリックした航跡のシンボル }`を求め、コールバックが返した項目でメニューを出す
    (どちらもNoneなら出さない。シンボルを右クリックしたらそのトラックを先に選択する)。どちらかが無ければ従来どおり、その地点にレーダー観測点を追加する。
    図形の作成中は、メニューではなく「置いた点を1つ戻す」(6.11節)を優先する。
  - **サンプルの項目**(`components/map_menu.rs`。ライブラリの各`State`を呼ぶだけ): 航跡=見出し(名前・種別・所属)/中心点をこの航跡へ/選択を解除。地表=緯度経度の見出し/
    ここにレーダー観測点を追加/ここを原点に設定(`OriginPickState::on_pick`。シミュレーション停止中のみサーバーが受理)/ここを中心点にする(`RecenterRequestState::request_at`。
    カメラの中心点だけを移し、原点は変えない。高さはその地点の地表)/ここに図形を作成 ▶(図形の種類。`DrawToolState::start_at`でその地点を1点目にして開始)/緯度経度をコピー。
  - **作図ウインドウの図形一覧**(`ui/drawing_editor.rs`): 行の右クリックで、名前を変更(選択して名前欄へフォーカス)/複製(`DrawToolState::duplicate`。東北へ大きさの半分ずらして選択)/
    表示・非表示/削除。`ContextMenuState`が無ければ何も出ない。
  - 未実装: 矢印キーでの項目移動・ショートカット表示・チェック付き項目。
- 「ファイル」「表示」「ヘルプ」は現時点では項目未定のため、クリックすると
  「(準備中)」のプレースホルダのみ表示する(実装の骨組みだけ用意し、後から
  項目を追加できるようにしてある)
- 実装上の注意: メニュー項目のクリックハンドラで`WsConnection`(非`Copy`)を
  ムーブするクロージャ(`on_submit`)を、開閉のたびに何度も呼ばれる`Fn`/`FnMut`
  クロージャの外側で1回だけ作ると「2回目以降の呼び出しでムーブ済みエラー」に
  なる(Leptosの`{move || ...}`は再実行される前提のため`FnMut`である必要がある)。
  `origin_dialog.rs`では、開閉のたびに実行される内側のクロージャの中で
  `conn.clone()`してから`on_submit`を作ることで回避している(`coverage_altitude_dialog.rs`
  は`WsConnection`を持たずRwSignalのみで完結するため、この問題自体が発生しない)
- 覆域高度の変更をメインパネルの3D描画へ反映する経路: `coverage_altitude_dialog.rs`は
  `coverage_altitude_m`シグナルを更新するだけで、実際のジオメトリ再構築・再描画は
  `ui/terrain_view/mod.rs`のEffect(レーダー観測点の一覧・選択状態を購読していた
  ものに`coverage_altitude_m`も加えた)が担う。ダイアログ側とメインパネル側が
  別コンポーネントであっても、共有シグナル経由のリアクティブな購読だけで完結し、
  互いを直接呼び出す必要がない

### 7.8 シミュレーションステータスパネルの状態表示(AppStatus)

シミュレーションステータスパネルの状態表示は、バッジなどの装飾を付けず、C++側から配信された
`AppStatus.text`を**そのまま文字列として表示する**だけ。取得できない場合(WebSocketが
`ConnectionStatus::Connected`でない、または接続済みでもまだ`AppStatus`を受け取っていない)は、
詳細を出し分けず一律「接続中」とだけ表示する(`ConnectionStatus`ごとの色分け・文言の出し分けはしない)。

左パネルのシミュレーションステータスパネルには、状態文字列の下に**「開始」「一時停止」ボタン**(それぞれ`resume`/`pause`
コマンドを送る)と、原点・フレームの下に**「航跡数」**(最新の`TrackList`のトラック数。6.12節)がある。以前は`resume`を送る部品が
フロントに無く、`running_`が`false`のまま経過時間が0で止まっていた。実行中は原点を変更できない(3.4節。拒否は`CommandError`)。

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
| 観測点の追加〜覆域表示のデータフロー | 6.9 | フローチャート |
| タブ付きパネルのクラス図 | 7.6 | クラス図 |
| フローティングパネルの開閉状態 | 7.7 | ステートマシン図 |
