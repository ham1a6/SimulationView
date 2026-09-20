# 実装仕様 第2部: カメラ・LOD計画・ピッキング・断面・見通し(覆域)計算

設計書体系([README](../README.md))の詳細設計(ライブラリ実装仕様)第2部で、[IMPLEMENTATION_GUIDE.md](../IMPLEMENTATION_GUIDE.md)の一部。
上位設計(方針・理由・図): [詳細設計書](../DETAILED_DESIGN.md) 6.6節(カメラ)・6.9節(見通し・覆域)・6.10節(地形LOD)。数値・手順はこの文書が正。

GPUにもネットワークにも触れない**純粋関数**を集めた部。
すべてネイティブ(`cargo test`)で検証できる。前提: [第1部](1_data_and_geodesy.md)。

---

## 1. カメラ `terrain::camera`

### 1.1 型

```rust
pub enum Projection { Perspective { fov_y_radians: f32 }, Orthographic { view_height_m: f32 } }
pub struct Camera { pub eye: Vec3, pub target: Vec3, pub up: Vec3, pub projection: Projection, pub aspect: f32, pub z_near: f32, pub z_far: f32 }
pub enum CameraPreset { Overview }            // 初期アングルは1種類だけ
pub enum ViewMode { ThreeD, TwoD }
pub struct OrbitCamera { pub target: Vec3, pub distance: f32, pub yaw: f32, pub pitch: f32, pub fov_y_radians: f32, pub z_near: f32, pub z_far: f32, pub mode: ViewMode }
```
ライブラリは`glam 0.33`。`glam::camera::rh::{proj::directx, view::look_at_mat4}`を使う。**座標系はENU(X=東,Y=北,Z=上)のまま**で、Z-upはビュー行列の`up`引数で吸収する。

### 1.2 定数

| 定数 | 値 | 意味 |
|---|---|---|
| `MIN_DISTANCE` | 100.0 | ズームインの下限(m) |
| `MAX_DISTANCE` | 2,000,000.0 | ズームアウトの上限(m) |
| `Z_FAR` | 8,000,000.0 | 3Dの遠方クリップ(MAX_DISTANCE+データ対角線に余裕) |
| `MIN_PITCH` / `MAX_PITCH` | -1.5 / +1.5 rad | 真上・真下(±π/2)は`look_at`が特異点になるので避ける |
| `MIN_EYE_CLEARANCE_M` | 30.0 | 視点が真下の地面から離れる最小高さ(最細セル≒30m) |
| `ORTHO_EYE_HEIGHT_M` | 100,000.0 | 2Dの視点高度(見た目に影響しない。クリップ範囲用) |
| `ORTHO_DEPTH_RANGE_M` | 1,200,000.0 | 2Dの奥行き範囲(丸みで原点から3,300kmの端が約850km下がるため) |
| Overview初期値 | distance=400,000, yaw=-π/4, pitch=0.6, fov_y=50°, z_near=1.0, z_far=`Z_FAR`, mode=ThreeD, target=(0,0,target_up) | |

### 1.3 `OrbitCamera`

- `eye()` = `target + (d·cos(pitch)·cos(yaw), d·cos(pitch)·sin(yaw), d·sin(pitch))`。yawはEast軸基準・反時計回り。
- `orbit(dyaw, dpitch)`: `yaw -= dyaw`, `pitch = clamp(pitch + dpitch, MIN_PITCH, MAX_PITCH)`(3Dのみ)。
- `pan(d_east, d_north)`(2D): `target.x -= d_east; target.y -= d_north`(地図を掴んで動かす操作感)。
- `zoom(factor)`: `distance = clamp(distance*factor, MIN_DISTANCE, MAX_DISTANCE)`。2Dでは`distance`を**正射影の画面縦幅(m)**として流用する。
- `pan_orbit_target(dx_px, dy_px, canvas_h_px)`(3DのShift+ドラッグ): `wpp = 2·distance·tan(fov/2)/max(canvas_h,1)`、`right=(-sin yaw, cos yaw, 0)`、
  `forward_h=(-cos yaw, -sin yaw, 0)`、`delta = right·(dx·wpp) + forward_h·(-dy·wpp)`、`target.x -= delta.x; target.y -= delta.y`。
  **`target.z`は呼び出し側が移動先の地表(`ground_at_enu`の上座標)に更新する**(camera.rsは標高を知らない)。原点(`OriginState`)・メッシュには触れない。
- `keep_above_ground(ground_up: Fn(east,north)->f32)`(3Dのみ。2Dは何もしない):
  ```
  repeat 6 times:
      eye = eye();  min_z = ground_up(eye.x, eye.y) + 30
      if eye.z >= min_z: return
      sin_pitch = (min_z - target.z) / distance
      if sin_pitch >= sin(MAX_PITCH): break           // 仰角を上げても届かない
      pitch = clamp(max(asin(sin_pitch), pitch), MIN_PITCH, MAX_PITCH)
  eye = eye();  min_z = ground_up(eye.x, eye.y) + 30
  if eye.z < min_z:                                    // 視点を真上へ持ち上げ、距離と仰角を求め直す(水平位置は変えない)
      offset = (eye.x - target.x, eye.y - target.y, min_z - target.z)
      distance = clamp(|offset|, MIN_DISTANCE, MAX_DISTANCE);  pitch = clamp(asin(offset.z/|offset|), MIN_PITCH, MAX_PITCH)
  ```
  仰角は水平より下(注視点を見上げる向き)にもなりうる(谷底から山頂を見上げるのは地面の上なので許す)。呼び出しは**描画の直前に必ず**通す(カメラ操作・原点変更・LOD切替のあと)。
- `to_camera(aspect) -> Camera`:
  - 3D: `eye=eye()`, `target`, `up=Vec3::Z`, `Perspective{fov_y}`, `z_near`, `z_far`。
  - 2D: `eye = target + (0,0,ORTHO_EYE_HEIGHT_M)`, `up=Vec3::Y`(北が画面上), `Orthographic{view_height_m = distance}`, `z_near=1.0`, `z_far = ORTHO_EYE_HEIGHT_M + ORTHO_DEPTH_RANGE_M`。

### 1.4 行列(**反転Z**)

```rust
view_proj = projection_matrix() * look_at_mat4(eye, target, up)
projection_matrix():
    Perspective  => directx::perspective(fov_y, aspect, z_far, z_near)          // near/farを入れ替えて渡す = 反転Z
    Orthographic => half_h = view_height_m/2; half_w = half_h*aspect;
                    directx::orthographic(-half_w, half_w, -half_h, half_h, z_far, z_near)
```
- 深度は範囲`[0,1]`(DirectX/WebGPU互換)。**近い=1、遠い=0**。レンダラーは`depth_compare: Greater`・深度クリア値0.0・`Depth32Float`。
- **有限のfar**を使うこと。無限遠射影(`perspective_infinite_reverse`)は「原点から遠いほど地表が消える」不具合が出たので使わない。
- なぜ反転Zか: near=1m・far=8,000,000mの広いレンジを通常の深度で扱うと遠方の精度が失われz-fightingが出る。浮動小数は0付近が密。

### 1.5 レイ

```
basis(): forward = normalize(target - eye);  right = normalize(forward × up);  up' = right × forward
screen_to_ray(x, y, w, h) -> (origin, dir):       // x,yは canvas内CSS px、左上原点
    ndc_x = x/w*2 - 1;   ndc_y = 1 - y/h*2
    Perspective:  half_h = tan(fov/2); half_w = half_h*aspect;   (eye,  forward + right*(ndc_x*half_w) + up'*(ndc_y*half_h))
    Orthographic: half_h = view_height/2; half_w = half_h*aspect; (eye + right*(ndc_x*half_w) + up'*(ndc_y*half_h),  forward)
water_ray_basis() -> [[f32;4];4]:                  // 水域シェーダーの視線用
    (right,up) を 画面端までの長さ(half_w, half_h)倍にして
    [ [eye.xyz, persp(1.0=透視/0.0=正射影)], [forward,0], [right*half_w,0], [up'*half_h,0] ]
    // 透視のhalf_h = tan(fov/2)、正射影のhalf_h = view_height/2(screen_to_rayと同じ式)
```
**逆VP行列でレイを求めない**(near点と中間点の差が約1mしかなく、カメラが数百〜2,000km離れるとf32丸め誤差で向きが大きくずれ、ズームアウト時に別の地点を拾う)。
`projection_matrix()`(ビュー行列なし)は、カメラ固定の作図(視点空間)がカメラから見た座標をそのまま射影するために公開する。

### 1.6 検証(必須テスト)

- 反転Z(透視): 視線上`z_near`の点の深度≈1(1e-3)、`z_far`≈0、`[10,100,1e4,1e5,1e6,4e6]`mで**単調減少**(f32で潰れない)。正射影は同様+`(near+far)/2`で0.5(1e-3)。
- `screen_to_ray`のレイ上の点を`view_proj`で射影し直すと元のNDCに戻る(距離30,000/400,000/2,000,000mの3D、100,000mの2D、画素(0,0),(350,350),(700,700),(123,456),(600,50))。
- `water_ray_basis`が`screen_to_ray`と同じ視線を与える。`orbit/zoom/pan`のクランプと移動方向。`pan_orbit_target`が画面軸に沿う。
- `keep_above_ground`: 高い地形の上に視点があれば仰角を上げ、届かなければ持ち上げる。2Dは変更なし。2Dカメラは真下向き・北が上。

---

## 2. LOD計画 `terrain::lod`

「どのタイルのどのチャンクをどのレベルで出すか」を決める純粋関数。適用(取得・メッシュ差し替え)は[第5部](5_ui_and_integration.md)の`lod_driver`。

### 2.1 定数

| 定数 | 値 |
|---|---|
| `DETAIL_VERTEX_BUDGET` | 25,000,000(全タイルの下限=レベル1の分を含むチャンク頂点の合計上限) |
| `CHUNK_TARGET_CELL_PX` | 1.0(画面上で1セルがこのpx以下になる最も粗いレベルを理想とする) |
| `METERS_PER_DEGREE` | 111,000.0(セル寸法の見積もり) |
| `FRUSTUM_MARGIN` | 1.2(視錐台の左右上下判定の余裕) |

```rust
pub enum TileLayout { Whole, Chunks(Vec<u8>) }     // Chunksの中身はチャンク(行*C+列)ごとのレベル(≥1)
cell_size_m(data, level) = 111000 / level_cells(level)
chunk_vertex_cost(data, level) = tile_vertex_count(chunk_cells(level))     // = (n+1)²+4(n+1)
```
実データの頂点コスト: レベル1(n=30) 1,085 / 全タイルのレベル1下限=390タイル×36チャンク×1085≈1,520万 / レベル4(n=600) ≈36万。

### 2.2 ビュー情報(`ViewInfo`)

- `canvas_h = max(canvas_height_px, 1)`。
- 透視: `focus = camera.eye`、`tan_half = tan(fov/2)`。正射影(2D): `focus = camera.target`(視点は真上にあるだけで見ている場所ではない)、`ortho_pixel_m = view_height_m/canvas_h`。
- `focus_lat/lon = transform.enu_to_geodetic(focus)`(丸め込み)。
- `pixel_m(distance)`: 透視 = `2·max(distance, 50)·tan_half/canvas_h`、正射影 = `ortho_pixel_m`。
- `rect(lat0, lon0, lat1, lon1, mid_h) -> (distance, visible)`:
  - `nearest = transform.transform(clamp(focus_lat, lat0, lat1), clamp(focus_lon, lon0, lon1), mid_h)`(矩形内で基準に最も近い点)。
  - `distance`: 透視=`|nearest - eye|`、正射影=`hypot(nearest.x - focus.x, nearest.y - focus.y)`。
  - `visible`: 4隅+中心の5点を`view_proj`でクリップ座標へ。`clip.w > 0`の点では`m = w*1.2`として面ごとに「`x<-m`」「`x>m`」「`y<-m`」「`y>m`」を、**5点すべてが同じ面の外側**のときその面は「外」。
    `w<=0`(背後)の点が1つでもあれば全面の`outside`を`false`にリセット(他の点と組んで面をまたぐ場合に備える)。
    `visible = !(全点が背後) && !(いずれかの面が外)`。

### 2.3 アルゴリズム `plan_levels(data, transform, camera, canvas_h_px, resident) -> Vec<(TileKey, TileLayout)>`

`resident`は現在GPUにある状態(`HashMap<TileKey, TileLayout>`。無ければWhole扱い)。戻り値は**優先度順**(見えていて近いタイルが先頭)。

**(1) タイルごとの評価 `evaluate_tile`**
```
mid_h = (tile.elevation_min + tile.elevation_max)/2
(distance, visible) = view.rect(lat0, lon0, lat0+1, lon0+1, mid_h)                 // タイル全体
have(c) = resident[tile]がChunks(v)なら v[c]、それ以外は0
ideal_level(pixel_m, from=1) = from..=max_level のうち cell_size_m(k) <= CHUNK_TARGET_CELL_PX*pixel_m を満たす最小のk。無ければmax_level
tile_ideal = ideal_level(pixel_m(distance), 1)
if !visible || tile_ideal == 1:      // 一番近い点でも下限で足りる(遠い多数のタイルの計算を省く)
    全チャンクが (distance, visible) を共有し、level = target_level(have(c), 1, visible, max_level)
else:                                  // チャンクごとに rect() で距離・視野を求め、理想レベルから目標を決める
    step = 1/C;  chunkの矩形 = (lat0+cy*step, lon0+cx*step, +step, +step)
    (cdist, cvisible) = rect(...)
    level = target_level(have(c), ideal_level(pixel_m(cdist), 1), cvisible, max_level)
target_level(have, ideal, visible, max_level):
    t = !visible ? max(have, 1)                   // 視野外は現状維持(下限1)
      : have > ideal ? min(ideal+1, have)         // 理想より1つだけ細かいなら保つ(ヒステリシス)、それ以上細かければ「理想+1」まで下げる
      : ideal
    return clamp(t, 1, max_level)
```
**(2) 並べ替え**: `visible`降順→`distance`昇順。

**(3) 予算配分 `allocate_levels(data, infos, budget)`**
```
remaining = budget;  base_cost = chunk_count * chunk_vertex_cost(1)
levels = {}
for info in infos(優先度順): if remaining >= base_cost { remaining -= base_cost; levels[info.key] = [1; chunk_count] }   // 下限を確保できたタイルだけチャンク化
upgrades = 下限を確保したタイルの全チャンクを (visible, distance, key, chunk, target) で列挙し、visible降順→distance昇順に並べる
for up in upgrades:
    level = 1
    while level < up.target:
        delta = cost(level+1) - cost(level);  if delta > remaining: break
        remaining -= delta;  level += 1
    levels[up.key][up.chunk] = level
```
戻り値: 下限を確保できたタイルは`Chunks(levels)`、できなかったタイルは`Whole`。
予算が足りない遠い側は目標より粗いままになる(近い側が優先)。

### 2.4 検証(必須テスト。実データと同じレベル定義の3×3タイル、標高0m、原点は中央タイル中心)

- セルサイズ・頂点コストが上の数値と一致(`111000/60=1850m`, `/3600≈30.8m`、レベル1コスト1085)。
- `ideal_level`: ピクセル寸法から最粗の適合レベルを選ぶ(境界含む)。最細でも足りなければ最細。
- 遠景(2,000km)では全タイルが下限レベル1。近景では近いチャンクだけが細かく、先頭が最も近いタイル。
- 予算: 全タイルの下限が入らない予算だと遠い側のタイルが`Whole`。**`levels`の頂点コスト合計は常に予算以下**。
- ヒステリシス: 現状が「理想+1」なら維持、それより細かければ「理想+1」へ下げる。視野外のタイルは現状維持。

---

## 3. ピッキング `terrain::pick`

`pick_lat_lon(data, mesh_origin, camera, screen_x, screen_y, canvas_w, canvas_h) -> Option<(lat, lon)>`。GPU読み戻しなし。CPUの標高グリッドに対するレイマーチ。

```
定数: MAX_MARCH_DISTANCE = 8,000,000 m,  NUM_MARCH_STEPS = 6000 (刻み≒1.33km),  NUM_BISECT_STEPS = 24
(ray_origin, ray_dir) = camera.screen_to_ray(...);  |ray_dir|² < 1e-12 なら None;  dir = normalize(ray_dir)
transform = EnuTransform::new(mesh_origin, ellipsoid)
diff(t) = p.z - ground_at_enu(data, transform, p.x, p.y).up      where p = ray_origin + dir*t
if diff(0) < 0: None                                       // カメラが地表の下(異常系)
prev_t = 0
for i in 1..=6000:
    t = MAX_MARCH_DISTANCE * i / 6000
    if (ray_origin + dir*t).z > data.metadata.elevation_max: prev_t = t; continue     // 最高標高より上は地表評価を省く
    if diff(t) <= 0:
        lo = prev_t; hi = t;  24回二分探索( diff(mid) > 0 なら lo=mid、でなければ hi=mid )
        hit = ray_origin + dir*hi
        (lat, lon, _) = ground_at_enu(data, transform, hit.x, hit.y)              // 接平面近似は使わない(数百km離れると数十kmずれる)
        範囲外(geodetic_bounds)なら None、そうでなければ Some((lat, lon))
    prev_t = t
None
```
検証: 中央画素が注視点を拾う。空を向く上端画素は`None`。正射影は画素→東西南北オフセットが線形(`view_height/canvas_h`)。データ範囲外の交点は`None`。

---

## 4. 断面プロファイル `terrain::profile`

```
NUM_SAMPLES = 300;  SEARCH_UPPER_BOUND_M = 1,000,000
max_valid_distance(data, transform, dir_east, dir_north) -> f64:     // 方位方向にデータ範囲内でいられる最大距離(losも共有)
    in_bounds(d) = transform.inverse(dir_east*d, dir_north*d) が geodetic_bounds 内
    if in_bounds(1,000,000): return 1,000,000
    lo=0, hi=1,000,000 で30回二分探索(in_bounds(mid)ならlo=mid、でなければhi=mid) → lo
build_profile(data, origin, azimuth_deg) -> Vec<ProfilePoint{distance_m, elevation_m: f32, lat_deg, lon_deg}>:
    dir_east=sin(az), dir_north=cos(az)(方位は北=0・東=90・時計回り)
    max_distance = max_valid_distance(...);  <=0 なら空
    0..=300 の i について d = max_distance*i/300、(lat,lon)=transform.inverse(dir*d)、elevation = sample_heightmap or 0.0
```
`inverse`は接平面近似だが、1,000km以内で使うため許容(断面は原点から直線に近い範囲)。

---

## 5. 見通し(覆域)計算 `terrain::los`

### 5.1 定数

| 定数 | 値 | 意味 |
|---|---|---|
| `EARTH_RADIUS_M` | 6,371,000 | 平均地球半径 |
| `K_FACTOR` | 4/3 | 等価地球半径係数(標準大気の電波屈折)。`r_eff = R·k`(固定値、UIパラメータ化していない) |
| `NUM_AZIMUTHS` | 6400 | 1周=6400mil(NATO式)、1mil=360/6400度=0.05625度 |
| `SAMPLES_PER_RAY` | 1000 | 1方位あたりのサンプル数(最大観測範囲50kmで50m間隔) |
| `MAX_TARGET_SAMPLES` | 200 | 対象1点までの判定(`is_visible`/`min_visible_altitude`)のサンプル数上限 |

```
azimuth_deg_of(i) = i*360/6400;   (方位, dir_east, dir_north) = (az, sin(az_rad), cos(az_rad))
curvature_drop(d) = d² / (2·r_eff)
RayContext::new(data, origin, antenna_height): ground = sample_heightmap(origin) or 0;  observer_altitude_msl = ground + antenna_height
                                                transform = EnuTransform::new(origin(=観測点), ellipsoid);  r_eff = 6,371,000*4/3
pub struct LosParams { observer_height_m: f64, max_range_m: f64 }
pub struct LosPoint { azimuth_deg: f64, range_m: f64 }
```
観測点は**それ自身を原点とするENU**で計算する(メッシュ原点とは独立)。標高は`sample_heightmap`(画面に出しているレベル)で引く。

### 5.2 `compute_los`(仰角0°の見通し限界。極座標図用)

```
for az_i in 0..6400:
    ray_max = min(max_valid_distance(...), max_range_m);  ray_max <= 0 なら range=0
    max_angle = -∞;  visible_range = 0
    for i in 1..=1000:
        d = ray_max*i/1000;  (lat,lon) = transform.inverse(dir*d)
        elevation = sample_heightmap or 0
        angle = (elevation - curvature_drop(d) - observer_altitude_msl) / d
        if angle >= max_angle { max_angle = angle; visible_range = d }      // マスク角アルゴリズム
    range_m = visible_range
```
手前の尾根の陰でも、その先で地形が十分高くなれば再び見える(単純な「最初の遮蔽物で打ち切り」より実際の覆域に近い)。

### 5.3 `is_visible` / `min_visible_altitude`(断面図用)

```
transform_to_target(t, lat, lon): pos = t.transform(lat,lon,0);  dist = hypot(pos.e,pos.n);  dist<1 なら (dist,0,0) 、それ以外 (dist, e/dist, n/dist)
is_visible(観測点(lat,lon,height,max_range), 対象(lat,lon)) -> bool:
    dist<1 → true;  dist>max_range → false
    target_angle = (elev(target) - drop(dist) - observer_alt) / dist
    samples = clamp(ceil(dist/500), 10, 200)
    for i in 1..samples: d = dist*i/samples; angle = (elev(d)-drop(d)-observer_alt)/d;  if angle > target_angle → false
    true
min_visible_altitude(…) -> Option<f64>:                   // 「これ以上の海抜高度なら見える」の下限
    dist>max_range → None;  dist<1 → Some(observer_altitude_msl)
    required_angle = max over i in 1..samples of (elev(d)-drop(d)-observer_alt)/d       (target自身の標高には依存しない)
    Some(required_angle*dist + drop(dist) + observer_alt)          // 仰角式が対象高度hについて線形なのでhを直接解く
```

### 5.4 `compute_coverage_area`(2D: 指定海抜高度の探知可能領域)

高度一定の直線を走査する。`compute_los`と同じマスク角に加え、対象(高度`target_altitude_m`固定)の仰角を同時に更新する。
```
for az_i: ray_max (5.2と同じ);  max_angle=-∞; visible_range=0
    for i in 1..=1000:
        d = ...;  angle = (elev - drop(d) - observer_alt)/d;   if angle > max_angle { max_angle = angle }        // 注意: 5.2は>=、ここは>
        target_angle = (target_altitude_m - drop(d) - observer_alt)/d
        if target_angle >= max_angle { visible_range = d } else { break }             // 最初に遮蔽されたら打ち切り(直線は陰から出てこない)
```

### 5.5 `compute_los_dome`(3D: 半球ドーム。仰角一定の直線)

入力: 仰角配列`elevation_degs`(**0以上90未満の昇順**、`debug_assert`)。出力: `Vec<DomeRing{elevation_deg, points: Vec<LosPoint(range_m=スラントレンジ)>}>`。
物理モデル: 直線(仰角一定)は一度遮蔽されたら二度と陰から出ないので**最初の遮蔽で打ち切る**。遮蔽されない方角は最大観測範囲(スラント)まで届く(=どの仰角でも同じ半径の滑らかな球面)。
```
ring_tans[k] = tan(el_k), ring_cos[k] = cos(el_k);   ring_slant_ranges = [[0.0; 6400]; num_rings]
for az_i:
    data_max = max_valid_distance(...)
    horizontal_cap(k) = min(data_max, max_range * ring_cos[k])            // 高仰角ほど水平距離の上限は小さい
    ray_max = max_k horizontal_cap(k);  <=0 なら continue
    sample_distance(i) = ray_max*i/1000
    finalize(k, reach):                // reach = 遮蔽されずに届いた最後のサンプル番号(0=手前で遮蔽)
        cap = horizontal_cap(k)
        cap_reached = reach >= 1000 || sample_distance(reach+1) > cap
        d = cap_reached ? cap : sample_distance(reach)                    // 上限まで届いたら**サンプル位置に丸めず上限ちょうど**(丸めると高仰角ほど半径が不揃いで球にならない)
        if d > 0: ring_slant_ranges[k][az_i] = ring_cos[k] > 1e-6 ? d/ring_cos[k] : d
    max_angle=-∞;  first_alive=0                                            // max_angleは単調非減少 → 遮蔽が確定したリングは先頭から順に確定
    for i in 1..=1000:
        angle = (elev(d_i) - drop(d_i) - observer_alt)/d_i
        if angle > max_angle:
            max_angle = angle
            while first_alive < num_rings && max_angle > ring_tans[first_alive]: finalize(first_alive, i-1); first_alive++
            if first_alive == num_rings: break
    for k in first_alive..num_rings: finalize(k, 1000)
```
この単調性を使った最適化は、全リング×全サンプル走査の素朴実装と**完全に同じ結果**になる(合成地形で17万9千点一致を確認済み。素朴実装との比較テストを書くこと)。

### 5.6 性能目標と検証

- 観測点1つ=6400方位×1000サンプル=640万回の`sample_heightmap`。リリースビルド(WASM)で、覆域の再計算+再描画は3Dで約470ms(ドーム約230ms+極座標図約240ms)、2Dで約230msを目標(許容上限1秒)。
- 検証(標高0mの平坦地形、原点=観測点): `los_range_on_flat_ground_is_the_radio_horizon_in_every_direction`(全方位で電波の地平線距離に一致。`sqrt(2·r_eff·(h_obs))`相当)、
  `point_visibility_on_flat_ground_follows_the_radio_horizon`、`a_ridge_hides_what_is_behind_it`(観測点の東約16kmの標高1,500m尾根)、
  `min_visible_altitude_is_consistent_with_visibility`、`coverage_area_reaches_the_max_range_for_a_high_target_and_stops_at_the_ridge`、
  `dome_rings_are_full_spheres_over_flat_ground`(平坦なら全リングが最大観測範囲のスラントレンジ)、`curvature_drop_grows_with_the_square_of_distance`、`azimuth_index_is_in_mils`。
