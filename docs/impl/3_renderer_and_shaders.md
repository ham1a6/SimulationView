# 実装仕様 第3部: wgpuレンダラー・シェーダー

設計書体系([README](../README.md))の詳細設計(ライブラリ実装仕様)第3部で、[IMPLEMENTATION_GUIDE.md](../IMPLEMENTATION_GUIDE.md)の一部。
上位設計(方針・理由・図): [詳細設計書](../DETAILED_DESIGN.md) 6.4節(描画パイプライン)・6.8節(シェーダー・MSAA)・6.10節(水域)・6.11節(描画パス)。数値・手順はこの文書が正。

`terrain::renderer`(`TerrainRenderer`)、`terrain::vertex`、`terrain.wgsl`、`draw.wgsl`。
前提: [第1部](1_data_and_geodesy.md)(`TerrainVertex`)、[第2部](2_lod_camera_los.md)(`Camera`)。

依存: `wgpu = "30"`、`bytemuck`(`Pod`/`Zeroable`)、`glam 0.33`、WGSL検証用に`naga = { version = "30", features = ["wgsl-in"] }`(dev-dependency)。
**wgpuのAPI名は版で変わる**ので、以下の`wgpu`記述はwgpu 30系(`Instance::new(InstanceDescriptor::new_without_display_handle())`、
`SurfaceTarget::Canvas`、`get_current_texture()`が`CurrentSurfaceTexture`列挙を返す、`immediate_size`、`multiview_mask`、`depth_write_enabled: Some(..)`、
`bind_group_layouts: &[Some(..)]`など)を前提にする。

---

## 1. 描画構成の全体像

| パス | 描画先 | 内容 |
|---|---|---|
| ① メイン | 4×MSAAカラー+深度(内部解像度=canvas×2) | 水域 → 地形メッシュ → `World`不透明作図 → 覆域ドーム → `World`半透明作図 → 2D覆域 → マーカー → 航跡 |
| ② オーバーレイ(カメラ固定の作図があるときだけ) | 同じMSAAカラー(`Load`)+深度(`Clear(0.0)`) | 視点空間の不透明 → 視点空間の半透明 → 画面座標(追加順) |
| ③ 縮小 | スワップチェーン(canvas解像度、1サンプル) | 内部解像度の解決結果を線形フィルタで2×2平均して縮小 |

- **MSAA=4**(WebGPU必須は1と4のみ。8は実装依存で`createTexture`が失敗しうる)+**スーパーサンプリング×2**の併用(4×MSAAだけでは浅い角度・遠景の細かい三角形のエイリアシングが残った)。
- 内部テクスチャの一辺は`SUPERSAMPLE_MAX_DIMENSION=4096`で頭打ち(`maxTextureDimension2D`保証最小8192に対する安全側)。
  `supersample_size(w,h) = (min(max(w,1)*2, 4096), min(max(h,1)*2, 4096))`。
- 深度は`Depth32Float`・**反転Z**(比較`Greater`、クリア0.0)。パス①終了時の深度は`Store`、パス②では`Discard`。
- 裏面カリングは**無効**(`cull_mode: None`)。地形は高さ場で裏面はほぼ映らず効果が小さい。作図の面は表裏とも見え、線の帯は向きが一定でない。
  有効化するなら先にスカートの巻き順を4辺で揃える必要がある(既知の技術的負債)。
- 空・データ範囲外(水域にも当たらない画素)は黒 `Color{0,0,0,1}`。

---

## 2. 頂点・uniformのバイトレイアウト

### 2.1 `TerrainVertex`(28バイト、`repr(C)`)

| オフセット | 型 | シェーダー`@location` |
|---|---|---|
| 0 | `[f32;3]` position | 0 `vec3<f32>` |
| 12 | `[f32;3]` color | 1 `vec3<f32>` |
| 24 | `[i16;2]` normal_xy | 2 `vec2<f32>`(`VertexFormat::Snorm16x2`) |

`wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Snorm16x2]`。`array_stride = 28`。

### 2.2 `DrawVertex`(56バイト、`repr(C)`。`terrain::vertex`)

| オフセット | 型 | `@location` | 意味 |
|---|---|---|---|
| 0 | `[f32;3]` position | 0 | 面/線: 座標。ビルボード: アンカーの3D位置 |
| 12 | `[f32;4]` color | 1 | RGBA(非乗算アルファ) |
| 28 | `[f32;3]` aux | 2 | 面: 単位法線(陰影ありのとき)。線: 反対側の端点。ビルボード: `(offset_x_px, offset_y_px, 0)` |
| 40 | `[f32;4]` params | 3 | 下表 |

`vertex_attr_array![0 => Float32x3, 1 => Float32x4, 2 => Float32x3, 3 => Float32x4]`、`array_stride=56`。

`params`: `x`=線の太さ(px。0以下=面。向きつきビルボードでは進行方向のラジアン=北から時計回り)、`y`=線の側(-1/+1)、
`z`=種類(`KIND_FLAT=0.0` 面・線 / `KIND_BILLBOARD=1.0` 画面サイズ固定 / `KIND_ORIENTED_BILLBOARD=2.0` 向きつき)、`w`=陰影を付けるなら1(面のみ)。
シェーダーは`z > 1.5`(向きつき)、`z > 0.5`(ビルボード)で分岐するので、定数がその閾値の正しい側にあること(テストで`draw.wgsl`の文字列を検査している)。

コンストラクタ:
```
surface(position, color, normal: Option<[f32;3]>): Some(n) → aux=n, params=[0,0,FLAT,1];  None → aux=0, params=[0,0,FLAT,0]
line(position, other, color, width_px, side):        aux=other, params=[width_px, side, FLAT, 0]
billboard(anchor, offset_px, color):                 position=anchor, aux=[ox,oy,0], params=[0,0,BILLBOARD,0]
oriented_billboard(anchor, offset_px, heading_rad, color): aux=[ox,oy,0], params=[heading_rad,0,ORIENTED,0]
```

### 2.3 uniform

`CameraUniform`(地形・水域・ドーム。**208バイト**、`@group(0) @binding(0)`、可視性=VERTEX|FRAGMENT):
`view_proj: [[f32;4];4]`(64) / `shading: [f32;4]`(x=陰影ON=1) / `eye, forward, right, up: [f32;4]`(各16。`Camera::water_ray_basis`) / `ellipsoid_m: [[f32;4];3]`(48) / `ellipsoid_g: [f32;4]`(16)。
`initial()`は単位行列・`forward=(0,0,-1,0)`・`right=(1,0,0,0)`・`up=(0,1,0,0)`・他0(バッファ作成用の仮値。毎フレーム`render`が上書き)。

`DrawUniform`(作図系。**96バイト**、可視性=VERTEXのみ): `view_proj: [[f32;4];4]`(64) / `viewport: [f32;4]`(x,y=canvasのpx) / `light: [f32;4]`(xyz=面から光源へ向かう単位ベクトル)。
座標の種類ごとに1つ(`draw_world`/`draw_view`/`draw_screen`)、それぞれ別バッファ・別bind group。

光源定数: `WORLD_DRAW_LIGHT = [-0.5, 0.5, 0.7071068, 0.0]`(北西・仰角45°、地形の陰影と同じ)、`VIEW_DRAW_LIGHT = [-0.348, 0.497, 0.795, 0.0]`(カメラから見て左上手前)。

`screen_matrix(w, h)`(ピクセル座標→クリップ。左上原点・y下向き・深度0.5一定):
`Mat4::from_cols((2/w,0,0,0), (0,-2/h,0,0), (0,0,0,0), (-1,1,0.5,1))`。検証: (0,0)→(-1,1,0.5)、(w,h)→(1,-1)、中心→(0,0)。

---

## 3. パイプライン(6本+縮小1本)

共通: 三角形リスト、`cull_mode: None`、`multisample.count = SAMPLE_COUNT(4)`(縮小のみ1)、カラー形式=スワップチェーン形式(sRGBがあればそれを選ぶ)、深度形式`Depth32Float`。

| 名前 | シェーダー・エントリ | 頂点バッファ | ブレンド | 深度(書く?, 比較) | bind group |
|---|---|---|---|---|---|
| `terrain` | terrain.wgsl `vs_main`/`fs_main` | `TerrainVertex` | REPLACE | (書く, Greater) | camera |
| `water` | terrain.wgsl `vs_fullscreen`/`fs_water` | なし(3頂点を`vertex_index`から生成) | REPLACE | (書く, **Always**) | camera |
| `dome` | terrain.wgsl `vs_main`/`fs_dome` | `TerrainVertex` | ALPHA_BLENDING | (**書かない**, Greater) | camera |
| `draw_opaque` | draw.wgsl `vs_main`/`fs_main` | `DrawVertex` | ALPHA_BLENDING | (書く, Greater) | draw |
| `draw_blend` | 同上 | 同上 | ALPHA_BLENDING | (書かない, Greater) | draw |
| `draw_screen` | 同上 | 同上 | ALPHA_BLENDING | (書かない, **Always**) | draw |
| `downsample`(縮小) | terrain.wgsl `vs_fullscreen`/`fs_downsample` | なし | REPLACE | なし | (texture, sampler) |

- `water`は深度テストしないが**深度を書く**(フラグメントシェーダーが`frag_depth`を出力)。空(楕円体に当たらない画素)は`discard`して黒・深度0のまま。
- `dome`が深度を書かない理由: 半透明の三角形どうしが互いに隠して欠けるのを防ぐ。比較を`Greater`にして、地形より奥なら隠れるようにする。
- bind groupレイアウト: uniformバッファ1つ(`binding 0`、`has_dynamic_offset: false`)。縮小のbind groupは`binding 0`=`texture_2d<f32>`(`filterable: true`, 非マルチサンプル)、`binding 1`=`Sampler(Filtering)`。
- 縮小サンプラー: `ClampToEdge`×3、`mag_filter`/`min_filter` = `Linear`、`mipmap_filter=Nearest`。**wgpu標準の`TextureBlitter`は縮小側がNearest固定なので使わない**(線形フィルタの2×2平均がスーパーサンプリングの要)。
- パイプラインレイアウトは`immediate_size: 0`。

---

## 4. `TerrainRenderer` の公開API

```rust
pub struct TerrainRenderer { /* surface, device, queue, config, pipelines, meshes: HashMap<MeshKey, MeshGpu>,
                               camera_buffer/bind_group, targets, downsample, 各VertexBatch, dome_vertex_buffer, hillshade, ellipsoid */ }
struct MeshGpu { vertex_buffer, index_buffer, num_indices: u32, bounds: ([f32;3],[f32;3]) }   // boundsは頂点位置のAABB(視錐台カリング用)

impl TerrainRenderer {
    pub async fn new(canvas: web_sys::HtmlCanvasElement) -> Result<Self, String>;
    pub fn set_mesh(&mut self, key: MeshKey, mesh: &TerrainMesh);           // 同じキーは置換。indices.is_empty()なら該当メッシュを削除
    pub fn remove_mesh(&mut self, key: MeshKey);
    pub fn update_mesh_vertices(&mut self, key: MeshKey, vertices: &[TerrainVertex]);   // 頂点数不変で位置だけ更新(原点変更)。boundsも更新
    pub fn update_markers(&mut self, v: &[DrawVertex]);                     // 各VertexBatch。空ならバッファを持たない
    pub fn update_coverage_2d(&mut self, v: &[DrawVertex]);
    pub fn update_tracks(&mut self, v: &[DrawVertex]);
    pub fn update_drawings(&mut self, batches: &DrawingBatches);            // world.opaque/blend, view.opaque/blend, screen
    pub fn update_dome(&mut self, v: &[TerrainVertex]);                     // 空なら解放
    pub fn set_hillshade(&mut self, enabled: bool);
    pub fn set_ellipsoid_origin(&mut self, t: &EnuTransform);               // 水域用の楕円体係数。頂点を作り直す場面(初期化・原点変更)で必ず呼ぶ
    pub fn resize(&mut self, w: u32, h: u32);                               // w/hが0または現状と同じなら何もしない。surface再設定+RenderTargets再作成+Downsample::rebind
    pub fn aspect_ratio(&self) -> f32;  pub fn canvas_height_px(&self) -> u32;  pub fn canvas_size_px(&self) -> (u32,u32);
    pub fn render(&self, camera: &Camera) -> Result<(), String>;
}
```

### 4.1 `new`

1. `canvas.width()/height()`(0なら1)。`Instance::new(new_without_display_handle())`、`create_surface(SurfaceTarget::Canvas(canvas))`。**ネイティブ(単体テスト)ではcanvas surfaceが作れない**ので`cfg(target_arch="wasm32")`で分け、非wasmは`Err`を返す(ライブラリ全体が`cargo test`でビルドできるように)。
2. `request_adapter{ HighPerformance, compatible_surface: Some(&surface), force_fallback_adapter: false }`、`request_device(default)`。
3. `surface.get_default_config(&adapter, w, h)`。**sRGB形式があればそれに変更**、`usage = RENDER_ATTACHMENT`、`present_mode = Fifo`、`surface.configure`。
4. シェーダーモジュール(terrain.wgsl)を作り、`RenderTargets::new`・`Downsample::new`・camera uniformバッファ(`UNIFORM|COPY_DST`、`initial()`)・bind group・`Pipelines::new`・`DrawSpace`×3を作る。`hillshade`初期値は`false`(`set_hillshade`で反映)。

### 4.2 `set_mesh`

頂点バッファ`VERTEX|COPY_DST`(`COPY_DST`は`update_mesh_vertices`用)、インデックスバッファ`INDEX`(**`Uint32`**)。`bounds = position_bounds(vertices)`(min/maxの各成分。頂点が無ければmin=+∞,max=-∞の空の箱)。

### 4.3 `render(camera)`

1. `[eye,forward,right,up] = camera.water_ray_basis()`。`CameraUniform{ view_proj: camera.view_proj_matrix(), shading:[hillshade?1:0,0,0,0], eye,forward,right,up, ellipsoid_m, ellipsoid_g }`を`queue.write_buffer`。
2. 作図のuniform 3種を書く: `viewport=[w,h,0,0]`(canvasの`config.width/height`)、`world`=`(camera.view_proj_matrix(), WORLD_DRAW_LIGHT)`、`view`=`(camera.projection_matrix(), VIEW_DRAW_LIGHT)`(**ビュー行列なし**)、`screen`=`(screen_matrix(w,h), [0;4])`。
3. `has_overlay = !(view_opaque.is_empty() && view_blend.is_empty() && screen_batch.is_empty())`。
4. `surface.get_current_texture()`の結果:
   `Success(t)`→描画。`Suboptimal(t)`→描画し、submit後に`surface.configure`。`Timeout|Occluded`→**このフレームは描かず`Ok(())`**(エラーではない。タブが隠れている等)。
   `Outdated`→`surface.configure`して`Ok(())`(次のフレームで復帰)。その他は`Err`。
5. コマンドエンコーダで ①メイン → (`has_overlay`なら)②オーバーレイ → ③縮小 を積み、`queue.submit`、`queue.present(frame)`。

### 4.4 パス①メイン

- カラー: `view=msaa_view`、`resolve_target = has_overlay ? None : Some(supersample_color_view)`、`load=Clear(黒)`、`store = has_overlay ? Store : Discard`。
- 深度: `depth_view`、`Clear(0.0)`、`Store`。
- 描画順: (1) `water`を`draw(0..3)` → (2) `terrain`: 全メッシュを**視錐台カリング**して`draw_indexed`(Uint32) → (3) `world_opaque`(`draw_opaque`+`draw_world`)
  → (4) `dome`(あれば) → (5) `world_blend`(`draw_blend`+`draw_world`) → (6) `coverage_2d`(`draw_screen`+`draw_world`。**深度テストなし**) → (7) `markers`(`draw_blend`+`draw_world`) → (8) `tracks`(同)。
- `VertexBatch::draw(pass, pipeline, space)`: バッファが空なら何もしない。`set_pipeline`→`set_bind_group(0)`→`set_vertex_buffer(0)`→`draw(0..count)`。

### 4.5 パス②オーバーレイ

カラー: `msaa_view`を`Load`、`resolve_target=Some(supersample_color_view)`、`store=Discard`。深度: `Clear(0.0)`・`Discard`(地形と隠し合わない)。
描画順: `view_opaque`(`draw_opaque`+`draw_view`)→`view_blend`(`draw_blend`+`draw_view`)→`screen_batch`(`draw_screen`+`draw_screen`)。

### 4.6 パス③縮小

カラー: スワップチェーンのview、`Clear(黒)`、`Store`、深度なし。`Downsample::draw`: `set_pipeline`→`set_bind_group(0)`→`draw(0..3)`。

### 4.7 視錐台カリング

`is_outside_frustum(view_proj, (min,max)) -> bool`: AABBの8つの角(`corner`のビット0/1/2でmin/max)をクリップ座標`c=view_proj*(p,1)`へ。
各角について6面のビットを立てる: `x<-w`(bit0)、`x>w`(bit1)、`y<-w`(bit2)、`y>w`(bit3)、`z<0`(bit4)、`z>w`(bit5)。
**全角のビットの論理積**が非0ならその面の完全に外(=描かない)。途中で論理積が0になったら`false`で打ち切り。
保守的判定(外にあるのに「外でない」とすることはあっても、見えているものを「外」とすることはない)。

検証: 明らかに外の箱は`true`、見えている箱は決して`true`にならない(ランダム/格子点で網羅)。

---

## 5. シェーダー

以下のWGSLは`sim3dview/src/terrain/*.wgsl`と**逐語一致**させる(`scripts/sync_impl_wgsl.py --check`が差分を検出する)。
実装時はこれをそのまま`terrain.wgsl`/`draw.wgsl`として保存してよい。設計意図は各コメントとその下の解説を参照。

### 5.1 `terrain.wgsl`

<!-- BEGIN-WGSL sim3dview/src/terrain/terrain.wgsl -->
```wgsl
struct CameraUniform {
    view_proj: mat4x4<f32>,
    // x: 陰影(ヒルシェード)を付けるなら1、付けないなら0。y,z,wは未使用。
    shading: vec4<f32>,
    // 水域レイヤー(fs_water)が各画素の視線を求めるための値(`Camera::water_ray_basis`):
    // eye.xyz=視点、eye.w=1なら透視投影・0なら正射影。forward=視線方向。right/up=画面端までの長さ倍。
    eye: vec4<f32>,
    forward: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    // WGS84楕円体(海抜0m)の陰関数の係数(`EnuTransform::ellipsoid_shader_params`)。
    // f(p) = ellipsoid_g.w + 2 g・(M p) + |M p|^2。M=ellipsoid_m(3行)、g=ellipsoid_g.xyz。
    ellipsoid_m: array<vec4<f32>, 3>,
    ellipsoid_g: vec4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    // 単位法線のx(East)・y(North)成分(snorm16x2)。z(Up)は復元する。長さが1を超える値は
    // 「陰影を付けない」印(`TerrainVertex::UNLIT_NORMAL`。マーカー・覆域ドームなど)。
    @location(2) normal_xy: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

// 陰影(ヒルシェード)の光源。ENU座標(東,北,上)で、北西から仰角45度の固定(地図の陰影の
// 一般的な向き)。カメラの向きに依らず、地形の見え方が変わらない。単位ベクトル。
const LIGHT_DIR = vec3<f32>(-0.5, 0.5, 0.70710678);
// 光が当たらない面(光源の反対側の斜面)の明るさの下限(0〜1)。
const AMBIENT = 0.35;

// 頂点の色に掛ける明るさ。平地(法線が真上)は1(色が変わらない)、光源側に向いた斜面は1より
// 明るく、反対側の斜面は暗くなる。
fn hillshade(normal_xy: vec2<f32>) -> f32 {
    let xy2 = dot(normal_xy, normal_xy);
    if (xy2 > 1.0) {
        return 1.0;
    }
    let normal = vec3<f32>(normal_xy, sqrt(1.0 - xy2));
    let lambert = max(dot(normal, LIGHT_DIR), 0.0);
    let flat_level = AMBIENT + (1.0 - AMBIENT) * LIGHT_DIR.z;
    return (AMBIENT + (1.0 - AMBIENT) * lambert) / flat_level;
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    // 陰影は頂点ごとに求めて色に掛け、面の内側は補間する(明るさは法線について線形なので、
    // 法線を補間してからフラグメントごとに求めるのとほぼ同じ結果になる)。
    out.color = in.color * mix(1.0, hillshade(in.normal_xy), camera.shading.x);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}

// 見通し範囲の覆域ドーム(半球状の面)用。地形やマーカーを透けて見せたいため、
// 固定の半透明アルファで出力する(頂点データ自体は他のパイプラインと共用のposition+colorのまま)。
@fragment
fn fs_dome(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 0.22);
}

// 水域レイヤー。「地形データが無いところ(マスクファイルの水域・欠損タイル・地形データの範囲外)」は
// 地形メッシュが張られないので、そこから見えるのは地形の後ろの背景になる。そこにWGS84楕円体の
// 海抜0mの面を水色で描く。メッシュではなく画面いっぱいの三角形1枚で、各画素の視線と楕円体の
// 交点があるかをシェーダーで直接判定する(交点なし=空は描かずclearの黒のまま)。
// - 地球の丸み・楕円体を厳密に扱える(メッシュ近似の弦の誤差・継ぎ目・範囲の限りがない)
// - 地形メッシュより先に描く。**交点の深度を書く**ので、楕円体(地球本体)の向こう側にある地形は
//   隠れる(地球の丸みの向こう側・海面の下・海岸の張り出した縁の下から見える地形の裏側が、
//   水面越しに透けて見えない)。ただし深度は交点より視線方向に`WATER_DEPTH_MARGIN_M`奥へずらして
//   書く: 地形は交点から`WATER_DEPTH_MARGIN_M`以内の奥までは水域より手前として描かれ、
//   標高が0m以下(DSMの負の値・海面以下の陸地)の地形が水域に隠れることはない
const WATER_COLOR = vec3<f32>(0.25, 0.55, 0.85);
// 水域の深度を、視線の交点から奥へずらす距離(メートル、視線に沿って)。地形が0m以下でも隠れない
// ための余裕。標高-Hの地形が視線と角度thetaで交わるとき、交点との視線方向の距離はH/sin(theta)。
// 標高-10mでもtheta>=約0.6度(ほぼ水平の見え方)まで隠れない。一方、地球の丸みの向こう側の地形は
// 水平線からの距離が数km以上なので、これより大きく奥になり隠れる。
const WATER_DEPTH_MARGIN_M = 1000.0;

struct WaterOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_water(in: DownsampleOutput) -> WaterOutput {
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let offset = camera.right.xyz * ndc.x + camera.up.xyz * ndc.y;
    let perspective = camera.eye.w;
    let origin = camera.eye.xyz + offset * (1.0 - perspective);
    let dir = camera.forward.xyz + offset * perspective;

    let m0 = camera.ellipsoid_m[0].xyz;
    let m1 = camera.ellipsoid_m[1].xyz;
    let m2 = camera.ellipsoid_m[2].xyz;
    let g = camera.ellipsoid_g.xyz;
    let q0 = vec3<f32>(dot(m0, origin), dot(m1, origin), dot(m2, origin));
    let w = vec3<f32>(dot(m0, dir), dot(m1, dir), dot(m2, dir));

    // f(origin + t*dir) = a t^2 + 2 half_b t + c = 0 の解(視線と楕円体の交点)。
    let a = dot(w, w);
    let half_b = dot(g, w) + dot(q0, w);
    let c = camera.ellipsoid_g.w + 2.0 * dot(g, q0) + dot(q0, q0);
    let disc = half_b * half_b - a * c;
    if (disc < 0.0) {
        discard;
    }
    // 桁落ちしにくい解の公式。視点は楕円体の外にあるので、2つの解は同符号で、
    // 正なら視線の前方で交わる(負なら後ろ向き=空)。手前の交点(小さい方の解)が水面。
    let s = sqrt(disc);
    let q = -(half_b + select(-s, s, half_b >= 0.0));
    let t1 = q / a;
    let t2 = c / q;
    let t_near = min(t1, t2);
    if (!(max(t1, t2) > 0.0) || !(t_near > 0.0)) {
        discard;
    }

    // 交点から視線に沿って奥へ`WATER_DEPTH_MARGIN_M`ずらした点の深度(反転Z: 0=遠い、1=近い)。
    let hit = origin + dir * (t_near + WATER_DEPTH_MARGIN_M / length(dir));
    let clip = camera.view_proj * vec4<f32>(hit, 1.0);
    var out: WaterOutput;
    out.color = vec4<f32>(WATER_COLOR, 1.0);
    out.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    return out;
}

// スーパーサンプリングのダウンサンプル用(renderer.rsのdownsample_pipeline)。地形メッシュを
// 内部解像度(画面の`SUPERSAMPLE_FACTOR`倍、4倍MSAA込み)で描いた後、このシェーダーで画面
// いっぱいの三角形を1枚描いて線形フィルタでサンプリングし、実際のcanvas解像度へ縮小する。
// 4倍MSAAだけでは、2048×2048化後の遠景・浅い角度で細かい陸地の三角形による
// エイリアシング(斑点・ちらつき)を抑えきれなかったため導入した
// (「背景と同じ色の点が多数表示される/ズーム操作やカメラ操作時に画面がちかちかする」との
// 報告を受けて対処)。頂点バッファを使わない「画面いっぱいの三角形」の定石
// (vertex_indexだけから3頂点を計算し、クリップ領域外にはみ出す部分はラスタライザが
// 自動的に切り捨てる)を使っている。
struct DownsampleOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> DownsampleOutput {
    var out: DownsampleOutput;
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    out.clip_position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

@group(0) @binding(0)
var supersample_texture: texture_2d<f32>;
@group(0) @binding(1)
var supersample_sampler: sampler;

@fragment
fn fs_downsample(in: DownsampleOutput) -> @location(0) vec4<f32> {
    return textureSample(supersample_texture, supersample_sampler, in.uv);
}
```
<!-- END-WGSL -->

解説:
- **陰影(hillshade)**: 光源`LIGHT_DIR=(-0.5,0.5,0.7071)`(ENU、北西・仰角45°)、`AMBIENT=0.35`。`(AMBIENT+(1-AMBIENT)·max(n·L,0)) / (AMBIENT+(1-AMBIENT)·L.z)`。
  分母で正規化して**平地は1**(ON/OFFで平地の色が変わらない)。光源側斜面は最大約1.25倍、反対側は約0.43倍。頂点ごとに求めて補間(明るさは法線について線形なので実用上同等)。
  `normal_xy`の長さの二乗が1を超えたら陰影なし(`UNLIT_NORMAL`)。ON/OFFはuniform`shading.x`の値だけで切り替わりメッシュ再作成は不要。
- **水域(`fs_water`)**: 「地形メッシュが張られない所」の背景にWGS84楕円体の海抜0m面を水色`(0.25,0.55,0.85)`で描く。メッシュ近似だと弦の凹みで継ぎ目・範囲の限りが出るため、
  画面いっぱいの三角形1枚で各画素の視線と楕円体の交点を解析的に求める。透視は`origin=eye, dir=forward+offset`、正射影は`origin=eye+offset, dir=forward`(`offset = right*ndc.x + up*ndc.y`、`perspective`が0/1)。
  二次方程式`a t²+2·half_b·t+c=0`を桁落ちしにくい解の公式で解き、手前の解`t_near>0`が水面。
  **書く深度は交点から視線に沿って`WATER_DEPTH_MARGIN_M=1000m`奥へずらした点の値**: 楕円体(地球本体)を不透明物体として扱い地球の丸みの向こう側・海面の下の地形が水面越しに透けるのを防ぎつつ、
  標高0m以下(DSMの負値・海面以下の陸地)の地形が水域に隠れないための余裕(標高-Hの地形と視線が角度θで交わるとき交点との視線方向距離はH/sinθ。-10mでもθ≧約0.6°まで隠れない)。
  深度はクリップ`z/w`を[0,1]にクランプ。
- **縮小(`fs_downsample`)**: 内部解像度のテクスチャを線形サンプルして縮小。頂点シェーダー`vs_fullscreen`は`vertex_index`(0,1,2)から画面いっぱいの三角形(`x=(i<<1)&2`, `y=i&2`、`clip=(x*2-1, 1-y*2)`, `uv=(x,y)`)を作る。
- 同じ`vs_main`を`fs_main`(不透明)と`fs_dome`(固定アルファ0.22)で共用する。
- **水域のbind groupと縮小のbind groupはどちらも`@group(0)`**(別のパイプラインで別のレイアウトを使う。`@binding`の意味がパイプラインごとに違う点に注意)。

### 5.2 `draw.wgsl`

<!-- BEGIN-WGSL sim3dview/src/terrain/draw.wgsl -->
```wgsl
// 作図機能(`terrain::drawing`)の描画シェーダー。面(塗り・立体)と太い線を同じ頂点形式で描く。
// 頂点データは`drawing_geometry::DrawVertex`。座標の種類(絶対座標・視点空間・画面)ごとに
// `view_proj`を差し替えて、同じシェーダーで描く。

struct DrawUniform {
    view_proj: mat4x4<f32>,
    // x,y: 描画先(canvas)の大きさ(px)。線の太さ(px)をクリップ空間へ換算するのに使う。
    viewport: vec4<f32>,
    // xyz: 光源の向き(面から光源へ向かう単位ベクトル。頂点の座標系で)。
    light: vec4<f32>,
};
@group(0) @binding(0)
var<uniform> u: DrawUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    // 面: 法線(陰影を付けるとき)。線: 反対側の端点。
    @location(2) aux: vec3<f32>,
    // x: 線の太さ(px。0以下なら面。向きつきビルボードでは進行方向(ラジアン、北から時計回り))、
    // y: 線の側(-1/+1)、z: 1ならビルボード(画面サイズ固定のマーカー)・2なら向きつきビルボード
    // (シンボル。進行方向が画面のどちらを向くかに合わせて回す)、w: 陰影を付けるなら1(面のみ)。
    @location(3) params: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

// 光が当たらない面の明るさの下限(0〜1)。
const AMBIENT = 0.4;
// 線の端点がカメラの後ろ側(クリップ空間のwが小さい)にあるとき、線分をこのwで打ち切る。
// (透視投影では、視点の後ろの点を画面へ射影すると向きが反転して、線の帯が壊れるため)
const LINE_MIN_W = 0.5;
// 線を、同じ位置の面(塗りや立体の表面)よりわずかに手前として深度テストするための、
// クリップ空間のzの相対的な加算量(反転Zなので大きいほど手前)。線が面と同じ深度になって
// 縞模様(Zファイティング)になるのを避ける。
const LINE_DEPTH_BIAS = 2.0e-5;
// ビルボード(マーカー)を手前へ寄せる、クリップ空間のzの相対的な加算量。マーカーは地表の1点に立てるので、
// 粗いLODの地形メッシュとの高さのずれ(遠いほど大きい)で地面に埋まって消えないよう、距離に比例して
// 大きめに寄せる(距離の0.2%: 400km先で約800m、5km先で10m)。これより手前の山には隠れる。
const BILLBOARD_DEPTH_BIAS = 2.0e-3;
// 向きつきビルボードで、進行方向の画面上の向きを求めるためにアンカーから進む距離(メートル)。
const ORIENT_STEP_M = 200.0;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    if (in.params.z > 1.5) {
        // 向きつきビルボード: aux.xyは「進行方向が上・その右が+x」の画面のpx。進行方向の画面上の向きは、
        // アンカーと、アンカーから進行方向(ENUの水平、北から時計回りparams.x)へ少し進んだ点の射影の差で求める。
        out.color = in.color;
        let c = u.view_proj * vec4<f32>(in.position, 1.0);
        let step = vec3<f32>(sin(in.params.x), cos(in.params.x), 0.0) * ORIENT_STEP_M;
        let c1 = u.view_proj * vec4<f32>(in.position + step, 1.0);
        let half = u.viewport.xy * 0.5;
        var forward = (c1.xy / c1.w - c.xy / c.w) * half;
        let len = length(forward);
        if (len > 1.0e-4) {
            forward = forward / len;
        } else {
            forward = vec2<f32>(0.0, 1.0); // 真上・真下から見るなど、向きが画面に現れないとき。
        }
        let right = vec2<f32>(forward.y, -forward.x);
        let offset_px = right * in.aux.x + forward * in.aux.y;
        let ndc = c.xy / c.w + offset_px / half;
        out.clip_position = vec4<f32>(ndc * c.w, min(c.z * (1.0 + BILLBOARD_DEPTH_BIAS), c.w), c.w);
        return out;
    }
    if (in.params.z > 0.5) {
        // ビルボード: position(アンカーの位置)を射影し、aux.xy(画面のpx、右・上が正)だけずらす。
        // 大きさが拡大・縮小・回転で変わらず、常に画面の正面を向く。
        out.color = in.color;
        let c = u.view_proj * vec4<f32>(in.position, 1.0);
        let ndc = c.xy / c.w + in.aux.xy / (u.viewport.xy * 0.5);
        out.clip_position = vec4<f32>(ndc * c.w, min(c.z * (1.0 + BILLBOARD_DEPTH_BIAS), c.w), c.w);
        return out;
    }
    let width_px = in.params.x;
    if (width_px <= 0.0) {
        out.clip_position = u.view_proj * vec4<f32>(in.position, 1.0);
        var shade = 1.0;
        if (in.params.w > 0.5) {
            shade = AMBIENT + (1.0 - AMBIENT) * max(dot(normalize(in.aux), u.light.xyz), 0.0);
        }
        out.color = vec4<f32>(in.color.rgb * shade, in.color.a);
        return out;
    }

    // 線: この頂点は線分の端点(position)にあり、反対側の端点(aux)との向きで帯を広げる。
    out.color = in.color;
    let this_clip = u.view_proj * vec4<f32>(in.position, 1.0);
    let other_clip = u.view_proj * vec4<f32>(in.aux, 1.0);
    var a = this_clip;
    var b = other_clip;
    if (a.w < LINE_MIN_W) {
        if (b.w < LINE_MIN_W) {
            // 両端ともカメラの後ろ: 何も描かない(クリップ空間の外に出す)。
            out.clip_position = vec4<f32>(0.0, 0.0, -1.0, 1.0);
            return out;
        }
        a = mix(this_clip, other_clip, (LINE_MIN_W - this_clip.w) / (other_clip.w - this_clip.w));
    } else if (b.w < LINE_MIN_W) {
        b = mix(other_clip, this_clip, (LINE_MIN_W - other_clip.w) / (this_clip.w - other_clip.w));
    }

    // 画面のピクセル単位で、線の向き(反対側→この端点)と、その左手の法線を求める。
    let half = u.viewport.xy * 0.5;
    let pa = a.xy / a.w * half;
    let pb = b.xy / b.w * half;
    var dir = pa - pb;
    let len = length(dir);
    if (len > 1.0e-4) {
        dir = dir / len;
    } else {
        dir = vec2<f32>(1.0, 0.0);
    }
    let normal = vec2<f32>(-dir.y, dir.x);
    let half_width = 0.5 * width_px;
    // 幅方向に±半幅、端点は線の向きへ半幅だけ延ばす(折れ線のつなぎ目の隙間を埋める)。
    let offset_px = normal * (in.params.y * half_width) + dir * half_width;
    let ndc = a.xy / a.w + offset_px / half;
    let z = min(a.z * (1.0 + LINE_DEPTH_BIAS), a.w);
    out.clip_position = vec4<f32>(ndc * a.w, z, a.w);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
```
<!-- END-WGSL -->

解説:
- **太い線**: WebGPUの線プリミティブは1pxしかないので、線分1本を四角形(三角形2枚)にして頂点シェーダーで**画面のpx幅**へ広げる。
  各頂点は「この端点`position`」と「反対側の端点`aux`」を持つ。クリップ座標→ピクセル座標で向き(反対側→この端点)と左手の法線を求め、`params.y`(±1)×半幅だけ左右へ、端点は線の向きへ半幅延ばす(折れ線のつなぎ目の隙間を埋める)。
  頂点生成側(`append_line_strip`)は線分(a→b)ごとに4頂点`a+, a-, b+(side=-1), b-(side=+1)`を作り、三角形`[a+, a-, b+,  b+, a-, b-]`にする(端の側で符号が逆なのは向きを「反対側→この端点」で取るため。物理的には同じ側に揃う)。
  `w < LINE_MIN_W(0.5)`の端点は線分をそこで打ち切る(透視で視点の後ろの点を射影すると向きが反転して帯が壊れる)。両端とも後ろなら何も描かない。
  `LINE_DEPTH_BIAS`だけ手前に寄せて面とのZファイティングを避ける。半透明の線は折れ線のつなぎ目でアルファが二重に掛かって濃く見える(不透明なら出ない。既知)。
- **ビルボード**: アンカーを射影し、`aux.xy`(px、右・上が正)だけずらす → 拡大縮小・回転で大きさが変わらず常に正面を向く。深度は`c.z*(1+BILLBOARD_DEPTH_BIAS)`(距離の0.2%相当を手前へ)で、粗いLODの地形に埋まらないようにする(遠くの山の陰には隠れる)。
- **向きつきビルボード**: アンカーと「アンカーからENUの水平方向`(sin h, cos h)`(h=北から時計回りの進行方向)へ200m進んだ点」を射影し、その差の画面上の向きを`forward`とする。
  `right = (forward.y, -forward.x)`、`offset = right*aux.x + forward*aux.y`。差が(ほぼ)0(真上・真下から見て向きが現れない)なら`forward=(0,1)`(画面の上)。
  3Dでカメラを回しても、2Dの地図でも、シンボルが実際の進行方向を指す。
- 面の陰影: `params.w>0.5`なら`AMBIENT(0.4)+(1-0.4)·max(dot(normalize(aux), light.xyz), 0)`をRGBに掛ける(アルファはそのまま)。

---

## 6. 描画まわりの検証(必須テスト)

`cargo test`(ネイティブ)で動く。GPUは使わない。

1. **WGSL検証**(`naga`): `terrain.wgsl`と`draw.wgsl`がパースでき、`naga::valid::Validator`(全capability)で検証を通る。
2. **`CameraUniform`のレイアウト**: `size_of == 208`、フィールドのオフセットがWGSL構造体(`view_proj`0, `shading`64, `eye`80, `forward`96, `right`112, `up`128, `ellipsoid_m`144, `ellipsoid_g`192)と一致。
   nagaでWGSLの型サイズ(`TypeInner::Struct`のspan・メンバoffset)を読んで比較する。
3. **`DrawUniform`**: `size_of==96`、`view_proj`0/`viewport`64/`light`80。
4. **頂点レイアウト**: `TerrainVertex`の`offset_of`(0,12,24)と`size_of==28`が`TERRAIN_VERTEX_ATTRIBUTES`(Float32x3,Float32x3,Snorm16x2)のオフセット・形式と一致し、WGSL`VertexInput`の`@location`(0,1,2)・型と一致。`DrawVertex`も同様(0,12,28,40、size56)。
5. `supersample_size`(700,500)→(1400,1000)、(0,0)→(2,2)、(3000,100)→(4096,200)。`position_bounds`、`screen_matrix`、視錐台(4.7)。
