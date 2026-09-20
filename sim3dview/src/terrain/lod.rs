//! 地形LOD(解像度レベル)の計画。「どのタイルを・どの解像度レベルで描くか」を、カメラとの距離・
//! 視錐台・頂点数の予算から決める純粋関数(GPUやネットワークには触れない)。実際のグリッド取得と
//! メッシュ差し替えは`ui::terrain_view`が行う。
//!
//! 構成:
//! - 全タイルを、**1度タイルを分割した6x6のチャンク**で描く。チャンクごとに独立して解像度
//!   レベルを選ぶ。最も粗くてもレベル1(約620m/セル)で、遠くのタイルもこれより粗くはしない。
//!   カメラに近いチャンクほど細かく(最細=元データの30m)なる。
//! - 起動直後は、全タイルを**タイル全体を1枚のメッシュ(レベル0、約1.85km/セル)**で描く
//!   (`loader::load_terrain`が起動時に全部取得して常駐する)。レベル1のグリッドが取得できた
//!   タイルから、近い順にチャンク表示に切り替わる。予算が足りないタイルはレベル0のまま残る。
//! - チャンクの頂点の合計は`DETAIL_VERTEX_BUDGET`以下に抑える(GPUメモリ・描画負荷の上限)。
//!   まず全タイルのレベル1の分(下限)を確保し、残りを「見えていて近いチャンク」から順に配って
//!   細かくする。
//! - 視野の外は、予算が余っている間は現状のレベルを保つ(カメラを戻したときにすぐ細かく見える
//!   ように)が、予算は見えているものより後回しにする。
//! - 理想より1レベル細かいだけなら下げない(距離の境目でレベルが行き来しないヒステリシス)。

use std::collections::HashMap;

use glam::{Mat4, Vec3};

use super::camera::{Camera, Projection};
use super::loader::{TerrainData, TileKey};
use super::mesh::{tile_vertex_count, EnuTransform};

/// チャンクの頂点数の合計の上限(全タイルの下限=レベル1の分を含む)。全タイルのレベル1は
/// 約1520万頂点(390タイル x 36チャンク x 1チャンク1085頂点)で、残りの約980万頂点を細かくする
/// のに使う。最細(30m)のチャンクは1個で約36万頂点なので、下限から最細へ上げられるチャンクは
/// 同時に約27個まで。GPUメモリは頂点(28バイト)とインデックスで合計約1.2GBになる。
/// 以前は600万頂点(下限は近いタイルだけ)だった。
pub const DETAIL_VERTEX_BUDGET: usize = 25_000_000;

/// チャンクのレベル選択に使う: 画面(CSSピクセル)上で1セルがこのピクセル数以下になる最も粗い
/// レベルを選ぶ。描画は2倍スーパーサンプリングなので、1ピクセルなら描画解像度では約2ピクセル分。
/// 最細の30mは、1ピクセルが約62m未満、canvas高さ700pxで視点から約46km以内で使われる。
const CHUNK_TARGET_CELL_PX: f32 = 1.0;

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

/// 1セルが画面上で`CHUNK_TARGET_CELL_PX`以下になる最も粗いレベル(`from`以上)。どれも満たさなければ最細。
fn ideal_level(data: &TerrainData, pixel_m: f32, from: usize) -> usize {
    let max_level = data.num_levels() - 1;
    (from..=max_level)
        .find(|&k| cell_size_m(data, k) <= CHUNK_TARGET_CELL_PX * pixel_m)
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
    plan_levels_with_budget(data, transform, camera, canvas_height_px, resident, DETAIL_VERTEX_BUDGET)
}

/// `plan_levels`の頂点数の予算を指定できる版(単体テストで予算の配分を確かめるため)。
fn plan_levels_with_budget(
    data: &TerrainData,
    transform: &EnuTransform,
    camera: &Camera,
    canvas_height_px: f32,
    resident: &HashMap<TileKey, Resident>,
    budget: usize,
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
        /// チャンクごとの(距離, 視野内か, 目標レベル)。
        chunks: Vec<(f32, bool, usize)>,
    }
    let mut infos: Vec<TileInfo> = Vec::with_capacity(data.tiles().len());
    for tile in data.tiles() {
        let (lat0, lon0) = (tile.key.0 as f64, tile.key.1 as f64);
        let mid_h = 0.5 * (tile.elevation_min + tile.elevation_max) as f64;
        let (distance, visible) = view.rect(lat0, lon0, lat0 + 1.0, lon0 + 1.0, mid_h);
        let cur_levels: Option<&Vec<u8>> = match resident.get(&tile.key) {
            Some(Resident::Chunks(v)) => Some(v),
            _ => None,
        };

        let mut chunks = Vec::with_capacity(chunk_count);
        // タイルの一番近い点でも下限のレベル1で足りるなら、どのチャンクも目標はレベル1になる
        // (チャンクは遠いほど1ピクセルが大きく、理想のレベルは粗くなるため)。視野に入るかも
        // タイルと同じ扱いにして、チャンクごとの距離・視錐台の計算を省く(全タイルがチャンクなので、
        // 遠くの多数のタイルで毎回計算すると重い)。
        let tile_ideal = ideal_level(data, view.pixel_m(distance), 1);
        if !visible || tile_ideal == 1 {
            for c in 0..chunk_count {
                let have = cur_levels.map_or(0, |v| v[c] as usize);
                let target = if !visible {
                    have.max(1) // 視野の外は現状維持(下限は1)。
                } else if have > 1 {
                    2.min(have) // 理想より1つだけ細かいなら保つ(下のヒステリシスと同じ)。
                } else {
                    1
                };
                chunks.push((distance, visible, target.clamp(1, max_level)));
            }
        } else {
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
        infos.push(TileInfo { key: tile.key, distance, visible, chunks });
    }

    // 見えているタイルを近い順に、その後に視野の外のタイルを近い順に並べる。
    infos.sort_by(|a, b| {
        b.visible
            .cmp(&a.visible)
            .then(a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal))
    });

    // 予算の配分。まず、全タイルに、全チャンクをレベル1で載せる下限の分を、近い(見えている)タイル
    // から順に確保する(足りなければそのタイルは全体表示のまま)。
    let mut remaining = budget;
    let base_cost = chunk_count * chunk_vertex_cost(data, 1);
    let mut levels: HashMap<TileKey, Vec<u8>> = HashMap::new();
    for info in &infos {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::camera::{CameraPreset, OrbitCamera};
    use crate::terrain::loader::Ellipsoid;
    use crate::terrain::mesh::Origin;

    /// 実データと同じレベル定義(1度あたり60/180/600/1800/3600セル、6x6チャンク)の3x3タイル
    /// (緯度30〜33度・経度130〜133度)。標高は全部0m。
    fn data() -> TerrainData {
        TerrainData::synthetic_with_levels(30, 130, 3, 3, vec![60, 180, 600, 1800, 3600], 6, |_, _| 0)
    }

    /// 原点は中央のタイル(31,131)の中心。
    fn transform() -> EnuTransform {
        EnuTransform::new(
            &Origin { lat_deg: 31.5, lon_deg: 131.5 },
            &Ellipsoid { a_m: 6_378_137.0, inv_f: 298.257_222_101 },
        )
    }

    /// 原点の真上(標高0m)を注視する既定の3Dカメラ。
    fn camera(target: Vec3, distance: f32) -> Camera {
        let mut orbit = OrbitCamera::preset(CameraPreset::Overview, 0.0);
        orbit.target = target;
        orbit.distance = distance;
        orbit.to_camera(1.5)
    }

    fn plan(cam: &Camera, resident: &HashMap<TileKey, Resident>) -> Vec<(TileKey, TilePlan)> {
        plan_levels(&data(), &transform(), cam, 700.0, resident)
    }

    fn levels_of(plan: &[(TileKey, TilePlan)], key: TileKey) -> Vec<u8> {
        match &plan.iter().find(|(k, _)| *k == key).expect("tile in plan").1 {
            TilePlan::Chunks(l) => l.clone(),
            TilePlan::Whole => panic!("{key:?} is planned as Whole"),
        }
    }

    #[test]
    fn cell_sizes_and_chunk_costs_match_the_documented_numbers() {
        let d = data();
        assert!((cell_size_m(&d, 1) - 616.7).abs() < 0.1);
        assert!((cell_size_m(&d, 4) - 30.8).abs() < 0.1);
        // 設計メモにある「チャンク1個=1085頂点(レベル1)」。
        assert_eq!(chunk_vertex_cost(&d, 1), 1085);
        assert!(chunk_vertex_cost(&d, 4) > 300_000);
    }

    #[test]
    fn ideal_level_picks_the_coarsest_level_fine_enough_for_the_pixel() {
        let d = data();
        assert_eq!(ideal_level(&d, 1.0e6, 1), 1);
        assert_eq!(ideal_level(&d, 200.0, 1), 2); // 185m/セル
        assert_eq!(ideal_level(&d, 50.0, 1), 4); // 61.7mは足りず、30.8m
        assert_eq!(ideal_level(&d, 1.0, 1), 4); // どれも満たさなければ最細
        assert_eq!(ideal_level(&d, 1.0e6, 2), 2); // `from`より粗くはしない
    }

    #[test]
    fn far_view_uses_the_floor_level_for_every_tile() {
        let cam = camera(Vec3::ZERO, 2_000_000.0);
        let result = plan(&cam, &HashMap::new());
        assert_eq!(result.len(), 9);
        for (key, _) in &result {
            assert!(levels_of(&result, *key).iter().all(|&l| l == 1), "{key:?}");
        }
    }

    #[test]
    fn close_view_refines_near_chunks_only_and_lists_the_nearest_tile_first() {
        let cam = camera(Vec3::ZERO, 20_000.0);
        let result = plan(&cam, &HashMap::new());
        // 視点は中央タイルの中にあるので、中央タイルが先頭で、最細のチャンクを持つ。
        assert_eq!(result[0].0, (31, 131));
        let near = levels_of(&result, (31, 131));
        assert_eq!(near.iter().copied().max(), Some(4));
        // 中央タイルの中でも、遠いチャンクは粗い(近いものより細かくならない)。
        assert!(near.iter().any(|&l| l < 4), "{near:?}");
        // 隣の外側のタイルは、中央より細かいチャンクを持たない。
        let far = levels_of(&result, (30, 130));
        assert!(far.iter().copied().max() <= near.iter().copied().max());
    }

    #[test]
    fn budget_decides_which_tiles_get_chunks() {
        let cam = camera(Vec3::ZERO, 20_000.0);
        let d = data();
        let per_tile = d.chunk_count() * chunk_vertex_cost(&d, 1);
        // 下限(全チャンクをレベル1)を3タイル分だけ確保できる予算。上げる余裕は無い。
        let result =
            plan_levels_with_budget(&d, &transform(), &cam, 700.0, &HashMap::new(), 3 * per_tile);
        let chunked: Vec<_> = result
            .iter()
            .filter(|(_, p)| matches!(p, TilePlan::Chunks(_)))
            .map(|(k, _)| *k)
            .collect();
        assert_eq!(chunked.len(), 3);
        // 近い順(先頭から)に確保されるので、視点のあるタイルは必ず含まれる。
        assert!(chunked.contains(&(31, 131)));
        assert!(levels_of(&result, (31, 131)).iter().all(|&l| l == 1));
        // 予算が0なら全タイルが全体表示。
        let none = plan_levels_with_budget(&d, &transform(), &cam, 700.0, &HashMap::new(), 0);
        assert!(none.iter().all(|(_, p)| *p == TilePlan::Whole));
    }

    #[test]
    fn upgrades_never_exceed_the_budget() {
        let cam = camera(Vec3::ZERO, 20_000.0);
        let d = data();
        let per_tile = d.chunk_count() * chunk_vertex_cost(&d, 1);
        let budget = 9 * per_tile + 1_000_000; // 全タイルの下限+100万頂点だけ上げられる
        let result = plan_levels_with_budget(&d, &transform(), &cam, 700.0, &HashMap::new(), budget);
        let used: usize = result
            .iter()
            .map(|(_, p)| match p {
                TilePlan::Chunks(l) => l.iter().map(|&l| chunk_vertex_cost(&d, l as usize)).sum(),
                TilePlan::Whole => 0,
            })
            .sum();
        assert!(used <= budget, "used={used} budget={budget}");
        // 余りを近いチャンクへ回すので、下限だけよりは多く使っている。
        assert!(used > 9 * per_tile);
    }

    #[test]
    fn hysteresis_keeps_one_level_finer_than_ideal_but_not_more() {
        let cam = camera(Vec3::ZERO, 2_000_000.0); // どのタイルも理想はレベル1(見えている)
        let key = (31, 131);
        let with = |levels: Resident| {
            let mut resident = HashMap::new();
            resident.insert(key, levels);
            levels_of(&plan(&cam, &resident), key)
        };
        // 理想(1)より1つ細かい(2)だけなら保つ。
        assert!(with(Resident::Chunks(vec![2; 36])).iter().all(|&l| l == 2));
        // それより細かい(4)なら、1つ細かい所(2)まで下げる。
        assert!(with(Resident::Chunks(vec![4; 36])).iter().all(|&l| l == 2));
        // 全体表示からは、まず下限(1)から。
        assert!(with(Resident::Whole).iter().all(|&l| l == 1));
    }

    #[test]
    fn tiles_outside_the_view_keep_their_current_level() {
        // 注視点を原点の3,000km東へ。原点のタイル群は視野の外(背後か脇)になる。
        let cam = camera(Vec3::new(3_000_000.0, 0.0, 0.0), 100_000.0);
        let key = (31, 131);
        let mut resident = HashMap::new();
        resident.insert(key, Resident::Chunks(vec![3; 36]));
        let result = plan(&cam, &resident);
        assert!(levels_of(&result, key).iter().all(|&l| l == 3));
        // 視野の外のタイルは、見えているタイルより後ろに並ぶ(ここでは全部が外なので順序は距離順)。
        assert_eq!(result.len(), 9);
        // 何も出していなかったタイルは、下限のレベル1。
        assert!(levels_of(&result, (30, 130)).iter().all(|&l| l == 1));
    }
}
