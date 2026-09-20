# 実装仕様 第1部: 地形データ・測地・標高サンプリング・メッシュ・前処理ツール

設計書体系([README](../README.md))の詳細設計(ライブラリ実装仕様)第1部で、[IMPLEMENTATION_GUIDE.md](../IMPLEMENTATION_GUIDE.md)の一部。
上位設計(方針・理由・図): [詳細設計書](../DETAILED_DESIGN.md) 1節(地形データの実態)・2節(前処理)・3節(座標系)・6.6〜6.7節。数値・手順はこの文書が正。

本書だけで、以下を1から実装できることを目標にする。

- 地形データのファイル形式(前処理ツールの出力 = フロントの入力。**バイト単位の契約**)
- 前処理ツール`geotiff_preprocess`(C++/GDAL)
- `terrain::{fetch, loader, geodesy, heightmap, mesh}`(Rust)

記号: 「タイル」=緯度経度1度×1度。「チャンク」=タイルを6×6に分けた1個(緯度経度とも1/6度)。
「レベル」=解像度段階。「ノード」=グリッドの格子点(セルの角)。

---

## 1. 地形データのファイル契約(サーバー ⇔ フロント)

### 1.1 ファイル一覧

ベースURL(例 `http://localhost:9001/terrain`。末尾スラッシュなし)の下に置く。

| パス | 内容 |
|---|---|
| `metadata.json` | 全体設定(1.2) |
| `tile_index.json` | 存在するタイルの一覧(1.3) |
| `base.bin` | レベル0(タイル全体で1枚のグリッド)を全タイル分連結(1.4) |
| `tiles/L{k}/N035E138.bin` | レベルk(≥1)のタイル別ファイル。チャンクのレコードを連結(1.5) |

タイル名は `{N|S}{|lat|:3桁ゼロ埋め}{E|W}{|lon|:3桁ゼロ埋め}`(`(35,138)`→`N035E138`、`(-1,-5)`→`S001W005`、`(0,0)`→`N000E000`)。
`lat`/`lon`はタイル**南西角**の整数度。

### 1.2 `metadata.json`

```json
{
  "tile_levels": [60, 180, 600, 1800, 3600],
  "chunks_per_tile": 6,
  "elevation_min": -330.0,
  "elevation_max": 3937.0,
  "geodetic_bounds": { "min_lat": 20.0, "max_lat": 50.0, "min_lon": 120.0, "max_lon": 150.0 },
  "source_crs": "EPSG:4326 (WGS84相当, GRS80楕円体)",
  "height_datum": "orthometric height (EGM96 geoid)",
  "ellipsoid": { "a_m": 6378137.0, "inv_f": 298.257222101 },
  "has_texture": false,
  "default_origin": { "lat_dms": "35°21'20\"N", "lon_dms": "138°51'35\"E", "lat_deg": 35.355556, "lon_deg": 138.859722 }
}
```

フロントが読む必須フィールド(それ以外は無視してよい。`serde`で未知フィールドは無視):
`tile_levels: [u32]`・`chunks_per_tile: u32`・`elevation_min/max: f32`・`geodetic_bounds{min_lat,max_lat,min_lon,max_lon: f64}`・
`ellipsoid{a_m,inv_f: f64}`・`has_texture: bool`・`default_origin{lat_deg,lon_deg: f64}`。

意味:
- `tile_levels[k]` = レベルkの「1度タイル1辺のセル数」N_k(先頭がレベル0=最粗)。実データは`[60,180,600,1800,3600]`
  (約1.85km/620m/185m/62m/31mのセル。3600は元データ1画素=1秒角)。
- **検証(フロント`load_terrain`)**: `tile_levels.len() >= 2`、`chunks_per_tile != 0`、`tile_levels[1..]`がすべて`chunks_per_tile`で割り切れる。違反は`Err`(取得失敗として`TerrainStore.error`に入る)。
- `elevation_min/max`: 前処理時に**ダウンサンプリング前の全画素**から実測(NaN除外)。色の正規化では**上限だけ**を使う(下限は0m固定。6.2)。
- `geodetic_bounds`: 存在するタイル全体の外接矩形(整数度。max側はタイル南西角+1)。範囲チェック・タイル索引の大きさに使う。
- 数値は`std::setprecision(15)`で出力する(既定6桁だと`138.859722`が`138.86`に丸まる不具合が過去にあった)。

### 1.3 `tile_index.json`

```json
{"tiles": [{"lat": 35, "lon": 138, "elevation_min": 0, "elevation_max": 3776}, ...]}
```

**陸のあるタイルだけ**を、緯度昇順→経度昇順で並べる(この順が`base.bin`の並びと一致)。海のみのタイルは含めない(フロントでは標高0mの海と同じ扱い=メッシュ無し+水域)。
`elevation_min/max`はそのタイルの実測(LODの距離計算で中間高さ`(min+max)/2`に使う)。

### 1.4 グリッドの共通形式と`base.bin`

- **グリッド** = `(N+1)×(N+1)`ノードの `int16` 標高(メートル、四捨五入)。リトルエンディアン。row-major。
  **行は南→北(j=0が南端)、列は西→東(i=0が西端)**。データなし(海・欠損)は `-32768`(`i16::MIN`。`NO_DATA`)。
  *(GDALのラスタは北→南なので前処理で行順を反転する。これを忘れると地形が南北反転する。過去に実際に発生)*
- ノード(列i, 行j)の位置: タイル内では 経度 = `lon_tile + i/N`、緯度 = `lat_tile + j/N`。
- **`base.bin`** = 全タイルのレベル0グリッド(N₀=`tile_levels[0]`、ノード数(N₀+1)²)を`tile_index.json`の順に連結。
  総バイト数 = `タイル数 × (N₀+1)² × 2`。フロントはこれと**一致しなければエラー**にする。起動時に全部取得して常時保持する(約2.9MB)。

### 1.5 `tiles/L{k}/{名前}.bin`(k≥1)

1タイルを`C×C`のチャンク(C=`chunks_per_tile`、6)に分け、チャンク番号 `c = cy*C + cx`(cy=行(南→北), cx=列(西→東))順に、
**固定サイズのレコード**を連結する。レコード = チャンクのグリッド `(n+1)×(n+1)`ノード(`n = N_k / C`)。

```
record_bytes = (n+1)² × 2
チャンクcのバイト範囲 = [c*record_bytes, (c+1)*record_bytes - 1]      // HTTP Range: bytes=a-b
ファイル全体 = C² × record_bytes
```

チャンク(cx, cy)のグリッド内ノード(i, j)は、タイル全体でのノード(cx*n + i, cy*n + j)に等しい(**隣のチャンクとは縁のノードを共有**する。
同じレベル同士なら縁が完全に一致)。

### 1.6 サーバーへの要件(フロントが前提とすること)

- 静的配信。`Content-Type`: JSONは`application/json`、binは`application/octet-stream`。
- **HTTP Range**(単一範囲`bytes=a-b`)に`206`で対応するのが望ましい。未対応(`200`で全体)でも動くが、毎回全体を転送する。
  Rangeは単純な`bytes=a-b`ならCORSのプリフライトが要らない。別オリジンなら`Access-Control-Allow-Origin`を付ける。
- フロントは失敗(HTTPエラー・サイズ不一致)を**再試行しない**([第5部](5_ui_and_integration.md) 4.3の`failed`集合)。

---

## 2. HTTP取得 `terrain::fetch`(フロント)

`gloo_net::http::Request`を使う。関数(すべてasync・`Result<_, String>`):

| 関数 | 動作 |
|---|---|
| `fetch_metadata(base_url)` | `{base}/metadata.json`をJSONとして`TerrainMetadata`へ |
| `load_terrain(base_url)` | metadata取得→**1.2の検証**→`tile_index.json`取得→`base.bin`取得→サイズ検証→`TerrainData::new(metadata, base_url, index, decode_i16_le(bytes))` |
| `fetch_tile_level(base_url, key, level, chunk_cells, chunk_count)` | `tiles/L{level}/{名前}.bin`全体を取得。期待長 = `chunk_count*(chunk_cells+1)²*2`。不一致は`Err`。`Vec<i16>`を返す |
| `fetch_chunk_grid(base_url, key, level, chunk, chunk_cells)` | 同ファイルからRangeでチャンク1個分(`start = chunk*record_bytes`, `end = start+record_bytes-1`)。長さ不一致は`Err` |
| `tile_name(key)` | 1.1の名前 |

- `decode_i16_le(bytes)` = `bytes.chunks_exact(2)`を`i16::from_le_bytes`へ(端数バイトは捨てる)。
- Range取得でサーバーが**200(全体)を返した場合**は、返ってきた全体から`[start..=end]`を切り出す。範囲外なら`Err`。
- `!response.ok()`は`Err("… HTTP {status}")`。

---

## 3. 取得済みデータの保持 `terrain::loader`

### 3.1 型

```rust
pub type TileKey = (i32, i32);            // 南西角 (緯度, 経度)
pub type MeshKey = (i32, i32, u8);        // (タイル緯度, タイル経度, チャンク番号) — GPUメッシュの識別子
pub const WHOLE_TILE: u8 = u8::MAX;       // MeshKeyのチャンク番号がこれ = タイル全体(レベル0)の1枚メッシュ
pub const NO_DATA: i16 = i16::MIN;
pub const WHOLE_FILE_MAX_LEVEL: usize = 2; // このレベル以下はタイル1ファイルをまとめて取得、超えるレベルはRangeでチャンク単位
```

`TerrainMetadata`/`GeodeticBounds`/`DefaultOrigin`は1.2のserde構造体。`Ellipsoid{a_m, inv_f}`はgeodesyにある。

### 3.2 `TerrainData`

```
TerrainData { metadata, base_url, base: Vec<i16>, tiles: Vec<TileEntry>, slots: Vec<Option<usize>>, cols, stamp: Cell<u64>, cached_bytes: Cell<usize> }
TileEntry { key, elevation_min, elevation_max, base_offset, chunk_level: Vec<Cell<u8>>(チャンクごと), detail: RefCell<Vec<Option<CachedGrid>>> }
CachedGrid { data: Rc<Vec<i16>>, last_used: Cell<u64> }
```

- `rows = round(max_lat-min_lat)`, `cols = round(max_lon-min_lon)`。`slots`は`rows*cols`個で、`(lat-min_lat)*cols + (lon-min_lon)` → `tiles`の添字。範囲外のタイルは索引に入れない。
- `tile(key)`: `row=key.0-min_lat, col=key.1-min_lon`が範囲外ならNone、`slots`引き。
- `base_offset = tile_index上の順番i × (N₀+1)²`。`whole_grid(tile)` = `base[base_offset .. +(N₀+1)²]`。
- `detail`の添字(`slot(level, chunk)`) = `(level-1)*chunk_count + chunk`(`level>=1 && chunk<chunk_count`のときだけ有効。レベル0や範囲外は`None`)。
  要素数 = `(num_levels-1) × chunk_count`。`chunk_level`は`chunk_count`個で**初期値0**(=タイル全体をレベル0で出している)。
- 補助: `num_levels()`, `chunks_per_tile()`, `chunk_count() = C²`, `level_cells(k) = tile_levels[k]`, `chunk_cells(k) = level_cells(k)/C`。
- `chunk_grid(tile, level, chunk) -> Option<Rc<Vec<i16>>>`: 取得済みなら返し、`stamp += 1; last_used = stamp`(LRU用)。
- `has_chunk_grid`: 取得済みか(stampは進めない)。
- `best_cached_level(tile, chunk, max_level)`: `1..=min(max_level, num_levels-1)`を**大きい方から**見て、取得済みの最初のレベル。無ければ0。
- `insert_chunk_grid(key, level, chunk, data)`: タイルが無い/`level>=num_levels`/`slot`が無効/`data.len() != (chunk_cells(level)+1)²`なら**黙って無視**。
  登録時、そのスロットが空だったときだけ`cached_bytes += len*2`。既存を置き換える場合は加算しない。
- `insert_tile_level(key, level, all)`: `record=(chunk_cells+1)²`ずつ切って各チャンクを`insert_chunk_grid`。
- `set_chunk_level(key, chunk, level)` / `set_whole_tile(key)`(全チャンクを0に): **「いま画面に出しているレベル」**を記録する。標高サンプリングはこれを見る(3.3)。
- `evict_unused(keep(tile_key, chunk, level)->bool, limit_bytes)`: `cached_bytes <= limit`なら何もしない。
  そうでなければ`keep`がfalseの取得済みグリッドを`last_used`昇順(古い順)に、`cached_bytes <= limit`になるまで破棄(`cached_bytes -= len*2`)。`keep`がtrueのものは上限を超えていても残す。

### 3.3 `sample_bilinear(tile, u, v) -> f32`(タイル内位置 u=経度方向0..1, v=緯度方向0..1)

1. `k=C`。`cx = clamp(floor(u*k), 0, k-1)`, `cy = clamp(floor(v*k), 0, k-1)`。`level = tile.chunk_level[cy*k + cx]`。
2. `level > 0` かつ そのチャンクが取得済みなら、そのグリッドで引く: `cells = chunk_cells(level)`, `N = level_cells(level)`,
   `fx = u*N - cx*cells`, `fy = v*N - cy*cells`。
   取得済みでなければ(または`level==0`)、レベル0の`whole_grid`で引く: `cells = N₀`, `fx = u*N₀`, `fy = v*N₀`。
3. 双線形: `i0 = clamp(floor(fx),0,cells-1)`, `j0 = clamp(floor(fy),0,cells-1)`, `tx = clamp(fx-i0,0,1)`, `ty = clamp(fy-j0,0,1)`。
   4ノード`v00=(j0,i0) v10=(j0,i0+1) v01=(j0+1,i0) v11=(j0+1,i0+1)`。**どれか1つでも`NO_DATA`なら0.0を返す**(海=標高0m)。
   `h0 = v00+(v10-v00)*tx; h1 = v01+(v11-v01)*tx; return h0+(h1-h0)*ty`。

### 3.4 テスト用の合成地形(`cfg(test)`)

`TerrainData::synthetic(min_lat, min_lon, rows, cols, height:(lat,lon)->i16)`: `rows×cols`枚のタイルを並べ、レベル0だけ`height`で埋める。
既定レベル`[6,12,24]`・チャンク2×2、`elevation_min=-100, elevation_max=4000`、`ellipsoid=WGS84`、`default_origin=(min_lat+0.5, min_lon+0.5)`、`base_url="http://test"`。
`synthetic_with_levels(…, tile_levels, chunks_per_tile, height)`はレベル定義を指定(LODのテストは実データと同じ`[60,180,600,1800,3600]`・6分割を使う)。
細かいレベルは空で、必要なら`insert_chunk_grid`で足す。**この合成データ生成を最初に作ると以降の全テストが書ける。**

---

## 4. 測地 `terrain::geodesy`

### 4.1 楕円体と原点

```rust
pub struct Ellipsoid { pub a_m: f64, pub inv_f: f64 }          // serde::Deserialize
impl Ellipsoid { pub const WGS84: Ellipsoid = Ellipsoid { a_m: 6_378_137.0, inv_f: 298.257_222_101 }; }
pub struct Origin { pub lat_deg: f64, pub lon_deg: f64 }        // terrain::origin(Copy, PartialEq)
```

### 4.2 式

```
f  = 1/inv_f            e2 = f(2-f)         b = a*sqrt(1-e2)
N(φ) = a / sqrt(1 - e2 sin²φ)
geodetic→ECEF: X=(N+h)cosφ cosλ, Y=(N+h)cosφ sinλ, Z=(N(1-e2)+h) sinφ         (φ,λ ラジアン)
原点(φ0,λ0,h=0)のECEFを P0 とし、d = P - P0 に対し
  East  = -sinλ0·dX + cosλ0·dY
  North = -sinφ0 cosλ0·dX - sinφ0 sinλ0·dY + cosφ0·dZ
  Up    =  cosφ0 cosλ0·dX + cosφ0 sinλ0·dY + sinφ0·dZ
```

`h`はDSMの標高(EGM96海抜)をそのまま楕円体高として使う(ジオイド高との差は無視する簡略化)。遠方ほどUpが下がる(地球の丸み)のは式に自然に含まれる。

### 4.3 `EnuTransform`(すべて原点ごとに1回だけ前計算)

`new(&Origin, &Ellipsoid)`で保持するもの: `a`, `e2`, `origin_ecef`, `enu_from_ecef: DMat3`(行が東・北・上の単位ベクトル。glamは列優先なので
`from_cols((-sinλ0, -sinφ0cosλ0, cosφ0cosλ0), (cosλ0, -sinφ0sinλ0, cosφ0sinλ0), (0, cosφ0, sinφ0))`)、
`meridian_radius = a(1-e2)/w³`、`parallel_radius = a/w·cosφ0`(`w = sqrt(1-e2 sin²φ0)`)。

| メソッド | 仕様 |
|---|---|
| `transform(lat,lon,h) -> [f32;3]` | `transform_f64`をf32へ |
| `transform_f64(lat,lon,h) -> [f64;3]` | ECEF→`enu_from_ecef*(P-origin_ecef)` |
| `ecef_to_enu(x,y,z) -> [f64;3]` | 同上のECEF入力版(メッシュ生成が行ごとの三角関数を前計算して使う) |
| `ellipsoid_params() -> (a, e2)` | |
| `enu_to_geodetic(e,n,u) -> (lat_deg, lon_deg, h)` | `ecef = origin_ecef + enu_from_ecef^T*(e,n,u)`。`p=hypot(x,y)`, `lon=atan2(y,x)`, 初期`lat=atan2(z, p(1-e2))`。**6回反復**: `N=a/sqrt(1-e2 sin²lat)`, `h=p/cos(lat)-N`, `lat=atan2(z, p(1-e2·N/(N+h)))` |
| `inverse(e,n) -> (lat_deg, lon_deg)` | **接平面近似**: `lat=φ0+n/meridian_radius`, `lon=λ0+e/parallel_radius`。原点近傍(数十km)専用。遠方に使ってはならない(見通し計算・断面図が「原点=観測点」から短距離で使う) |
| `ellipsoid_shader_params() -> ([[f32;4];3], [f32;4])` | 水域シェーダー用の係数(4.4) |

### 4.4 水域シェーダーの楕円体係数

ENU点`p`について陰関数 `f(p) = c0 + 2 g·(M p) + |M p|²`(f=0が楕円体面、f<0が内側)。
`D = diag(1/a, 1/a, 1/b)`、`R = enu_from_ecef^T`(ENU→ECEF回転)。

```
M の行 = [R.row(0)*(1/a), R.row(1)*(1/a), R.row(2)*(1/b)]     // 各行はvec4の先頭3要素、w=0
g = (origin_ecef.x/a, origin_ecef.y/a, origin_ecef.z/b) を**f32に丸める**
c0 = Σ (f32丸めしたg_i)² を f64で足して - 1     // f32丸めのgに対して|g|²-1を求める(丸めで原点が面からずれない)
返り値 = (M: [[f32;4];3], [g.x, g.y, g.z, c0 as f32])
```

原点は楕円体上(h=0)なので定数項がほぼ0で、原点から遠くても桁落ちしない(ECEF絶対座標約6.4e6mのままf32で二乗すると数mずれる)。

### 4.5 検証(必須テスト)

- 往復: `(24.34,124.16,0)`など原点(35.355556,138.859722)から数千km離れた4点で`transform_f64`→`enu_to_geodetic`が、緯度経度1e-9度・高さ1e-3m以内で戻る。
- 遠方沈下: 石垣島(24.34,124.16,0)は原点から1.8〜2.0e6mで、`-350000 < Up < -250000`。
- 軸: 原点で(0,0,h)。経度+0.001度(緯度35)で東≒80〜100m・北≈0、緯度+0.001度で北≒100〜120m。
- 水域係数: 原点でf≈0(|f|<1e-6)、原点の真上+1000mでf>0、真下-1000mでf<0、遠方の標高0m点でも|f|<1e-6。
- `inverse`は原点(35,135)から(東10km,北-20km)で、厳密逆と緯度1e-4度・経度5e-4度以内で一致(上座標は`-(e²+n²)/(2a)`で近似して`enu_to_geodetic`へ)。

---

## 5. 標高サンプリング `terrain::heightmap`

```rust
pub fn sample_heightmap(data, lat, lon) -> Option<f32>
```
1. `geodetic_bounds`の外(`lat<min_lat || lat>max_lat || lon<min_lon || lon>max_lon`)なら`None`。
2. `lat0 = min(floor(lat), max_lat-1)`, `lon0 = min(floor(lon), max_lon-1)`(**東端・北端ちょうどは内側のタイルの端**として扱う)。
3. `data.tile((lat0,lon0))`が無ければ`Some(0.0)`(海=存在しないタイル)。
4. `u = clamp(lon-lon0, 0,1)`, `v = clamp(lat-lat0, 0,1)` → `Some(data.sample_bilinear(tile, u, v))`。

```rust
pub fn ground_at_enu(data, transform, east, north) -> (lat, lon, up_f32)
```
「ENUの水平位置(東,北)を通る鉛直線と地表の交点」。**標高ではなく丸み込みのENU上座標**を返す。遠方の地表の高さ・クリック判定・注視点の高さはこれを使う
(`EnuTransform::inverse`は近似なので使わない)。
```
up = -(east² + north²) / (2a)                      // 丸みによる低下の第一近似
repeat 4 times:
    (lat, lon, _h) = enu_to_geodetic(east, north, up)
    elevation = sample_heightmap(lat, lon) or 0.0   // 範囲外・海は0m
    up = transform_f64(lat, lon, elevation)[2]      // その地表点のENU上座標
return (lat, lon, up as f32)
```
```rust
pub fn ground_at_geodetic(data, transform, lat, lon) -> (east, north, up)   // f32
```
`elevation = sample_heightmap(..) or 0.0` → `transform(lat, lon, elevation)`。

検証: 平坦(標高0)で400km離れた点の`up`は`-d²/(2a)`と3%以内。標高1000mの丘なら`up`の差は1000mから10m以内。原点を変えても`ground_at_geodetic`→`ground_at_enu`で緯度経度が1e-5度以内で戻る。

---

## 6. メッシュ生成 `terrain::mesh`

### 6.1 頂点

```rust
#[repr(C)] #[derive(Pod, Zeroable)]
pub struct TerrainVertex { pub position: [f32;3], pub color: [f32;3], pub normal_xy: [i16;2] }   // 28バイト
impl TerrainVertex { pub const UNLIT_NORMAL: [i16;2] = [i16::MIN, i16::MIN]; pub fn unlit(position, color) -> Self }
pub struct TerrainMesh { pub vertices: Vec<TerrainVertex>, pub indices: Vec<u32> }
```
- `position`はメッシュ原点基準のENU座標(東,北,上)。**頂点データは軸を入れ替えない**(Z-upのままカメラ側で吸収)。
- `normal_xy`は単位法線のEast・North成分を snorm16(`round(clamp(v/len,-1,1)*32767)`)。Up成分はシェーダーが`sqrt(1-x²-y²)`で復元。
  `UNLIT_NORMAL`は「陰影を付けない」印(xy成分の長さが1を超える単位法線にありえない値。x=y=0は真上=平地で別物)。

### 6.2 定数と配色

- `COLOR_MIN_ELEVATION_M = 0.0`(0m以下は低地色。DSMには水面ノイズ由来の大きな負値があり、`elevation_min`を下限にすると全体の色がずれるため)。上限は`metadata.elevation_max`。
- `t = (max>min) ? clamp((elev-min)/(max-min),0,1) : 0`。5点カラーストップの区間線形補間:

| t | RGB |
|---|---|
| 0.0 | (0.12, 0.40, 0.18) |
| 0.25 | (0.15, 0.50, 0.20) |
| 0.5 | (0.55, 0.50, 0.25) |
| 0.75 | (0.45, 0.32, 0.22) |
| 1.0 | (0.95, 0.95, 0.95) |

- NO_DATA頂点の色は(0,0,0)・標高0mで配置(NaNだと位置が破綻するため)。この頂点を含む三角形は張らない(6.5)。
- スカート(縁の壁)の深さ `skirt_depth_m(level) = [800, 400, 250, 150, 100][min(level, 4)]` メートル。

### 6.3 頂点列の生成 `grid_vertices(grid, cells, place, max_elevation, transform)`

`place = { lat_start, lon_start, step_deg, skirt_depth }`: ノード(列i,行j)の緯度=`lat_start + j*step_deg`、経度=`lon_start + i*step_deg`。

- 頂点数 = `(cells+1)² + 4(cells+1)`(`tile_vertex_count(cells)`)。並び = ノード(行=南→北、列=西→東)全部 → スカート4辺(南,東,北,西)。
  **頂点数と並びは原点に依存しない**(原点変更時は位置だけを書き換える`update_mesh_vertices`)。
- 高速化: 行jごとに`(sinφ, cosφ, N(φ))`、列iごとに`(sinλ, cosλ)`を前計算し、`x=(N+h)cosφcosλ, y=(N+h)cosφ sinλ, z=(N(1-e2)+h) sinφ`→`transform.ecef_to_enu`。
- 各ノード: `value==NO_DATA`なら`(h=0, color=(0,0,0))`、それ以外は`(h=value, color=elevation_to_color(value, 0, max_elevation))`。
- 法線用に**f64のENU位置**も保持する(f32だと原点から遠いタイルで隣ノードとの差が誤差に埋もれてざらつく)。
- **スカート**: 辺e(0=南: ノードk=i, 行0 / 1=東: 列cells / 2=北: 行cells / 3=西: 列0)の各ノードについて、頂点をコピーし位置だけ
  `enu(j, i, min(h - skirt_depth, 0.0))`(**海抜0mより上で止めない**=少なくとも海抜0mまで届かせる。水域の深度が隠せるのは海抜0mより下を通る視線だけで、標高の高い縁の下に隙間が残ると地形の裏側が見えるため)。
  `h`はNO_DATAなら0。

### 6.4 法線 `node_normals(grid, n, positions)`

各ノード(i,j)について:
1. 自身がNO_DATAなら`UNLIT_NORMAL`。
2. 東西: `i_lo = (i>0 && (i-1,j)が陸) ? i-1 : i`, `i_hi = (i+1<n && (i+1,j)が陸) ? i+1 : i`。南北も同様(`j_lo`, `j_hi`)。
   `i_lo==i_hi` または `j_lo==j_hi`(差分が取れない)なら`UNLIT_NORMAL`。
3. `east = pos[j][i_hi] - pos[j][i_lo]`、`north = pos[j_hi][i] - pos[j_lo][i]`(中心差分。縁・海隣接は片側差分)。
4. `cross = east × north`(標準の外積)。`len = |cross|`。`len < 1e-9` または `cross.z <= 0` なら`UNLIT_NORMAL`。
5. `[pack(cross.x), pack(cross.y)]`、`pack(v) = round(clamp(v/len, -1, 1)*32767)`。

チャンクの縁ノードは隣チャンクの標高を見ない片側差分になり、隣と法線がわずかに食い違うが許容(目立たない)。

### 6.5 インデックス `grid_indices(grid, cells)`

`n=cells+1`, `land(k) = grid[k] != NO_DATA`。セル(i,j)ごとに`i0=j*n+i, i1=i0+1, i2=i0+n, i3=i2+1`:
- `land(i0)&&land(i1)&&land(i2)` なら三角形`[i0,i1,i2]`
- `land(i1)&&land(i3)&&land(i2)` なら三角形`[i1,i3,i2]`

**NO_DATAノードを1つでも含む三角形は張らない**(海岸線は最大1セル陸側へ退く)。
スカート: `skirt_base = n*n`。辺e・区間k(`0..cells`)で、`a=edge_node(e,k)`, `b=edge_node(e,k+1)`が**どちらも陸のときだけ**、`sa=skirt_base+e*n+k`, `sb=sa+1`として
`[a, b, sa,  b, sb, sa]`(2枚)。`edge_node(e,k,cells)`: e=0→`k`、1→`k*n+cells`、2→`cells*n+k`、3→`k*n`。
三角形の有無はグリッドだけで決まり原点に依存しない。全部陸なら三角形数 = `2cells² + 4cells·2`。

### 6.6 公開関数

| 関数 | 内容 |
|---|---|
| `build_whole_tile_vertices/mesh(data, tile, transform)` | レベル0。`cells=N₀`, `place={tile.lat, tile.lon, 1/N₀, skirt_depth_m(0)}`, グリッド=`whole_grid` |
| `build_chunk_vertices/mesh(data, tile, chunk, level, transform) -> Option<_>` | グリッド未取得ならNone。`(cx,cy)=(chunk%C, chunk/C)`, `cells=chunk_cells(level)`, `step=1/level_cells(level)`, `lat_start=tile.lat+cy*cells*step`, `lon_start=tile.lon+cx*cells*step`, `skirt_depth_m(level)` |
| `tile_vertex_count(cells)` | `(cells+1)²+4(cells+1)` |

### 6.7 検証(必須テスト)

- 色: `-50m`→最低色、`0m`(min=0,max=1000)→最低色、`5000m`→最高色、`max<=min`→常に最低色、`t=0.125`の緑成分=0.45。
- スカート: レベル0=800、レベル1<レベル0、レベル≥4は同じ値。
- 全陸のタイル(レベル0, N₀=6)の頂点数=`tile_vertex_count`、インデックス数=`3*(2*cells² + 8*cells)`、全インデックス<頂点数。原点をタイル中央にすると中央ノードは(≈0,≈0,標高)。
- 中央ノード1個をNO_DATAにすると三角形が6枚減り、そのノードを参照するインデックスが無い。南縁の中央ノードをNO_DATAにすると 地表3枚+スカート4枚(2区間×2枚)減る。
- 東へ上る斜面の中央ノードの法線はx<-100・|y|<5(西向き)。四隅は片側差分で法線が求まる。NO_DATAノードは`UNLIT_NORMAL`。

---

## 7. 前処理ツール `tools/geotiff_preprocess`(C++/GDAL)

単発実行CLI。**独立したCMakeプロジェクト**(サーバーはGDALをリンクしない)。C++20、`find_package(GDAL CONFIG REQUIRED)`、vcpkgで`gdal`(default-features無し)。
MSVCでは`/Zc:__cplusplus`と`_CRT_SECURE_NO_WARNINGS`。使い方: `geotiff_preprocess [map_dataディレクトリ=map_data] [出力ディレクトリ=sample/sim_server/assets/terrain]`(リポジトリルートから実行する前提)。

### 7.1 入力

`map_data/`内の通常ファイルのうち、名前に`_DSM.tif`を含むものだけを走査する。**`_DSM.tif`の直前8文字**(例 `ALPSMLC30_N035E138_DSM.tif` → `N035E138`)を`[N|S][3桁][E|W][3桁]`として解析してタイルIDにする(`_DSM.tif`の位置が8未満、半球文字が不正、数値変換失敗のファイルは「cannot parse tile id」と警告してスキップ)。GDALはまだ開かない。`*_MSK.tif`・`*_STK.tif`・`*_HDR.txt`などの同居ファイルは対象外(MSKだけ7.2で使う)。
外接矩形 = 全タイルIDの最小/最大(max側は南西角+1)。**固定値は持たない**(`map_data`を増減すると次回実行で自動追従)。
各タイル: 1度=**3600×3600画素**のFloat32(GDTを指定して読めば16bit整数でも自動変換)。サイズが違うタイルは警告してスキップ。ジオトランスフォームが無ければエラー。
出力順を安定させるため、タイルは(緯度,経度)昇順にソートする。

### 7.2 1タイルの読み込み(`load_dsm_tile`)

1. `RasterIO`で全画素をFloat32で読む(行0=北端、GDAL標準)。`GetNoDataValue`を取得。
2. **海マスク** `*_MSK.tif`(同名で`_DSM.tif`→`_MSK.tif`)を読み、**画素値==3を海**とするbitmapを作る(0=有効,3=海,4/12=代替DEM補完の陸)。
   DSM側の海は標高0mで格納され、NODATAでは検出できない。マスクが無い/サイズ不一致/開けないときは警告してNODATAのみで判定(空bitmap)。
3. **小さなNODATA穴の補間**(NODATAが有る場合): 候補 = `値==NODATA && !海`。8連結で穴の連結成分に分け、**2000画素超の成分は埋めない**(`kMaxFillableHolePixels`)。
   埋める成分は、穴の境界(有効画素に隣接する穴画素)から内側へBFSで波及させ、その時点で**確定済み(有効または埋め済み。海は材料にしない)の8近傍の平均**で順に埋める。
4. 残ったNODATA画素(大きすぎて埋められなかったもの)を`NaN`にする。
5. 海マスクの画素を`NaN`にする(海は「欠損」ではなく「実際に海」なので補間しない)。

### 7.3 レベルグリッド(`build_level_grid(src, cells)`)

`nodes=cells+1`, `cell_px = 3600/cells`, `half_px = max(cell_px/2, 1.0)`。ノード位置`center`(画素座標、0=タイル端=画素境界)を中心とする窓:
```
start = clamp(floor(center - half_px + 0.5), 0, 3599)
end   = clamp(floor(center + half_px + 0.5), start+1, 3600)          // [start, end)
列ノードi: window(i*cell_px)          行ノードj(南→北): window((cells-j)*cell_px)   // 北端基準の行範囲へ反転
```
値 = 窓内の**有効(非NaN)画素の平均を`lround`し[-32767, 32767]にクランプ**。有効画素が0個なら`-32768`(NO_DATA)。
出力は`(cells+1)²`のint16、行は南→北。

### 7.4 レベルと出力

- `kLevelCells = {60, 180, 600, 1800, 3600}`、`kChunksPerTile = 6`。起動時に「レベル1以上が`kChunksPerTile`で割り切れる」ことを検証(違反はエラー終了)。
- タイルごと: 全画素の有効値から`elevation_min/max`を求める。**有効画素が1つも無ければ「陸なし」としてスキップ**(索引・baseに入れない)。
- 各レベルのグリッドを作り、レベル0は結果として返す(メインスレッドで`base.bin`へ連結)。レベル≥1は`tiles/L{k}/{名前}.bin`へ即書き出し
  (チャンク番号`cy*6+cx`順に、`(cells/6+1)²`ノードの部分グリッドを切り出して連結。`write_chunked_level_file`)。
- `tiles/L1..L4`ディレクトリを先に作る。旧方式の`heightmap.bin`が残っていれば削除する。
- 並列: ワーカー数 = `min(8, max(1, hardware_concurrency/2))`。1スレッドあたりメモリは数百MB(標高52MB+マスク13MB+穴埋め)。全域モザイクは作らない。
- 出力: `base.bin`(陸タイルのlevel0連結)、`tile_index.json`、`metadata.json`(1.2。`elevation_min/max`は全陸タイルの最小最大)。陸タイルが0個ならエラー。
  JSONは手書き(ライブラリ不要)。`metadata.json`は`std::setprecision(15)`の後に出力し、`geodetic_bounds`は**整数のまま**(`20`、`20.0`ではない。serdeの`f64`は整数リテラルも読める)、`tile_levels`は`[60, 180, 600, 1800, 3600]`(カンマ+空白区切り)。`default_origin.lat_dms`/`lon_dms`は度記号を`°`エスケープで書く(`"35°21'20\"N"`)。`tile_index.json`はタイル1件を1行(`{"lat": 35, "lon": 138, "elevation_min": 0, "elevation_max": 3776}`)。標高は`float`をそのまま出力する。
- ファイルは`int16`リトルエンディアンで書く(x86/x64ホスト前提)。

### 7.5 検証

- 実データ: `metadata.json`の`geodetic_bounds`が`map_data`の外接矩形と一致、`base.bin`のサイズ=`タイル数×61²×2`(N₀=60)、
  `tiles/L4/N035E138.bin`のサイズ=`36×601²×2`(約26MB)。
- 縁の共有: 同じレベルの隣接チャンクで、東西・南北の境界ノード列が一致する。
- 南北: 富士山(35.36N,138.73E付近のタイル`N035E138`)の最高点が、グリッドの**北寄りでなく南→北の正しい位置**にある(反転バグの確認)。
