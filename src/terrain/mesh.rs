//! 地形メッシュ生成。DETAILED_DESIGN.md 6.5節(頂点構造)・6.7節(配色)。座標変換は`geodesy`、標高の
//! サンプリングは`heightmap`。

use super::geodesy::EnuTransform;
use super::loader::{TerrainData, TileEntry, NO_DATA};

/// 頂点構造(DETAILED_DESIGN.md 6.5節)。UV座標は使わず、標高由来の色を直接持たせる。
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TerrainVertex {
    pub position: [f32; 3], // x(East), y(North), z(Up) — ENU変換結果
    pub color: [f32; 3],
    /// 陰影(ヒルシェード)用の単位法線のx(East)・y(North)成分(snorm16。-32767〜32767が-1〜1)。
    /// z(Up)成分は`sqrt(1-x^2-y^2)`でシェーダーが復元する(地表の法線は常に上向きなので符号は
    /// 決まっている。xyだけにして頂点を4バイト増やすだけで済ませ、snorm8より精度が高い)。
    /// 陰影を付けない頂点(マーカー・覆域・海に隣接して法線が求まらない点)は`UNLIT_NORMAL`。
    pub normal_xy: [i16; 2],
}

impl TerrainVertex {
    /// 「陰影を付けない」を表す法線。xy成分の長さが1を超える(=単位法線ではありえない)値にしてあり、
    /// シェーダーがこれを見て陰影を掛けない。x=y=0(真上向き=平地)とは区別される。
    pub const UNLIT_NORMAL: [i16; 2] = [i16::MIN, i16::MIN];

    /// 陰影を付けない頂点(マーカー・覆域ドームなど、色をそのまま出したいもの)。
    pub fn unlit(position: [f32; 3], color: [f32; 3]) -> Self {
        Self {
            position,
            color,
            normal_xy: Self::UNLIT_NORMAL,
        }
    }
}

pub struct TerrainMesh {
    pub vertices: Vec<TerrainVertex>,
    pub indices: Vec<u32>,
}

/// 色の正規化に使う標高の下限(メートル)。元データ(DSM)には水面などのノイズによる大きな
/// 負の値が一部のタイルにあり、`metadata.elevation_min`をそのまま下限にすると低地全体の
/// 色がずれるため、0m以下はまとめて低地の色にする。
const COLOR_MIN_ELEVATION_M: f32 = 0.0;

/// 標高を正規化し、低地(深緑)→高山(白に近い明色)の地形図的カラーランプへ写像する
/// (DETAILED_DESIGN.md 6.7節)。カラーストップは実装時に調整可能な固定テーブル。
fn elevation_to_color(elevation: f32, min: f32, max: f32) -> [f32; 3] {
    const STOPS: [(f32, [f32; 3]); 5] = [
        (0.0, [0.12, 0.4, 0.18]),   // 低地: 深緑
        (0.25, [0.15, 0.5, 0.2]),   // 緑
        (0.5, [0.55, 0.5, 0.25]),   // 黄土色
        (0.75, [0.45, 0.32, 0.22]), // 茶
        (1.0, [0.95, 0.95, 0.95]),  // 山頂付近: 白に近い明色
    ];

    let t = if max > min {
        ((elevation - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };

    for pair in STOPS.windows(2) {
        let (t0, c0) = pair[0];
        let (t1, c1) = pair[1];
        if t <= t1 {
            let local_t = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
            return [
                c0[0] + (c1[0] - c0[0]) * local_t,
                c0[1] + (c1[1] - c0[1]) * local_t,
                c0[2] + (c1[2] - c0[2]) * local_t,
            ];
        }
    }
    STOPS[STOPS.len() - 1].1
}

/// メッシュの縁に沿って下へ垂らす「スカート」の深さ(メートル)。解像度の違う隣のメッシュ同士は、
/// 縁のノードの高さがわずかに食い違い、縁に隙間ができて背景の黒が見える。縁から下向きの壁
/// (スカート)を付けて隙間を隠す。粗いレベルほど食い違いが大きいので深くしてある。
fn skirt_depth_m(level: usize) -> f32 {
    const DEPTHS: [f32; 5] = [800.0, 400.0, 250.0, 150.0, 100.0];
    DEPTHS[level.min(DEPTHS.len() - 1)]
}

/// グリッド1枚(一辺`cells`セル)のメッシュの頂点数(ノード(N+1)^2 + 縁4辺のスカート4(N+1))。
/// タイル全体(レベル0)でもチャンクでも同じ式。
pub fn tile_vertex_count(cells: usize) -> usize {
    let n = cells + 1;
    n * n + 4 * n
}

/// スカートの辺e(0=南,1=東,2=北,3=西)のk番目のノードの、グリッド内の番号。
fn edge_node(edge: usize, k: usize, cells: usize) -> usize {
    let n = cells + 1;
    match edge {
        0 => k,
        1 => k * n + cells,
        2 => cells * n + k,
        _ => k * n,
    }
}

/// メッシュにするグリッド1枚の位置決め: ノード(列i, 行j)は緯度`lat_start + j*step_deg`、
/// 経度`lon_start + i*step_deg`にある。
struct GridPlacement {
    lat_start: f64,
    lon_start: f64,
    step_deg: f64,
    skirt_depth: f32,
}

/// グリッドの各ノードの法線のxy成分(`TerrainVertex::normal_xy`)。ノードの東西・南北の隣の
/// ノードの位置の差(中心差分。縁のノード・海に隣接するノードは、陸の側だけを使う片側差分)の
/// 外積から求める。位置はENU座標(地球の丸み込み)なので、法線もENUの向きで得られる。
/// 海のノード自身、隣が両側とも海(差分が取れない)のノードは陰影なし(`UNLIT_NORMAL`)。
/// チャンクの縁のノードは隣のチャンクの標高を見ないので片側差分になり、隣のチャンクの縁と
/// わずかに食い違うが、目立たない程度(下記の実機確認参照)。
fn node_normals(grid: &[i16], n: usize, positions: &[[f64; 3]]) -> Vec<[i16; 2]> {
    let land = |i: usize, j: usize| grid[j * n + i] != NO_DATA;
    // 隣(-1/+1)のうち陸のものを選ぶ。なければ自分自身。
    let neighbor = |k: usize, lo_land: bool, hi_land: bool| -> (usize, usize) {
        (
            if lo_land { k - 1 } else { k },
            if hi_land { k + 1 } else { k },
        )
    };

    let mut normals = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            if !land(i, j) {
                normals.push(TerrainVertex::UNLIT_NORMAL);
                continue;
            }
            let (i_lo, i_hi) = neighbor(i, i > 0 && land(i - 1, j), i + 1 < n && land(i + 1, j));
            let (j_lo, j_hi) = neighbor(j, j > 0 && land(i, j - 1), j + 1 < n && land(i, j + 1));
            if i_lo == i_hi || j_lo == j_hi {
                normals.push(TerrainVertex::UNLIT_NORMAL);
                continue;
            }
            let (e0, e1) = (positions[j * n + i_lo], positions[j * n + i_hi]);
            let (n0, n1) = (positions[j_lo * n + i], positions[j_hi * n + i]);
            let east = [e1[0] - e0[0], e1[1] - e0[1], e1[2] - e0[2]];
            let north = [n1[0] - n0[0], n1[1] - n0[1], n1[2] - n0[2]];
            // 東向き x 北向き = 上向き。
            let cross = [
                east[1] * north[2] - east[2] * north[1],
                east[2] * north[0] - east[0] * north[2],
                east[0] * north[1] - east[1] * north[0],
            ];
            let len = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
            if len < 1e-9 || cross[2] <= 0.0 {
                normals.push(TerrainVertex::UNLIT_NORMAL);
                continue;
            }
            let pack = |v: f64| ((v / len).clamp(-1.0, 1.0) * 32767.0).round() as i16;
            normals.push([pack(cross[0]), pack(cross[1])]);
        }
    }
    normals
}

/// グリッドの頂点列を、指定した原点のENU座標で作る。並びは、ノード(行=南→北、列=西→東)の後に、
/// スカート4辺(南・東・北・西)の順。頂点数と並びは原点に依存しない(原点変更時は
/// `TerrainRenderer::update_mesh_vertices`で位置だけを書き換える)。行(緯度)・列(経度)ごとの
/// 三角関数を前計算して、1頂点あたりの計算を軽くしてある(原点変更時に全メッシュの頂点を
/// 作り直すため)。
fn grid_vertices(
    grid: &[i16],
    cells: usize,
    place: &GridPlacement,
    max_elevation: f32,
    transform: &EnuTransform,
) -> Vec<TerrainVertex> {
    let n = cells + 1;
    let (a, e2) = transform.ellipsoid_params();

    // 行(緯度)ごと: (sinφ, cosφ, 卯酉線曲率半径N)。列(経度)ごと: (sinλ, cosλ)。
    let rows: Vec<(f64, f64, f64)> = (0..n)
        .map(|j| {
            let (s, c) = (place.lat_start + j as f64 * place.step_deg)
                .to_radians()
                .sin_cos();
            (s, c, a / (1.0 - e2 * s * s).sqrt())
        })
        .collect();
    let cols: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            (place.lon_start + i as f64 * place.step_deg)
                .to_radians()
                .sin_cos()
        })
        .collect();

    // ノード(行j, 列i)の、楕円体高hでのENU座標(東, 北, 上)。
    let enu = |j: usize, i: usize, h: f64| -> [f64; 3] {
        let (s, c, prime) = rows[j];
        let (sl, cl) = cols[i];
        let x = (prime + h) * c * cl;
        let y = (prime + h) * c * sl;
        let z = (prime * (1.0 - e2) + h) * s;
        transform.ecef_to_enu(x, y, z)
    };

    let mut vertices = Vec::with_capacity(tile_vertex_count(cells));
    // 法線を求めるためのノードのENU位置(f64。原点から遠いタイルではf32だと隣のノードとの差が
    // 誤差に埋もれて陰影がざらつくので、丸める前の値を使う)。
    let mut positions: Vec<[f64; 3]> = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let value = grid[j * n + i];
            // データなし(海域)の頂点位置はNaNだと破綻するため標高0mで配置するが、この頂点を
            // 含む三角形は`grid_indices`で捨てるので描画されない。
            let (h, color) = if value == NO_DATA {
                (0.0, [0.0; 3])
            } else {
                (
                    value as f64,
                    elevation_to_color(value as f32, COLOR_MIN_ELEVATION_M, max_elevation),
                )
            };
            let [east, north, up] = enu(j, i, h);
            positions.push([east, north, up]);
            vertices.push(TerrainVertex::unlit(
                [east as f32, north as f32, up as f32],
                color,
            ));
        }
    }
    for (vertex, normal_xy) in vertices.iter_mut().zip(node_normals(grid, n, &positions)) {
        vertex.normal_xy = normal_xy;
    }

    // スカート(チャンク・タイルの縁の壁)の底。`place.skirt_depth`だけ下げるが、海抜0m(水域レイヤーの面)
    // より上には止めず、少なくとも水域まで届かせる。標高の高い縁(データ範囲の端など)の下に隙間が
    // 残ると、縁の外の低い視点から、その下を通して地形の裏側が見える(水域の面より上を通る視線は
    // 水域の深度で隠れないため)。
    for edge in 0..4 {
        for k in 0..n {
            let node = edge_node(edge, k, cells);
            let (j, i) = (node / n, node % n);
            let h = if grid[node] == NO_DATA {
                0.0
            } else {
                grid[node] as f64
            };
            let bottom = enu(j, i, (h - place.skirt_depth as f64).min(0.0));
            let mut v = vertices[node];
            v.position = [bottom[0] as f32, bottom[1] as f32, bottom[2] as f32];
            vertices.push(v);
        }
    }

    vertices
}

/// グリッドの三角形インデックスを作る。データなし(海域)のノードを1つでも含む三角形は張らない
/// (海は描画せず背景色のまま見える)。スカートは、隣り合う2ノードがどちらも陸のときだけ壁を張る。
/// 三角形の有無はグリッドだけで決まり原点に依存しない。
fn grid_indices(grid: &[i16], cells: usize) -> Vec<u32> {
    let n = cells + 1;
    let land = |k: usize| grid[k] != NO_DATA;

    let mut indices = Vec::with_capacity(cells * cells * 6);
    for j in 0..cells {
        for i in 0..cells {
            let i0 = j * n + i;
            let i1 = i0 + 1;
            let i2 = i0 + n;
            let i3 = i2 + 1;
            if land(i0) && land(i1) && land(i2) {
                indices.extend_from_slice(&[i0 as u32, i1 as u32, i2 as u32]);
            }
            if land(i1) && land(i3) && land(i2) {
                indices.extend_from_slice(&[i1 as u32, i3 as u32, i2 as u32]);
            }
        }
    }

    let skirt_base = n * n;
    for edge in 0..4 {
        for k in 0..cells {
            let (a, b) = (edge_node(edge, k, cells), edge_node(edge, k + 1, cells));
            if land(a) && land(b) {
                let (sa, sb) = (
                    (skirt_base + edge * n + k) as u32,
                    (skirt_base + edge * n + k + 1) as u32,
                );
                indices.extend_from_slice(&[a as u32, b as u32, sa, b as u32, sb, sa]);
            }
        }
    }

    indices
}

/// タイル全体(レベル0)の頂点列。
pub fn build_whole_tile_vertices(
    data: &TerrainData,
    tile: &TileEntry,
    transform: &EnuTransform,
) -> Vec<TerrainVertex> {
    let cells = data.level_cells(0);
    let place = GridPlacement {
        lat_start: tile.key.0 as f64,
        lon_start: tile.key.1 as f64,
        step_deg: 1.0 / cells as f64,
        skirt_depth: skirt_depth_m(0),
    };
    grid_vertices(
        data.whole_grid(tile),
        cells,
        &place,
        data.metadata.elevation_max,
        transform,
    )
}

/// タイル全体(レベル0)のメッシュ。
pub fn build_whole_tile_mesh(
    data: &TerrainData,
    tile: &TileEntry,
    transform: &EnuTransform,
) -> TerrainMesh {
    TerrainMesh {
        vertices: build_whole_tile_vertices(data, tile, transform),
        indices: grid_indices(data.whole_grid(tile), data.level_cells(0)),
    }
}

/// チャンク(行(南→北)*分割数+列(西→東))・レベル(1以上)の頂点列。グリッドが未取得ならNone。
pub fn build_chunk_vertices(
    data: &TerrainData,
    tile: &TileEntry,
    chunk: usize,
    level: usize,
    transform: &EnuTransform,
) -> Option<Vec<TerrainVertex>> {
    let grid = data.chunk_grid(tile, level, chunk)?;
    let k = data.chunks_per_tile();
    let (cx, cy) = (chunk % k, chunk / k);
    let cells = data.chunk_cells(level);
    let step_deg = 1.0 / data.level_cells(level) as f64;
    let place = GridPlacement {
        lat_start: tile.key.0 as f64 + (cy * cells) as f64 * step_deg,
        lon_start: tile.key.1 as f64 + (cx * cells) as f64 * step_deg,
        step_deg,
        skirt_depth: skirt_depth_m(level),
    };
    Some(grid_vertices(
        &grid,
        cells,
        &place,
        data.metadata.elevation_max,
        transform,
    ))
}

/// チャンク・レベル(1以上)のメッシュ。グリッドが未取得ならNone。
pub fn build_chunk_mesh(
    data: &TerrainData,
    tile: &TileEntry,
    chunk: usize,
    level: usize,
    transform: &EnuTransform,
) -> Option<TerrainMesh> {
    let vertices = build_chunk_vertices(data, tile, chunk, level, transform)?;
    let grid = data.chunk_grid(tile, level, chunk)?;
    Some(TerrainMesh {
        vertices,
        indices: grid_indices(&grid, data.chunk_cells(level)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::geodesy::Ellipsoid;
    use crate::terrain::origin::Origin;

    fn transform_at(lat: f64, lon: f64) -> EnuTransform {
        EnuTransform::new(
            &Origin {
                lat_deg: lat,
                lon_deg: lon,
            },
            &Ellipsoid::WGS84,
        )
    }

    /// 東へ1度で600m上がる斜面(ノード間隔1/6度で整数になる)。タイル(30,120)用。
    fn east_slope(_lat: f64, lon: f64) -> i16 {
        (600.0 * (lon - 120.0)).round() as i16
    }

    #[test]
    fn color_ramp_clamps_and_handles_empty_range() {
        let same = |a: [f32; 3], b: [f32; 3]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5);
        let lowest = [0.12, 0.4, 0.18];
        let highest = [0.95, 0.95, 0.95];
        assert!(same(elevation_to_color(-50.0, 0.0, 1000.0), lowest));
        assert!(same(elevation_to_color(0.0, 0.0, 1000.0), lowest));
        assert!(same(elevation_to_color(5000.0, 0.0, 1000.0), highest));
        // 範囲が空(max<=min)なら常に最低標高の色。
        assert!(same(elevation_to_color(500.0, 0.0, 0.0), lowest));
        // 中間はストップの間を線形補間する(t=0.125は、最初の2ストップの中点)。
        let mid = elevation_to_color(125.0, 0.0, 1000.0);
        assert!((mid[1] - 0.45).abs() < 1e-5, "{mid:?}");
    }

    #[test]
    fn skirt_gets_shallower_at_finer_levels() {
        assert_eq!(skirt_depth_m(0), 800.0);
        assert!(skirt_depth_m(1) < skirt_depth_m(0));
        // 定義済みの段数より細かいレベルは、最後の深さのまま。
        assert_eq!(skirt_depth_m(4), skirt_depth_m(99));
    }

    #[test]
    fn whole_tile_mesh_has_the_documented_vertex_count() {
        let data = TerrainData::synthetic(30, 120, 1, 1, east_slope);
        let tile = data.tile((30, 120)).unwrap();
        let t = transform_at(30.5, 120.5);
        let mesh = build_whole_tile_mesh(&data, tile, &t);
        let cells = data.level_cells(0);
        assert_eq!(mesh.vertices.len(), tile_vertex_count(cells));
        // 全部陸: 地表 2*cells^2 三角形 + スカート 4辺*cells*2 三角形。
        assert_eq!(mesh.indices.len(), 3 * (2 * cells * cells + 4 * cells * 2));
        assert!(mesh
            .indices
            .iter()
            .all(|&i| (i as usize) < mesh.vertices.len()));
        // 原点(タイル中央のノード)の頂点は、ENUの原点=(0,0,標高)にある。
        let n = cells + 1;
        let center = mesh.vertices[(cells / 2) * n + cells / 2].position;
        assert!(center[0].abs() < 1.0 && center[1].abs() < 1.0, "{center:?}");
        assert!((center[2] - 300.0).abs() < 1.0, "{center:?}");
    }

    #[test]
    fn triangles_touching_no_data_are_dropped() {
        // 全部陸のグリッドから、1ノードだけデータなしにする。
        let cells = 4;
        let n = cells + 1;
        let full = vec![10i16; n * n];
        let mut holed = full.clone();
        holed[2 * n + 2] = NO_DATA; // 中央のノード
        let count = |g: &[i16]| grid_indices(g, cells).len() / 3;
        let all = count(&full);
        // 中央のノードを含む三角形は6枚(内側のノード1個は、周囲の4セル・計6三角形に含まれる)。
        assert_eq!(count(&holed), all - 6);
        assert!(grid_indices(&holed, cells)
            .iter()
            .all(|&i| i as usize != 2 * n + 2));

        // 縁のノードがデータなしなら、その隣り合う2辺のスカートの壁も張らない。
        let mut edge_hole = full.clone();
        edge_hole[2] = NO_DATA; // 南の縁の中央
                                // 地表は節点2を含む3枚(セル1の2枚+セル2の1枚)、スカートは節点2に接する2区間の壁(2枚ずつ)。
        assert_eq!(count(&full) - count(&edge_hole), 3 + 2 * 2);
    }

    #[test]
    fn normals_are_lit_only_where_defined_and_lean_away_from_the_slope() {
        let data = TerrainData::synthetic(30, 120, 1, 1, east_slope);
        let tile = data.tile((30, 120)).unwrap();
        let t = transform_at(30.5, 120.5);
        let cells = data.level_cells(0);
        let n = cells + 1;
        let vertices = build_whole_tile_vertices(&data, tile, &t);
        let normals = node_normals(
            data.whole_grid(tile),
            n,
            &vertices[..n * n]
                .iter()
                .map(|v| {
                    [
                        v.position[0] as f64,
                        v.position[1] as f64,
                        v.position[2] as f64,
                    ]
                })
                .collect::<Vec<_>>(),
        );
        // 東へ上る斜面の法線は、西を向く(x<0)。南北方向には傾かない(y≈0)。
        let center = normals[(cells / 2) * n + cells / 2];
        assert!(center[0] < -100, "{center:?}");
        assert!(center[1].abs() < 5, "{center:?}");
        // 四隅のノードは片側に隣がないだけなので、法線は求まる。
        assert_ne!(normals[0], TerrainVertex::UNLIT_NORMAL);

        // 海(NO_DATA)のノードと、隣が両側とも海のノードは陰影なし。
        let mut grid = data.whole_grid(tile).to_vec();
        let positions = vec![[0.0, 0.0, 0.0]; n * n];
        grid[3 * n + 3] = NO_DATA;
        let lit = node_normals(&grid, n, &positions);
        assert_eq!(lit[3 * n + 3], TerrainVertex::UNLIT_NORMAL);
    }
}
