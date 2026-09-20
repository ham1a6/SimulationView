# 実装仕様 第4部: 地形の上に重ねるもの(観測点・覆域・作図・航跡)

設計書体系([README](../README.md))の詳細設計(ライブラリ実装仕様)第4部で、[IMPLEMENTATION_GUIDE.md](../IMPLEMENTATION_GUIDE.md)の一部。
上位設計(方針・理由・図): [詳細設計書](../DETAILED_DESIGN.md) 6.9節(観測点・覆域)・6.11節(作図)・6.12節(航跡)。数値・手順はこの文書が正。

`terrain::{render_bias, markers, drawing, drawing_geometry, draw_tool, tracks}`。
すべて「状態(Leptosのシグナルcontext)」+「頂点列を作る純粋関数」に分かれ、頂点列は[第3部](3_renderer_and_shaders.md)の`DrawVertex`/`TerrainVertex`で表す。
前提: 第1〜3部。

---

## 1. Zファイティング対策の持ち上げ量 `terrain::render_bias`

地表に貼り付くものは、地形メッシュ(粗いLODを含む)と同じ深度になると縞になったり、LODの高さのずれで地面に埋まって消える。地表の標高にこの高さ(m)を足して置く。
値は種類ごとに実機で調整したもので、揃えていない。

| 定数 | 値(m) | 使い道 |
|---|---|---|
| `DRAWING_M` | 15.0 | 作図の図形・線(`AboveGround`) |
| `TRACK_M` | 25.0 | 航跡(地表基準)と高度線の足元 |
| `MARKER_M` | 25.0 | 観測点マーカー(ピン)の先端 |
| `DOME_M` | 20.0 | 覆域ドーム全体(遮蔽される方角ではドーム境界が定義上ちょうど地表に接するので、一律に持ち上げる) |
| `COVERAGE_AREA_M` | 20.0 | 2D覆域(深度テストはしないが、正射影の奥行き範囲に収めるため地表の高さに置く) |

シェーダー側にも対の深度バイアスがある(`draw.wgsl`の`LINE_DEPTH_BIAS`・`BILLBOARD_DEPTH_BIAS`、`terrain.wgsl`の`WATER_DEPTH_MARGIN_M`)。**両方をセットで調整する**。

---

## 2. レーダー観測点と覆域 `terrain::markers`

### 2.1 状態

```rust
#[derive(Clone, Copy, PartialEq)] pub struct RadarMarker { pub id: u64, pub lat_deg: f64, pub lon_deg: f64, pub height_m: f64 /*アンテナ高(地表から)*/, pub max_range_m: f64 }
#[derive(Clone, Copy)] pub struct RadarMarkersState {
    pub markers: RwSignal<Vec<RadarMarker>>, pub selected: RwSignal<Option<u64>>, next_id: RwSignal<u64>,
    pub coverage_altitude_m: RwSignal<f64>,           // 2D表示の覆域の対象海抜高度。既定1000.0
}
RadarMarkersState::new()            // markers=[], selected=None, next_id=1, coverage_altitude_m=1000.0
add(lat, lon) -> u64                // 既定 height_m=10.0, max_range_m=50_000.0 で追加し、選択状態にする。IDは単調増加
remove(id)                          // 一覧から除去。選択中なら selected=None
```
観測点は**緯度経度の絶対値**で保持し、メッシュ原点(`OriginState`)とは独立(原点が変わっても値は変わらず、描画側が再変換するだけ。シミュレーション中でも自由に編集できる)。

### 2.2 色・寸法定数

| 定数 | 値 |
|---|---|
| `SELECTED_MARKER_COLOR` | `[1.0, 0.92, 0.25, 1.0]`(選択中=黄) |
| `MARKER_COLOR` | `[1.0, 0.55, 0.15, 1.0]`(非選択=橙) |
| `MARKER_OUTLINE_COLOR` | [0.08, 0.08, 0.10, 1.0] |
| `DOME_SURFACE_COLOR` | `[0.3, 0.9, 1.0]`(RGB。アルファはシェーダーの0.22) |
| `PIN_HEAD_CENTER_PX / PIN_HEAD_RADIUS_PX / PIN_OUTLINE_PX / PIN_DOT_RADIUS_PX / PIN_HEAD_SEGMENTS` | 26.0 / 10.0 / 2.5 / 4.0 / 24 |
| `DOME_RING_ELEVATIONS_DEG`(38個) | 0,1,…,10(1°刻み) / 12,14,…,30(2°) / 33,36,…,60(3°) / 64,68,72,76,80,84,87(4°、最上段87°) |
| `DOME_AZIMUTH_STRIDE` | 2(計算は6400方位のまま、平滑化後に3200方位へ間引いて描く) |
| `DOME_APEX_SEGMENTS` | 48 |
| `DOME_SMOOTH_MEDIAN_HALF / _MEAN_HALF` | 3 / 6 |
| `COVERAGE_AREA_COLOR / _ALPHA` | [0.35, 0.9, 0.4] / 0.32 |
| `COVERAGE_OUTLINE_COLOR / _WIDTH_PX` | [0.75, 1.0, 0.4, 1.0] / 2.5 |
| `COVERAGE_SMOOTH_MEDIAN_HALF / _MEAN_HALF` | 2 / 3 |

### 2.3 円環の平滑化 `smooth_circular(values, median_half, mean_half) -> Vec<f64>`

端は反対側へつながる(`rem_euclid`)。①各iで前後`median_half`個ずつ(計`2*mh+1`個)を昇順にしてその**中央値**(外れ値除去。段差の位置は保つ)
→ ②その結果の前後`mean_half`個ずつ(`2*ah+1`個)の**平均**(段差をなだらかに)。空入力は空を返す。
検証: 定数列は変わらない、1点だけのスパイクは除かれる、端をまたいで平滑化される、段差は柔らかくなる。

### 2.4 マーカー(ピン)`build_marker_geometry(data, mesh_origin, markers, selected) -> Vec<DrawVertex>`

各観測点について `push_marker_pin`:
- `ground = sample_heightmap(marker) or 0`、`anchor = mesh_transform.transform(lat, lon, ground + MARKER_M)`(`mesh_transform`=メッシュ原点のENU変換)。
- すべて`DrawVertex::billboard(anchor, offset_px, color)`の三角形(画面サイズ固定・常に正面)。`push_tri`は3頂点。
- `push_pin_shape(anchor, color, head_radius, tip_y)`: 頭の円=中心`(0, PIN_HEAD_CENTER_PX)`から円周の扇形`PIN_HEAD_SEGMENTS`(24)枚
  (`circle(i) = (r·cos(τ i/24), 26 + r·sin(τ i/24))`)。先端の三角形は`(0,tip_y)`と、頭の円への**接線の接点**`(±tx, ty)`:
  `β = acos(r/(26 - tip_y))`, `tx = r·sin β`, `ty = 26 - r·cos β`。
- 積む順: ① 縁取り `push_pin_shape(MARKER_OUTLINE_COLOR, r = 10+2.5, tip_y = -2.5·1.4)` → ② 本体 `push_pin_shape(color, r=10, tip_y=0)` → ③ 中の点(縁取り色、半径4、24枚の扇形)。
  選択中は`SELECTED_MARKER_COLOR`、それ以外は`MARKER_COLOR`。

### 2.5 3D覆域ドーム `build_dome_surface_geometry(...) -> Vec<TerrainVertex>`(**選択中の1つだけ**)

`push_dome_surface`:
1. `local_transform = EnuTransform::new(観測点を原点, ellipsoid)`、`rings = compute_los_dome(data, 観測点, params, DOME_RING_ELEVATIONS_DEG)`。
   `azimuth_indices = (0..6400).step_by(2)`(3200方位)。`num_azimuths<2 || rings.len()<2`なら空。
2. `observer_height = sample_heightmap(marker) + height_m`。
3. リングごとに`smooth_circular(range_m全方位, 3, 6)`(6400方位で平滑化してから間引く)。
4. ドーム頂点(リングk・方位az_i): `el=elevation_deg`, `az=azimuth_deg`, `range=smoothed[az_i]`, `horizontal = range·cos(el)`,
   `(lat,lon) = local_transform.inverse(horizontal·sin az, horizontal·cos az)`、`h = observer_height + range·sin(el) + DOME_M`、
   `pos = mesh_transform.transform(lat, lon, h)`、`TerrainVertex::unlit(pos, DOME_SURFACE_COLOR)`。
5. リング間の四角形パッチ: リングk・k+1、方位az_i・az_next=(az_i+1)%N で`(a,b,c,d)=(low[az_i], low[next], up[az_i], up[next])`→三角形`a,b,c, b,d,c`(表裏とも見えるので巻き順は問わない)。
6. 最上段リングを閉じる傘: `apex = mesh_transform.transform(marker.lat, marker.lon, observer_height + 平均(最上段の平滑化range) + DOME_M)`。
   最上段リングの方位を`step_by(max(N/48,1))`に**間引いて**(約48枚)、`[top[az_i], top[az_next], apex]`の三角形。
   間引く理由: 全方位で傘にすると極細の三角形が1点に大量に重なり、**半透明合成の描画順依存でカメラ操作中にチカチカする**。リング間パッチは重ならないので間引かない。

### 2.6 2D覆域 `build_coverage_2d_geometry(..., target_altitude_m) -> Vec<DrawVertex>`(選択中の1つだけ)

`push_coverage_2d`:
1. `points = compute_coverage_area(data, 観測点, params, target_altitude_m)`(6400方位の水平距離)。`ranges = smooth_circular(_, 2, 3)`。
2. `position_at(lat, lon) = mesh_transform.transform(lat, lon, sample_heightmap(lat,lon) + COVERAGE_AREA_M)`(**覆域の高度ではなく地表に貼る**。地図上の塗り分けオーバーレイであるため)。
3. 境界点 = 各方位の`local_transform.inverse(range·sin az, range·cos az)`→`position_at`。中心 = `position_at(観測点)`。
4. 塗り: 各iで`[center, boundary[i], boundary[(i+1)%n]]`を`DrawVertex::surface(pos, [0.35,0.9,0.4,0.32], None)`(星形領域なので自己交差しない=ファンで足りる。3Dと違い2Dは回転しないので間引き不要)。
5. 輪郭: `append_line_strip(out, &boundary, closed=true, COVERAGE_OUTLINE_COLOR, 2.5px)`。
6. **描画は深度テストなし**(`draw_screen`パイプライン+絶対座標のuniform)。深度テストありだと、観測点から境界への大きな三角形が地形の起伏(数百m)に埋まって塗りが欠けて不均一になる(2Dは真上からの正射影なので隠れることがない)。
   3Dと2Dでバッファ・パイプラインが別。モード切替時に使わない方は空にする。

---

## 3. 作図のデータモデル `terrain::drawing`

```rust
pub type DrawingId = u64;
#[derive(Copy, PartialEq, Serialize, Deserialize)] pub struct Color { r, g, b, a: f32 }      // 各0..1、アルファ非乗算
  定数: WHITE(1,1,1) BLACK RED(1,0.2,0.2) GREEN(0.2,0.9,0.3) BLUE(0.2,0.5,1.0) YELLOW(1,0.9,0.2);  rgba/rgb/with_alpha(a)/to_array()
#[derive(Copy, PartialEq, Serialize, Deserialize)] pub struct Style { fill: Option<Color>, stroke: Option<Color>, stroke_width_px: f32 }
  filled(c)=fill=c,stroke=None,width=2.0 / stroked(c,w) / fill_and_stroke(f,s,w);  Default=fill(0.2,0.6,1.0,0.35)+stroke(0.2,0.6,1.0)+width2.0
pub enum Altitude { Msl(f64) /*海抜。2D図形はその高さの水平面*/, AboveGround(f64) /*地表から。2D図形・線は地形に貼り付く。3D図形は真下の地表を基準に置くだけ*/ }
pub enum Corner { TopLeft, TopRight, BottomLeft, BottomRight, Center }
pub enum Position { World{lat_deg,lon_deg,altitude}, Screen{corner,x_px,y_px}, View{right_m,up_m,forward_m} }   // コンストラクタ world/screen/view。space()→Space
pub enum Space { World, View, Screen }
pub enum Shape {
    Circle{center, radius}, Rect{center, width, height, rotation_deg}, Polygon{points}, Sector{center, radius, start_deg, end_deg},
    Sphere{center, radius}, Cuboid{base_center, size_m:[f64;3], heading_deg}, Cylinder{base_center, radius, height}, Cone{base_center, radius, height},
    Polyline{points},
}
pub struct Drawing { pub id, pub shape, pub style, pub visible: bool }
pub struct DrawingState { pub items: RwSignal<Vec<Drawing>>, next_id: RwSignal<u64> }   // Copy
```

`Shape`/`Position`/`Altitude`/`Corner`/`Style`/`Color`は`serde::{Serialize,Deserialize}`(localStorage保存用)。

- 角度は**時計回りの度数、0度が上(World=北)**。`Rect.rotation_deg`は時計回り。`Sector`は`start_deg`から`end_deg`まで時計回り。`Cuboid.size_m = [東西(横), 南北(奥行), 高さ]`、`heading_deg`だけ時計回り。
- 大きさの単位: `World`/`View`はメートル、`Screen`はピクセル。3D図形の位置は球=中心、他=底面の中心。多角形は全頂点を**先頭の点の高度**の面に置く。
- `Shape::validate() -> Result<Space, &'static str>`: 位置が1つも無い → `Err("位置が指定されていない")`。種類が混在 → `Err("位置の種類(World/Screen/View)が混ざっている")`。
  `Screen`に3D図形 → `Err("3D図形は画面座標(Screen)に置けない(ViewかWorldを使う)")`。それ以外は`Ok(space)`。不正な図形は描かず警告ログ。
- `positions() / positions_mut()`: 中心1つ、または点の並び(`Cuboid/Cylinder/Cone`は`base_center`)。`is_solid()`: Sphere/Cuboid/Cylinder/Cone。
  `depends_on_terrain()`: `AboveGround`の`World`位置を1つでも持つ(地形LODが変わったら描き直す必要がある)。
- `DrawingState`: `new()`(items=[], next_id=1)、`add(shape, style) -> id`(visible=true。後ろに追加したものほど手前=`Screen`の重なり順、`World/View`の半透明どうしの重なり順)、
  `update(id, |&mut Drawing|)`(無ければ何もしない)、`remove(id)`、`clear()`。

---

## 4. 作図のジオメトリ生成 `terrain::drawing_geometry`(純粋関数)

`build(ctx, drawings) -> DrawingBatches`。地形なしで単体テストできるよう、標高は`ctx.ground: &dyn Fn(lat,lon)->f64`で受ける。

```rust
pub struct Batch { pub opaque: Vec<DrawVertex>, pub blend: Vec<DrawVertex> }        // opaque=アルファ>=0.999
pub struct DrawingBatches { pub world: Batch, pub view: Batch, pub screen: Vec<DrawVertex> }   // Screenは追加順に重ねるので1列
pub struct BuildContext<'a> { mesh_transform: &EnuTransform /*現在のメッシュ原点*/, ellipsoid: &Ellipsoid, ground: &dyn Fn(f64,f64)->f64 /*範囲外・海は0*/, viewport_px: (f32,f32) }
```
`visible`でない・`validate`が`Err`の図形は飛ばす。`Space`で出力先を選び(World→`world`、View→`view`、Screen→`screen`)、色のアルファ>=`OPAQUE_ALPHA(0.999)`なら`opaque`、未満なら`blend`(`Screen`は常に1列)。

### 4.1 出力座標系

| Space | 頂点座標 | 使う`view_proj` |
|---|---|---|
| World | 現在のメッシュ原点のENU(x東,y北,z上) | カメラの`view_proj` |
| View | カメラから見た座標 (右, 上, **-前方**) | `Camera::projection_matrix()`(ビュー行列なし) |
| Screen | ピクセル座標(左上原点、x右、y下、z=0) | `screen_matrix(w,h)` |

### 4.2 面と太い線の頂点

- 面: `push_triangle(color, [3位置], normals: Option<[3法線]>)` → `DrawVertex::surface`。
- 太い線 `append_line_strip(list, points, closed, color, width_px)`(`markers`・`tracks`も使うので`pub(crate)`):
  `segments = closed ? n : n-1`。各線分(a→b)(`a==b`は飛ばす)について 4頂点
  `a+=line(a,b,.., +1)`, `a-=line(a,b,.., -1)`, `b+=line(b,a,.., -1)`, `b-=line(b,a,.., +1)` を作り、三角形 `[a+, a-, b+,  b+, a-, b-]`。`n<2`なら何もしない。

### 4.3 緯度経度 ⇔ 基準点からの方位・距離(方位角等距離図法。球面の直接解)

```
mean_radius(ellipsoid, lat) = a·sqrt(1-e2) / (1 - e2·sin²(lat))              // 子午線と卯酉線の曲率半径の幾何平均
destination(lat, lon, bearing_rad, dist_m, radius) -> (lat2, lon2)            // 北から時計回りの方位
    δ = dist/radius
    lat2 = asin(sin lat1·cos δ + cos lat1·sin δ·cos bearing)
    lon2 = lon1 + atan2(sin bearing·sin δ·cos lat1,  cos δ - sin lat1·sin lat2)
to_local(ref_lat, ref_lon, lat, lon, radius) -> [east, north]                 // destinationの逆
    dlon = wrap(lon-ref_lon → (-π, π]);  a = sin²((lat2-lat1)/2) + cos lat1·cos lat2·sin²(dlon/2)
    dist = 2·radius·asin(min(sqrt(a),1));  bearing = atan2(sin dlon·cos lat2,  cos lat1·sin lat2 - sin lat1·cos lat2·cos dlon)
    [dist·sin bearing, dist·cos bearing]
height_of(ctx, lat, lon, altitude, ground_bias) = Msl(h) → h / AboveGround(o) → ctx.ground(lat,lon) + o + ground_bias
resolve_screen(corner, x, y) = corner基準点(TopLeft=(0,0), TopRight=(w,0), BottomLeft=(0,h), BottomRight=(w,h), Center=(w/2,h/2)) + (x, y)
```

### 4.4 2D図形 `Geom2d` と `Frame2d`

2D図形は、置いた位置を中心とする**ローカル平面**(x=右/東, y=上/北。m または px)で三角形`fill`(3点ずつ)と輪郭`outlines: Vec<{points, closed}>`を作り、`Frame2d`で出力座標へ写す。

`Frame2d::at(ctx, position) -> (frame, steps)`:
- World: `radius=mean_radius(ellipsoid, lat)`。`steps = Msl → STEPS_FLAT{fill:20000, arc:2000} / AboveGround → STEPS_GROUND{fill:500, arc:250}`。
- View / Screen: `steps = STEPS_NONE`(∞=分割しない。平面のまま)。
`frame.map(ctx, p:[x,y]) -> [f32;3]`:
- World: `(lat,lon) = destination(ref_lat, ref_lon, atan2(p.x,p.y), hypot(p.x,p.y), radius)`、`ctx.mesh_transform.transform(lat, lon, height_of(ctx, lat, lon, altitude, DRAWING_M))`。
- View: `[right+p.x, up+p.y, -forward]`。Screen: `[x+p.x, y-p.y, 0]`(y反転)。
`frame.local_of(ctx, pos)`(多角形の各頂点をローカルへ): World=`to_local`、View=`[right_m-right, up_m-up]`、Screen=`[px-x, y-py]`。

分割定数: `MAX_FILL_TRIANGLES=50_000`、`MIN_CIRCLE_SEGMENTS=48`、`MAX_CIRCLE_SEGMENTS=720`、`MAX_GRID_CELLS=512`、`MAX_EDGE_PARTS=2000`、`MAX_REFINED_VERTICES=300_000`。
`effective_fill_step(area, fill) = max(fill, sqrt(2·area/MAX_FILL_TRIANGLES))`。

- **円・扇形 `sector_geom(radius, start_deg, end_deg, steps)`**: `radius>0`でなければ空。`raw = end-start`、`full = raw >= 360-1e-9`、`sweep = full ? 360 : raw.rem_euclid(360)`(`<1e-9`なら空)。
  `arc = min(steps.arc, radius·0.09)`、`full_segments = clamp(ceil(τ·radius/arc), 48, 720)`、`n = max(ceil(full_segments·sweep/360), 1)`、
  `area = 0.5·r²·sweep_rad`、`rings = clamp(ceil(radius/effective_fill_step(area, steps.fill)), 1, 512)`。
  `at(r,i) = [r·sin θ, r·cos θ]`、`θ = (start + sweep·i/n)°`。リングk=1..rings(`r0=radius(k-1)/rings, r1=radius·k/rings`)、各i: k==1なら`[center, at(r1,i), at(r1,i+1)]`、
  それ以外は四角形`[at(r0,i), at(r1,i), at(r1,i+1),  at(r0,i), at(r1,i+1), at(r0,i+1)]`。
  輪郭: 円=`at(radius,i) for i in 0..n`(閉)、扇形=`[center] + at(radius,i) for i in 0..=n`(閉。中心→弧→中心)。
- **矩形 `rect_geom(w, h, rot, steps)`**: `w>0&&h>0`でなければ空。`s=effective_fill_step(w·h, steps.fill)`、`nx=clamp(ceil(w/s),1,512)`、`ny`同様。格子点`p(ix,iy) = rotate_cw([-w/2 + w·ix/nx, -h/2 + h·iy/ny], rot)`。
  各セル`(a,b,c,d) = p(ix,iy), p(ix+1,iy), p(ix+1,iy+1), p(ix,iy+1)` → `[a,b,c, a,c,d]`。輪郭=4隅`[p(0,0),p(nx,0),p(nx,ny),p(0,ny)]`を`densify(閉, steps.arc)`。
  `rotate_cw([x,y], deg) = [x·cos + y·sin, -x·sin + y·cos]`(deg→rad、x右・y上で時計回り)。
- **多角形 `polygon_geom(points, steps)`**: 先頭==末尾なら末尾を除く。3点未満・面積0は空。符号付き面積<0(時計回り)なら反転して反時計回りに揃える。
  `step = effective_fill_step(|area|, steps.fill)`、`fill = refine_triangles(triangulate(poly), step)`、輪郭=`densify(poly, 閉, steps.arc)`。
  - `triangulate(poly)`: `earcutr::earcut(flat, &[], 2)`(凹・共線を含む入力も扱える)。出力の三角形を**反時計回りに揃える**(`orient(a,b,c)<0`なら`[a,c,b]`)。3点未満・失敗は空。
  - `refine_triangles(tris, max_edge)`: 最長辺が`max_edge`を超える三角形を、辺の中点で**4分割**(`[a,ab,ca],[ab,b,bc],[ca,bc,c],[ab,bc,ca]`)することを繰り返す(スタック)。
    `out.len()+stack.len()*3 >= 300_000`になったらそれ以上分割しない。
  - `densify(points, closed, max_len)`: `max_len`が有限でなければそのまま。各辺を`parts=clamp(ceil(len/max_len),1,2000)`等分(閉のとき最初の点は重ねない、開のとき最後の点を足す)。
- `emit_geom2d`: `style.fill`があれば`fill`の各三角形を`frame.map`して`push_triangle(color, pos, None)`(法線なし=陰影なし)。
  `style.stroke`があり`stroke_width_px>0`なら各`outline`を`frame.map`して`push_line_strip`。

### 4.5 折れ線

`emit_polyline`: `stroke`が無い・幅0以下は何もしない。
- World `world_polyline(ctx, points)`: 隣接2点(lat0,lon0,alt0)→(lat1,lon1,alt1)ごとに `radius=mean_radius(lat0)`、`[e,n]=to_local(lat0,lon0,lat1,lon1,radius)`、`length=hypot`、`bearing=atan2(e,n)`。
  `grounded = どちらかがAboveGround`、`step = grounded ? 250 : 2000`(`POLYLINE_STEP_*_M`)、`parts=clamp(ceil(length/step), 1, 4000)`。
  `offset(alt) = Msl(h) → h - ground(lat,lon) / AboveGround(o) → o`。k in 0..parts、`t=k/parts`: `(lat,lon)=destination(lat0,lon0,bearing,length·t,radius)`、
  `h = grounded ? ground(lat,lon) + off0 + (off1-off0)·t + DRAWING_M : h0 + (h1-h0)·t`(`h0,h1 = height_of(…, DRAWING_M)`)。`mesh_transform.transform(lat,lon,h)`。**最後に終点**(`height_of(…, DRAWING_M)`)を足す。
- View: `[right, up, -forward]`。Screen: `[x, y, 0]`(`resolve_screen`)。
- 出力は`push_line_strip(mapped, closed=false, stroke, width)`。

### 4.6 3D図形 `Solid` と `Frame3d`

```rust
struct Solid { verts: Vec<([f64;3] 位置, [f64;3] 単位法線)>, indices: Vec<u32>, lines: Vec<(Vec<[f64;3]>, bool /*閉*/)> }   // ローカル座標: x=東/右, y=北/前方, z=上
```
定数: `SPHERE_SEGMENTS=48`, `SPHERE_RINGS=24`, `SOLID_SEGMENTS=48`。`Solid::ring(r,z)` = `[r cos t, r sin t, z]`, `t=τ i/48`(i in 0..48)。
`push_quad(corners[4], normal)`: 4頂点(同じ法線)を追加し`[b, b+1, b+2,  b, b+2, b+3]`。`rotate_heading(deg)`: 位置・法線・稜線のx,yを`rotate_cw`(z不変)。

- **球** `sphere_solid(r)`(`r>0`でなければ空): i in 0..=24, j in 0..=48: `φ=π·i/24, θ=τ·j/48`、`n=(sin φ cos θ, sin φ sin θ, cos φ)`、頂点=`(r·n, n)`。
  三角形(i in 0..24, j in 0..48): `a=i·49+j, b=a+49` → `[a,b,a+1,  a+1,b,b+1]`。稜線=互いに直交する3つの大円(赤道`ring(r,0)`と、その軸を入れ替えた2つ: `[p.y→…]`)
  (`equator.map(|p| [p[0],p[2],p[1]])`、`equator.map(|p| [p[2],p[0],p[1]])`、`equator`)、いずれも閉。
- **直方体** `cuboid_solid([sx,sy,h], heading)`(3辺すべて>0): `hx=sx/2, hy=sy/2`。6面のquad(法線付き):
  `+x: [(hx,-hy,0),(hx,hy,0),(hx,hy,h),(hx,-hy,h)]` / `-x: [(-hx,hy,0),(-hx,-hy,0),(-hx,-hy,h),(-hx,hy,h)]` / `+y: [(hx,hy,0),(-hx,hy,0),(-hx,hy,h),(hx,hy,h)]` /
  `-y: [(-hx,-hy,0),(hx,-hy,0),(hx,-hy,h),(-hx,-hy,h)]` / `+z(天): [(-hx,-hy,h),(hx,-hy,h),(hx,hy,h),(-hx,hy,h)]` / `-z(底): [(-hx,hy,0),(hx,hy,0),(hx,-hy,0),(-hx,-hy,0)]`。
  稜線=4本の縦線(開、底の角→天の角)+底面の閉ループ+天面の閉ループ。最後に`rotate_heading(heading)`。底面中心が原点。
- **円柱** `cylinder_solid(r,h)`: 側面=j in 0..=48で`(r cos t, r sin t, 0)`と`(…, h)`を交互に(法線`(cos t, sin t, 0)`)、`[bottom,next_bottom,top,  top,next_bottom,next_top]`。
  上下の蓋=中心頂点+`ring`(法線`(0,0,±1)`)、`[center, center+1+j, center+1+(j+1)%48]`(天z=h・法線+z、底z=0・法線-z)。稜線=90°ごと4本の縦線(開)+底・天の円(閉)。
- **円錐** `cone_solid(r,h)`: `slant=hypot(r,h)`、側面法線 `(h cos t/slant, h sin t/slant, r/slant)`。底の円周j in 0..=48に頂点、各j in 0..48に**専用の頂点先端**`(0,0,h)`(法線は`t=τ(j+0.5)/48`の側面法線)を追加し`[j, j+1, apex]`。
  底面の蓋=中心+`ring(r,0)`(法線-z)。稜線=90°ごと4本(底の点→先端、開)+底の円(閉)。

`Frame3d`:
- World: **位置における局所ENU**(位置を通る鉛直線が+z)で作り、`(lat,lon,h) = local.enu_to_geodetic(p.x, p.y, base_height + p.z)`→`mesh_transform.transform(lat,lon,h)`で厳密に変換
  (遠方でも地球の丸みで傾いた上向きが正しい)。`base_height = Msl(h)→h / AboveGround(o)→ground(lat,lon)+o`(**3D図形は`render_bias`の持ち上げを付けない**)。`local = EnuTransform::new(Origin{lat,lon}, ellipsoid)`。
  法線はそのまま`[nx,ny,nz]`(f32)。
- View: 位置`[right+p.x, up+p.z, -(forward+p.y)]`、法線`[nx, nz, -ny]`((東,北,上)→(右,前方=-z,上))。
- Screen: 3D図形は置けない(`None`)。

`emit_solid`: `fill`があれば頂点をmapして三角形を`push_triangle(color, pos, Some(法線))`(陰影あり)。`stroke`(幅>0)があれば稜線を`push_line_strip`。

### 4.7 検証(必須テスト。`drawing_geometry`の`#[cfg(test)]`)

`local_coordinates_round_trip`(to_local∘destination)、`destination_matches_enu_scale`、円・矩形の面積(三角形の面積総和)、扇形の角度、凹多角形の面積が保たれる、
不透明/半透明の振り分け、不正・不可視の図形は飛ばす、折れ線は線分あたり三角形2枚、`Screen`の角とオフセット、視点空間の立体が前方を向く、立体のインデックスが有効で法線が単位長、
`World`の立体が地面に立つ、`AboveGround`は地形に沿い`Msl`は水平、長い折れ線が細分される、`triangulate`が向き・凹・共線を扱い退化入力で空を返す。

---

## 5. 図形の対話作成 `terrain::draw_tool`(`DrawToolState`)

作図の上に載る別context。作る図形はすべて絶対座標(`Position::World`)。

### 5.1 定数・ツール

`MIN_POINT_SPACING_M = 1.0`(直前の点とこれ未満のクリックは無視=ダブルクリックの2回目)。`SAVE_VERSION = 1`。

`ToolKind::ALL = [Circle, Rect, Polygon, Sector, Polyline, Sphere, Cuboid, Cylinder, Cone]`。ラベル: 円/矩形/多角形/扇形/折れ線/球/直方体/円柱/円錐。

| ツール | 必要クリック数(`fixed_points`) | 確定 |
|---|---|---|
| Circle, Rect, Sphere, Cuboid, Cylinder, Cone | 2 | 2点目で自動 |
| Sector | 3 | 3点目で自動 |
| Polygon | なし(≥3) | ダブルクリック/Enter(`finish`) |
| Polyline | なし(≥2) | 同上 |

案内文`hint(placed)`(`{ツール名}: {文}`の形で`hint()`が返す): Circle 0「円の中心をクリック」/以降「円周上の点をクリック(中心からの距離が半径)」、
Rect「矩形の1つ目の角をクリック」/「対角の角をクリック(東西・南北に沿った矩形)」、Sector「扇形の中心をクリック」/「扇の開始側の縁をクリック(半径と開始方位)」/「扇の終了側の縁をクリック(開始から時計回り)」、
Polygon「頂点を順にクリック(3点以上)」/「次の頂点をクリック / ダブルクリックかEnterで確定」、Polyline「折れ線の始点をクリック」/「次の点をクリック / ダブルクリックかEnterで確定」、
Sphere「球の中心をクリック」/「球の表面の点をクリック(中心からの距離が半径)」、Cuboid「直方体の底面の1つ目の角をクリック」/「底面の対角の角をクリック」、
Cylinder/Cone「…の底面の中心をクリック」/「底面の円周上の点をクリック」。

### 5.2 点→図形 `build_shape(kind, pts, altitude) -> Option<Shape>`(純粋関数)

ヘルパ(すべて楕円体はWGS84固定): `local(from,to)=to_local(.., mean_radius(WGS84, from.lat))`、`offset(from, east, north)=destination(.., atan2(east,north), hypot, radius)`、
`distance`, `bearing_deg(from,to) = atan2(l.east, l.north)→度→rem_euclid(360)`。`sized(r) = (r >= 1.0) ? Some(r) : None`。
- Circle: `[c,e]`、`radius=sized(distance(c,e))?`。Sphere: 同じ半径。`altitude`が`AboveGround(o)`なら`AboveGround(o+radius)`(球は地表に載る)、Mslはそのまま。
- Cylinder/Cone: 半径=`distance`、`height = 2·radius`、`base_center=c`。
- Rect/Cuboid: `[a,b]`、`[dx,dy]=local(a,b)`、`width=sized(|dx|)?, depth=sized(|dy|)?`、`center = offset(a, dx/2, dy/2)`(東西・南北に沿う。回転は数値編集)。
  Rect=`{width, height: depth, rotation 0}`、Cuboid=`{size_m:[width, depth, (width+depth)/2], heading 0}`。
- Sector: `[c,a,b]`、`radius=sized(distance(c,a))?`、`sized(distance(c,b))?`(0なら作らない)、`start=bearing_deg(c,a)`、`sweep=(bearing_deg(c,b)-start).rem_euclid(360)`、`sweep>=1.0`のときだけ`Sector{radius, start_deg:start, end_deg:start+sweep}`。
- Polygon: `len>=3`。Polyline: `len>=2`。各点を`Position::world(lat,lon,altitude)`。

`preview_shape(kind, pts, hover, altitude)`: `pts+hover`で`build_shape`、作れなければ`Polyline`(2点以上)で代替(扇形・多角形の途中経過)。

### 5.3 状態と操作

```rust
pub struct UserShape { pub id: DrawingId, pub name: String }
pub struct DrawToolState { pub drawings: DrawingState,
    pub tool: RwSignal<Option<ToolKind>>, pub points: RwSignal<Vec<(f64,f64)>>, pub new_style: RwSignal<Style>, pub new_altitude: RwSignal<Altitude>,
    pub shapes: RwSignal<Vec<UserShape>>, pub selected: RwSignal<Option<DrawingId>>,
    hover, draft: RwSignal<Option<DrawingId>>, highlight: RwSignal<Option<DrawingId>>, serial: RwSignal<u32> }      // Copy
```
初期値: `new_style = fill_and_stroke((1.0,0.6,0.1,0.35), (1.0,0.7,0.2), 2.0)`(地形の緑・茶にも海の青にも埋もれない橙)、`new_altitude = AboveGround(0.0)`。

| 操作 | 動作 |
|---|---|
| `start(kind)` | 同じツールを再選択したら`cancel`。そうでなければ点を空にして選択、`refresh_draft` |
| `start_at(kind, lat, lon)` | 点・hoverを空にしてツール選択→`click(lat,lon)`(すでに同じツールでも解除しない。右クリックメニュー「ここに図形を作成」用) |
| `cancel()` | ツール解除・点/hoverを空に・`refresh_draft` |
| `click(lat,lon)` | ツール無しなら無視。直前の点から1m未満なら無視。点を追加し、`fixed_points == len`なら`build_shape`→成功で`commit`して点を空に、**失敗(大きさ0等)なら最後の点だけ取り消す**。`refresh_draft` |
| `set_hover(lat,lon)` | 変化があれば保持して`refresh_draft` |
| `finish()` | 可変点数ツールのみ。`build_shape`成功で`commit`、点を空に |
| `undo()` | 点を1つ戻す。無ければ`cancel` |
| `commit(kind, shape)` | `serial+=1`、名前`"{ラベル} {serial}"`で`add_user_shape`、`select(新id)`。**ツールは選んだまま**(続けて作れる) |
| `select(id)` | `selected`を設定し`refresh_highlight` |
| `update_shape(id, f)` | `drawings.update`。選択中なら`refresh_highlight` |
| `duplicate(id)` | 位置を`offset(pos, shift, shift)`(`shift = 特徴サイズ×0.5`、特徴サイズ=半径/幅高さの大きい方の半分/直方体の東西南北の大きい方の半分/多角形・折れ線は1000)して`"{名前} のコピー"`で追加・選択 |
| `rename(id, name)` / `remove(id)` / `remove_all()` | `remove_all`は`shapes`にある図形だけ消す(アプリが足した図形は残す) |
| `wants_hover()` | ツールがあり点が1つ以上(カーソル追従が要る) |
| `can_finish()` | 可変点数ツールで`len >= min_points`(Polygon 3、Polyline 2) |
| `is_active()` | ツール選択中 |

**仮の図形とハイライトは、`drawings`の中の1要素として持つ**(`sync_temp(slot, shape, style)`: 形があり枠がある→更新、形あり・枠なし→追加してid保持、形なし・枠あり→削除)。
- `refresh_draft`: ツール選択中かつ点が1つ以上あれば`preview_shape`。`style = new_style`だが**`stroke`が`None`なら黄色**(折れ線は線の色だけで描くため)。
- `refresh_highlight`: 選択中の図形を`highlight_shape`(2D図形の全位置の高度を**+5m**持ち上げて自身の輪郭とのZファイティングを避ける。3D図形はそのまま)、`Style::stroked(YELLOW, 4.0)`。

**`serial`**: 名前の通し番号。`add_user_shape`のたびに`max(serial, shapes.len())`まで進める(復元した数だけ進めて名前の重複を減らす)。

### 5.4 保存(localStorage)

`persist(key: &'static str) -> Self`: 作成直後に**1度だけ**呼ぶ。`localStorage[key]`のJSON`{version:1, shapes:[{name, shape, style, visible}]}`を読み、`version==1`なら`add_user_shape`+`visible`復元。
版が違う・壊れているなら警告ログを出して読まない。以後、`shapes`と`drawings.items`を購読する`Effect`が**内容が変わったときだけ**JSONを書き戻す(`prev`と文字列比較)。
localStorageの読み書きは全て失敗しうるのでエラーは無視(プライベートウインドウ・容量超過・ブロック)。
検証: JSON往復で全図形が復元される。

---

## 6. 航跡 `terrain::tracks`

### 6.1 モデル

```rust
pub type TrackId = u64;
pub enum SymbolKind { Unknown, Aircraft, Helicopter, Ship, Vehicle, Missile }       // label(): 不明/固定翼機/ヘリコプター/艦船/地上車両/ミサイル
pub enum Affiliation { Unknown, Friendly, Hostile, Neutral }                        // label(): 不明/友軍/敵/中立
   color(): Unknown=(1.0,0.9,0.3), Friendly=(0.35,0.65,1.0), Hostile=(1.0,0.3,0.3), Neutral=(0.4,0.9,0.45)
pub struct Track { id, kind, affiliation, label: String, lat_deg, lon_deg, altitude: Altitude, heading_deg /*北から時計回り*/, speed_mps }
pub struct TrackEntry { pub track: Track, pub trail: Vec<(lat, lon, Altitude)> }            // trailは過去の位置(現在位置は含まない)
pub struct TracksState { pub entries: RwSignal<Vec<TrackEntry>>, pub show_labels/show_trails/show_altitude_lines: RwSignal<bool>(既定すべてtrue), pub selected: RwSignal<Option<TrackId>> }   // Copy
```

- 航跡定数: `TRAIL_MAX_POINTS = 400`、`TRAIL_MIN_STEP_M = 250.0`。`approx_distance_m(lat0,lon0,lat1,lon1)`: `north=(lat1-lat0)·111320`、`east=(lon1-lon0)·111320·cos(lat0)`、`hypot`(等距円筒近似で十分)。
- `advance_trail(previous: Option<TrackEntry>)`: 前回が無ければ空。あれば前回の位置が**trailの最後の点から250m以上**(trailが空なら常に)離れていれば前回の位置をtrailに追加し、400点を超えたら先頭を捨てる。
- `set(tracks: Vec<Track>)`: **受信のたびに全トラックの最新状態を渡す**。前回のエントリをIDで引き当て、同じIDならtrailを`advance_trail`で引き継ぐ。前回に無いIDは新規(trail空)、今回に無いIDは消える。
  選択中のトラックが今回に無ければ`selected=None`。
- `clear()`: 全消去+`selected=None`。`select(Option<id>)`: 変化があるときだけ設定。`selected_track() -> Option<Track>`: リアクティブに最新の`Track`(選択が無い・一覧から消えたら`None`)。

### 6.2 ジオメトリ `build_track_geometry(ctx, entries, options) -> TrackGeometry{ vertices: Vec<DrawVertex>, labels: Vec<TrackLabel> }`

```rust
pub struct TrackOptions { selected: Option<TrackId>, trails: bool, altitude_lines: bool /*3Dのみtrue*/ }
pub struct TrackLabel { id, selected: bool, position: [f32;3] /*メッシュ原点のENU*/, name, detail, color: [f32;3] }
```
定数: `SYMBOL_SIZE_PX=30`, `SYMBOL_OUTLINE_SCALE=1.3`, `SYMBOL_OUTLINE_COLOR=[0.04,0.04,0.07,0.9]`, `ALTITUDE_LINE_MIN_M=30`, `ALTITUDE_LINE_WIDTH_PX=1.0`, `TRAIL_WIDTH_PX=1.5`, `LINE_ALPHA=0.55`,
`SELECT_RING_OUTER_PX=26`, `SELECT_RING_INNER_PX=21`, `SELECT_RING_BAND_PX=3`, `SELECT_RING_SEGMENTS=40`, `PICK_RADIUS_PX=20`。

各エントリについて(この順に頂点を積む):
1. `height = height_of(ctx, lat, lon, altitude, TRACK_M)`、`anchor = mesh_transform.transform(lat, lon, height)`、`base = affiliation.color()`。
2. **航跡**(`options.trails`かつtrail非空): trailの各点を`height_of(…, TRACK_M)`でENUへ→末尾に`anchor`→`append_line_strip(開, base.with_alpha(0.55), 1.5px)`。
3. **高度線**(`options.altitude_lines`): `ground = ctx.ground(lat,lon)`、`height - ground > 30`のとき`foot = transform(lat, lon, ground + TRACK_M)`、`append_line_strip([anchor, foot], base.with_alpha(0.55), 1.0px)`。
4. **選択の輪**(`options.selected == id`): `push_ring(anchor, outer=26, inner=21-1, SYMBOL_OUTLINE_COLOR)`(縁取り)→`push_ring(anchor, outer=21+3, inner=21, 白)`。
   `push_ring`: i in 0..40、`at(r,i)=[r cos, r sin](τ i/40)`、四角形`(outer_i, outer_i+1, inner_i+1, inner_i)`→`[a,b,c,a,c,d]`を`DrawVertex::billboard`で。
5. **シンボル**: 縁取り(`SYMBOL_OUTLINE_COLOR`、スケール`15·1.3`)→本体(所属色、スケール15)。頂点は`DrawVertex::oriented_billboard(anchor, [p.x·scale, p.y·scale], heading_rad, color)`(`heading_rad = heading_deg.to_radians()`)。
   三角形は種別ごとに1回だけ`glyph(kind)`の各ポリゴンを`triangulate`して`HashMap<種別,Vec>`にキャッシュ。
6. ラベル: `TrackLabel{ id, selected, position: anchor, name: label, detail: label_detail, color: [base.r,g,b] }`。
   `label_detail = format!("{prefix}{meters:.0} m  {:.0} km/h", speed_mps·3.6)`(`prefix = Msl→"" / AboveGround→"AGL "`、`meters`=高度値)。

### 6.3 シンボルの形 `glyph(kind) -> Vec<Vec<[f64;2]>>`(進行方向が+y・右が+x、全体がおよそ[-1,1])

`mirrored(right_half)`: 右半分の点(機首から時計回りに尾まで)を並べ、その逆順で`x!=0`の点を`-x`に折り返して閉じる(中心線上の点は折り返さない)。
`rect(x0,y0,x1,y1)=[(x0,y0),(x1,y0),(x1,y1),(x0,y1)]`。`bar(half_length, half_width, angle_deg)`: 中心を通り`angle_deg`(時計回り、+yが0°)に伸びる細長い矩形(`[x cos + y sin, -x sin + y cos]`で回転)。

| 種別 | ポリゴン |
|---|---|
| Unknown | `[(0,0.9),(-0.7,0),(0,-0.9),(0.7,0)]`(ひし形) |
| Aircraft | `mirrored([(0,1.0),(0.10,0.62),(0.11,0.18),(0.95,-0.22),(0.95,-0.40),(0.11,-0.14),(0.09,-0.62),(0.40,-0.86),(0.40,-0.98),(0,-0.86)])` |
| Helicopter | 胴体=16角形の楕円`(0.32 cos t, 0.1+0.5 sin t)`, `t=τ i/16` / 尾部=`rect(-0.06,-1.0,0.06,-0.3)` / ローター=`bar(0.95,0.06,45)`と`bar(0.95,0.06,-45)` |
| Ship | `mirrored([(0,1.0),(0.42,0.45),(0.42,-0.85)])` |
| Vehicle | `rect(-0.5,-0.7,0.5,0.55)` と 進行方向の三角形`[(0,1.0),(-0.32,0.6),(0.32,0.6)]` |
| Missile | `mirrored([(0,1.0),(0.11,0.62),(0.11,-0.55),(0.40,-1.0)])` |

### 6.4 当たり判定 `pick_track(anchors, view_proj, viewport_px, point, radius_px) -> Option<TrackId>`

各アンカー(ID, ENU)を`view_proj`でクリップ座標へ。**`clip.w <= 0`(カメラの後ろ)は対象外**。`sx=(clip.x/clip.w+1)/2·W`、`sy=(1-clip.y/clip.w)/2·H`。
クリック位置との距離が`radius_px`以内で最小のもの。**深度は見ない**(地形の陰のシンボルも対象)。アンカーは`ViewState::pick_anchors`(ラベル表示設定に関係なく`rebuild_tracks`が更新)。

### 6.5 検証(必須テスト)

全種別のグリフが三角形分割でき、正の面積で[-1,1]の箱に収まる。機首が+y側で左右対称。シンボルは向きつきビルボード(縁取り+本体)で頂点の`params.x`が進行方向。
高度線は地表から30mを超える機体だけ。航跡は250m以上動いたときだけ伸び、400点で頭打ち。航跡の線は過去→現在をつなぐ。ラベル文字が高度・速度を含む。
選択中は輪とラベル強調。最寄りのシンボルを半径内で選ぶ。日本語ラベル。選択は更新をまたいで生き残り、消えたら解除。
