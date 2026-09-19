//! 地形LOD(解像度レベル)の計画。「どのタイルを・どの解像度レベルで描くか」を、カメラとの距離・
//! 視錐台・頂点数の予算から決める純粋関数(GPUやネットワークには触れない)。実際のグリッド取得と
//! メッシュ差し替えは`ui::terrain_view`が行う。
//!
//! 構成:
//! - 全タイルは、離れているうちは**タイル全体を1枚のメッシュ(レベル0、約1.85km/セル)**で描く
//!   (`loader::load_terrain`が起動時に全部取得して常駐する)。
//! - カメラに近づいて、レベル0では粗すぎる(画面上で1セルが`TARGET_CELL_PX`ピクセルを超える)
//!   タイルは、**1度タイルを分割した6x6のチャンク**に切り替え、チャンクごとに独立して解像度
//!   レベル(1〜最細=元データの30m)を選ぶ。カメラのすぐ近くのチャンクだけが最細になる。
//! - チャンクの頂点の合計は`DETAIL_VERTEX_BUDGET`以下に抑える(GPUメモリ・描画負荷の上限)。
//!   予算は「見えていて近いチャンク」から順に配る。
//! - 視野の外は、予算が余っている間は現状のレベルを保つ(カメラを戻したときにすぐ細かく見える
//!   ように)が、予算は見えているものより後回しにする。
//! - 理想より1レベル細かいだけなら下げない(距離の境目でレベルが行き来しないヒステリシス)。

use std::collections::HashMap;

use glam::{Mat4, Vec3};

use super::camera::{Camera, Projection};
use super::loader::{TerrainData, TileKey};
use super::mesh::{tile_vertex_count, EnuTransform};

/// チャンクの頂点数の合計の上限。全タイル分のレベル0(約165万頂点)に、この分を足したものが
/// 常駐する頂点の総数になる(従来の単一メッシュは約420万頂点)。最細(30m)のチャンクは
/// 1個で約36万頂点なので、最細のチャンクは同時に8個程度まで。
pub const DETAIL_VERTEX_BUDGET: usize = 3_000_000;

/// 画面(CSSピクセル)上で1セルがこのピクセル数以下になる最も粗いレベルを選ぶ。
/// 描画は2倍スーパーサンプリングなので、2ピクセルなら描画解像度では約4ピクセル分。
const TARGET_CELL_PX: f32 = 2.0;

/// タイル全体(レベル0)からチャンクへ切り替える/戻すときのヒステリシス。チャンクにしている
/// タイルは、レベル0で足りるとみなせる距離の`REVERT_MARGIN`倍まで近づけないと全体表示へ戻さない。
const REVERT_MARGIN: f32 = 0.7;

/// 緯度1度の長さ(メートル)。セルの大きさの見積もりに使う。
const METERS_PER_DEGREE: f32 = 111_000.0;

/// 視錐台の左右・上下の判定の余裕(クリップ座標の外側へこの倍率まで許す)。
const FRUSTUM_MARGIN: f32 = 1.2;

/// いまGPUにあるタイルの状態(`ui::terrain_view`が保持する)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resident {
    /// タイル全体をレベル0の1枚のメッシュで出している。
    Whole,
    /// チャンクごとのメッシュで出している。中身はチャンク(行(南→北)*分割数+列(西→東))ごとの
    /// 現在のレベル(1以上)。
    Chunks(Vec<u8>),
}

/// 計画: タイルをどう描くか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TilePlan {
    Whole,
    /// チャンクごとの目標レベル(1以上)。
    Chunks(Vec<u8>),
}

/// レベルkのセルの大きさ(メートル、緯度方向)。
pub fn cell_size_m(data: &TerrainData, level: usize) -> f32 {
    METERS_PER_DEGREE / data.level_cells(level) as f32
}

/// チャンク1個(レベルk、1以上)がGPUに載せる頂点数。
pub fn chunk_vertex_cost(data: &TerrainData, level: usize) -> usize {
    tile_vertex_count(data.chunk_cells(level))
}

/// カメラまわりの計算をまとめたもの(タイルごと・チャンクごとに何度も使う)。
struct ViewInfo<'a> {
    camera: &'a Camera,
    view_proj: Mat4,
    transform: &'a EnuTransform,
    canvas_h: f32,
    /// 「一番近い点」を求める基準の位置(緯度経度)。透視投影では視点の真下、正射影(2D)では
    /// 注視点(視点は真上にあるだけで見ている場所ではない)。
    focus: Vec3,
    focus_lat: f64,
    focus_lon: f64,
    tan_half: f32,
    ortho_pixel_m: f32,
}

impl ViewInfo<'_> {
    /// 緯度経度の矩形について、基準位置から見て最も近い点までの距離と、視野に入るか。
    fn rect(&self, lat0: f64, lon0: f64, lat1: f64, lon1: f64, mid_h: f64) -> (f32, bool) {
        let nearest = Vec3::from(self.transform.transform(
            self.focus_lat.clamp(lat0, lat1),
            self.focus_lon.clamp(lon0, lon1),
            mid_h,
        ));
        let distance = match self.camera.projection {
            Projection::Perspective { .. } => (nearest - self.camera.eye).length(),
            Projection::Orthographic { .. } => {
                ((nearest.x - self.focus.x).powi(2) + (nearest.y - self.focus.y).powi(2)).sqrt()
            }
        };

        // 視錐台の判定: 4隅と中心のうち、どれも視野に入らない(全部が同じ側の外)なら見えない。
        let points = [
            (lat0, lon0),
            (lat0, lon1),
            (lat1, lon0),
            (lat1, lon1),
            (0.5 * (lat0 + lat1), 0.5 * (lon0 + lon1)),
        ];
        // 各面(左・右・下・上)ごとに「全点が外側」か、全点が背後か。
        let mut outside = [true; 4];
        let mut all_behind = true;
        for (lat, lon) in points {
            let p = Vec3::from(self.transform.transform(lat, lon, mid_h));
            let clip = self.view_proj * p.extend(1.0);
            if clip.w > 0.0 {
                all_behind = false;
                let m = clip.w * FRUSTUM_MARGIN;
                outside[0] &= clip.x < -m;
                outside[1] &= clip.x > m;
                outside[2] &= clip.y < -m;
                outside[3] &= clip.y > m;
            } else {
                // 背後の点はどの面の判定にも使えない(他の点と組み合わせて面をまたぐ場合に備えて、
                // 外側扱いにはしない)。
                outside = [false; 4];
            }
        }
        (distance, !all_behind && !outside.iter().any(|&o| o))
    }

    /// 距離distanceの地点での、画面1ピクセルが表す実寸(メートル)。
    fn pixel_m(&self, distance: f32) -> f32 {
        match self.camera.projection {
            Projection::Perspective { .. } => {
                2.0 * distance.max(50.0) * self.tan_half / self.canvas_h
            }
            Projection::Orthographic { .. } => self.ortho_pixel_m,
        }
    }
}

/// 1セルが画面上で`TARGET_CELL_PX`以下になる最も粗いレベル(`from`以上)。どれも満たさなければ最細。
fn ideal_level(data: &TerrainData, pixel_m: f32, from: usize) -> usize {
    let max_level = data.num_levels() - 1;
    (from..=max_level)
        .find(|&k| cell_size_m(data, k) <= TARGET_CELL_PX * pixel_m)
        .unwrap_or(max_level)
}

/// 各タイルの描き方(全体1枚か、チャンクごとのレベルか)を決める。`resident`は現在GPUにある状態
/// (無ければ`Whole`扱い)。戻り値は存在するタイルすべての計画で、優先度の高い順(見えていて近い
/// タイルが先頭)に並ぶ。適用側はこの順に処理すると、近いところから先に細かくなる。
pub fn plan_levels(
    data: &TerrainData,
    transform: &EnuTransform,
    camera: &Camera,
    canvas_height_px: f32,
    resident: &HashMap<TileKey, Resident>,
) -> Vec<(TileKey, TilePlan)> {
    let canvas_h = canvas_height_px.max(1.0);
    let (focus, tan_half, ortho_pixel_m) = match camera.projection {
        Projection::Perspective { fov_y_radians } => {
            (camera.eye, (fov_y_radians * 0.5).tan(), 0.0)
        }
        Projection::Orthographic { view_height_m } => {
            (camera.target, 0.0, view_height_m / canvas_h)
        }
    };
    let (focus_lat, focus_lon, _) =
        transform.enu_to_geodetic(focus.x as f64, focus.y as f64, focus.z as f64);
    let view = ViewInfo {
        camera,
        view_proj: camera.view_proj_matrix(),
        transform,
        canvas_h,
        focus,
        focus_lat,
        focus_lon,
        tan_half,
        ortho_pixel_m,
    };

    let k = data.chunks_per_tile();
    let chunk_count = data.chunk_count();
    let max_level = data.num_levels() - 1;

    struct TileInfo {
        key: TileKey,
        distance: f32,
        visible: bool,
        wants_chunks: bool,
        /// wants_chunksのとき: チャンクごとの(距離, 視野内か, 目標レベル)。
        chunks: Vec<(f32, bool, usize)>,
    }
    let mut infos: Vec<TileInfo> = Vec::with_capacity(data.tiles().len());
    for tile in data.tiles() {
        let (lat0, lon0) = (tile.key.0 as f64, tile.key.1 as f64);
        let mid_h = 0.5 * (tile.elevation_min + tile.elevation_max) as f64;
        let (distance, visible) = view.rect(lat0, lon0, lat0 + 1.0, lon0 + 1.0, mid_h);
        let pixel_m = view.pixel_m(distance);
        let current = resident.get(&tile.key);
        let chunked_now = matches!(current, Some(Resident::Chunks(_)));

        // チャンクに切り替えるか: 視野の外は現状維持。見えているタイルは、全体(レベル0)では
        // 粗すぎるならチャンクにする。すでにチャンクなら、余裕を持って足りるようになるまで戻さない。
        let coarse_ok = cell_size_m(data, 0) <= TARGET_CELL_PX * pixel_m;
        let coarse_ok_with_margin = cell_size_m(data, 0) <= REVERT_MARGIN * TARGET_CELL_PX * pixel_m;
        let wants_chunks = if !visible {
            chunked_now
        } else if chunked_now {
            !coarse_ok_with_margin
        } else {
            !coarse_ok
        };

        let mut chunks = Vec::new();
        if wants_chunks {
            let cur_levels: Option<&Vec<u8>> = match current {
                Some(Resident::Chunks(v)) => Some(v),
                _ => None,
            };
            let step = 1.0 / k as f64;
            for c in 0..chunk_count {
                let (cx, cy) = (c % k, c / k);
                let (clat0, clon0) = (lat0 + cy as f64 * step, lon0 + cx as f64 * step);
                let (cdist, cvisible) = view.rect(clat0, clon0, clat0 + step, clon0 + step, mid_h);
                let have = cur_levels.map_or(0, |v| v[c] as usize);
                let ideal = ideal_level(data, view.pixel_m(cdist), 1);
                let target = if !cvisible {
                    have.max(1)
                } else if have > ideal {
                    // 理想より1つだけ細かいなら保つ(ヒステリシス)。それ以上細かければ1つ細かい所まで下げる。
                    (ideal + 1).min(have)
                } else {
                    ideal
                };
                chunks.push((cdist, cvisible, target.clamp(1, max_level)));
            }
        }
        infos.push(TileInfo { key: tile.key, distance, visible, wants_chunks, chunks });
    }

    // 見えているタイルを近い順に、その後に視野の外のタイルを近い順に並べる。
    infos.sort_by(|a, b| {
        b.visible
            .cmp(&a.visible)
            .then(a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal))
    });

    // 予算の配分。まず、チャンクにしたいタイルに、全チャンクをレベル1で載せる最低限の分を確保する
    // (足りなければそのタイルは全体表示のまま)。
    let mut remaining = DETAIL_VERTEX_BUDGET;
    let base_cost = chunk_count * chunk_vertex_cost(data, 1);
    let mut levels: HashMap<TileKey, Vec<u8>> = HashMap::new();
    for info in infos.iter().filter(|i| i.wants_chunks) {
        if remaining >= base_cost {
            remaining -= base_cost;
            levels.insert(info.key, vec![1; chunk_count]);
        }
    }
    // 次に、チャンクを近い順(見えているものが先)に、目標レベルまで予算の許す限り上げる。
    let mut upgrades: Vec<(bool, f32, TileKey, usize, usize)> = Vec::new();
    for info in &infos {
        if levels.contains_key(&info.key) {
            for (c, &(cdist, cvisible, target)) in info.chunks.iter().enumerate() {
                upgrades.push((cvisible, cdist, info.key, c, target));
            }
        }
    }
    upgrades.sort_by(|a, b| {
        b.0.cmp(&a.0).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    for (_, _, key, c, target) in upgrades {
        let entry = levels.get_mut(&key).expect("tile with chunks");
        let mut level = 1usize;
        while level < target {
            let delta = chunk_vertex_cost(data, level + 1) - chunk_vertex_cost(data, level);
            if delta > remaining {
                break;
            }
            remaining -= delta;
            level += 1;
        }
        entry[c] = level as u8;
    }

    infos
        .into_iter()
        .map(|info| {
            let plan = match levels.remove(&info.key) {
                Some(l) => TilePlan::Chunks(l),
                None => TilePlan::Whole,
            };
            (info.key, plan)
        })
        .collect()
}
