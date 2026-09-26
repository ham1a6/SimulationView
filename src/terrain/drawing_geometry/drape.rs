//! 地表貼り付け図形と表示LODの三角形を交差させる。設計書9.11節。
//! 点サンプリングによる細分化では山頂を見落とすため、地形の面そのものを切り抜く。
//!
//! 計算はすべて経度・緯度の平面([経度, 緯度]の2次元)で行う:
//! - 塗り: 図形の輪郭を三角形に分け、その三角形ごとに、重なる地形の三角形
//!   (`TerrainData::visit_surface_triangles`)を図形の三角形で切り抜く(`clip_triangle`)。
//!   切り抜いた多角形の頂点の高さは、地形の三角形の頂点(GPUへ渡すのと同じf32の位置)から
//!   補間する(`mapper`)ので、画面に描かれている地形の面にぴったり沿う。
//! - 線: 輪郭の各辺を、地形の三角形ごとに、その三角形の内側にある区間へ切り分けて出す。
//!
//! 地形の三角形は、経度・緯度の平面で反時計回り(内側が`orient`の正の側)で渡される前提。

use super::*;
use crate::terrain::loader::{GeodeticBounds, TerrainData};

/// 点列([経度, 緯度])を囲む矩形。端がちょうど地形のセルの境界に乗っても隣のセルを取りこぼさないよう、
/// わずかに広げる。
fn bounds(points: &[[f64; 2]]) -> GeodeticBounds {
    GeodeticBounds {
        min_lon: points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min) - 1e-10,
        max_lon: points
            .iter()
            .map(|p| p[0])
            .fold(f64::NEG_INFINITY, f64::max)
            + 1e-10,
        min_lat: points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min) - 1e-10,
        max_lat: points
            .iter()
            .map(|p| p[1])
            .fold(f64::NEG_INFINITY, f64::max)
            + 1e-10,
    }
}

/// 凸三角形で切り抜く。出力頂点は常に入力の地形三角形の内部にある。
///
/// 地形の三角形`surface`を、図形の三角形`shape`の3辺それぞれの内側の半平面で順に切っていく
/// (Sutherland–Hodgman法)。結果は凸多角形(最大6頂点。空なら重なりなし)。
fn clip_triangle(surface: [[f64; 2]; 3], mut shape: [[f64; 2]; 3]) -> Vec<[f64; 2]> {
    // 図形の三角形を反時計回りにそろえ、各辺の左側(`orient`が正)を内側にする。
    if orient(shape[0], shape[1], shape[2]) < 0.0 {
        shape.swap(1, 2);
    }
    let mut poly = surface.to_vec();
    for edge in 0..3 {
        let (a, b) = (shape[edge], shape[(edge + 1) % 3]);
        let mut next = Vec::with_capacity(7);
        if poly.is_empty() {
            break;
        }
        // 多角形の各辺p→qについて: 境界をまたぐなら交点を、qが内側ならqを出す。
        let mut p = *poly.last().unwrap();
        let mut dp = orient(a, b, p);
        for &q in &poly {
            let dq = orient(a, b, q);
            if (dp >= 0.0) != (dq >= 0.0) {
                // 符号付きの距離dp・dqの比で、辺p→q上の交点の位置が決まる。
                next.push(mix(p, q, dp / (dp - dq)));
            }
            if dq >= 0.0 {
                next.push(q);
            }
            p = q;
            dp = dq;
        }
        poly = next;
    }
    poly
}

/// 地形の三角形と同じf32頂点を補間し、ENUの上方向へ持ち上げる。
/// 切り抜いた点を再度標高サンプリングすると、別のLODを引くので禁止する。
///
/// `tri`は地形の三角形の頂点([経度, 緯度, 標高])。返す関数は、三角形内の点([経度, 緯度])を
/// 重心座標で補間したENUの位置に、`height`(地表からの高さ)と`DRAWING_M`を上座標へ足したものにする。
fn mapper<'a>(
    ctx: &'a BuildContext,
    tri: [[f64; 3]; 3],
    height: f64,
) -> impl Fn([f64; 2]) -> [f32; 3] + 'a {
    let xy = tri.map(|p| [p[0], p[1]]);
    // 地形メッシュと同じく`transform`(f32に丸める)で頂点を作る。丸めた値どうしを補間することで、
    // GPUが描く地形の面と同じ位置になる。
    let xyz = tri.map(|p| ctx.mesh_transform.transform(p[1], p[0], p[2]));
    // 三角形の符号付き面積の2倍(重心座標の分母)。
    let area = orient(xy[0], xy[1], xy[2]);
    move |p| {
        // 各頂点の重み = 向かい側の辺と点pで作る三角形の面積の割合。
        let weights = [
            orient(xy[1], xy[2], p) / area,
            orient(xy[2], xy[0], p) / area,
        ];
        let w = [weights[0], weights[1], 1.0 - weights[0] - weights[1]];
        std::array::from_fn(|k| {
            let value = (0..3).map(|i| xyz[i][k] as f64 * w[i]).sum::<f64>();
            (value + if k == 2 { height + DRAWING_M } else { 0.0 }) as f32
        })
    }
}

/// 地表に貼り付ける2D図形(`frame`は`World`・`AboveGround(height)`)を、表示中の地形の面に重ねて出す。
pub(super) fn emit(
    sink: &mut Sink,
    ctx: &BuildContext,
    terrain: &TerrainData,
    frame: &Frame2d,
    geom: &Geom2d,
    style: &Style,
    height: f64,
) {
    let Frame2d::World {
        lat_deg,
        lon_deg,
        radius_m,
        ..
    } = frame
    else {
        return;
    };
    // 図形のローカル座標(メートル)を[経度, 緯度]にする。
    let geodetic = |p: [f64; 2]| {
        let (lat, lon) = from_local(*lat_deg, *lon_deg, p, *radius_m);
        [lon, lat]
    };
    if let Some(fill) = style.fill {
        // 塗りと輪郭で同じ境界点を使う。長い辺を緯度経度へ写すと直線ではなくなる。
        let footprint: Vec<_> = geom
            .outlines
            .iter()
            .filter(|o| o.closed)
            .flat_map(|o| {
                let points: Vec<_> = o.points.iter().copied().map(geodetic).collect();
                triangulate(&points)
            })
            .collect();
        for &shape in footprint.as_chunks::<3>().0 {
            terrain.visit_surface_triangles(bounds(&shape), |surface| {
                let xy = surface.map(|p| [p[0], p[1]]);
                let poly = clip_triangle(xy, shape);
                if poly.len() < 3 {
                    return;
                }
                // 切り抜いた凸多角形を、最初の頂点からの扇で三角形にする(面積0の三角形は出さない)。
                let map = mapper(ctx, surface, height);
                for i in 1..poly.len() - 1 {
                    if orient(poly[0], poly[i], poly[i + 1]).abs() > 1e-18 {
                        push_triangle(
                            sink,
                            fill.to_array(),
                            [map(poly[0]), map(poly[i]), map(poly[i + 1])],
                            None,
                        );
                    }
                }
            });
        }
    }
    if let Some(stroke) = style.visible_stroke() {
        for outline in &geom.outlines {
            for (a, b) in segments(&outline.points, outline.closed) {
                let (a, b) = (geodetic(a), geodetic(b));
                // 辺a→bのうち、各地形三角形の内側にある区間[lo, hi](0〜1の割合)と、その両端の位置。
                let mut pieces = Vec::new();
                terrain.visit_surface_triangles(bounds(&[a, b]), |surface| {
                    let xy = surface.map(|p| [p[0], p[1]]);
                    // 三角形の3辺それぞれの内側の半平面で、線分のパラメータ区間を狭める。
                    let (mut lo, mut hi): (f64, f64) = (0.0, 1.0);
                    for edge in 0..3 {
                        let p = xy[edge];
                        let q = xy[(edge + 1) % 3];
                        let da = orient(p, q, a);
                        let db = orient(p, q, b);
                        // 両端とも外側なら、この三角形とは重ならない。
                        if da < 0.0 && db < 0.0 {
                            return;
                        }
                        // 境界をまたぐ: 外→内なら入る位置(loを上げる)、内→外なら出る位置(hiを下げる)。
                        if (da < 0.0) != (db < 0.0) {
                            let t = da / (da - db);
                            if da < 0.0 {
                                lo = lo.max(t);
                            } else {
                                hi = hi.min(t);
                            }
                        }
                    }
                    if hi - lo <= 1e-10 {
                        return;
                    }
                    let map = mapper(ctx, surface, height);
                    pieces.push((lo, hi, [map(mix(a, b, lo)), map(mix(a, b, hi))]));
                });
                // 地形の共有辺と一致する線を二重に描かない。LOD境界は高い側を優先する。
                pieces.sort_by(|a, b| {
                    a.0.total_cmp(&b.0)
                        .then(a.1.total_cmp(&b.1))
                        .then(b.2[0][2].total_cmp(&a.2[0][2]))
                });
                let mut previous = None;
                for (lo, hi, points) in pieces {
                    if previous.is_some_and(|(p, q): (f64, f64)| {
                        (lo - p).abs() < 1e-10 && (hi - q).abs() < 1e-10
                    }) {
                        continue;
                    }
                    push_line_strip(
                        sink,
                        &points,
                        false,
                        stroke.to_array(),
                        style.stroke_width_px,
                    );
                    previous = Some((lo, hi));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::{drawing::Color, heightmap, loader::NO_DATA, mesh};

    fn height_at(vertices: &[DrawVertex], point: [f32; 3]) -> Option<f64> {
        vertices.as_chunks::<3>().0.iter().find_map(|tri| {
            let xy = tri.map(|v| [v.position[0] as f64, v.position[1] as f64]);
            let p = [point[0] as f64, point[1] as f64];
            let area = orient(xy[0], xy[1], xy[2]);
            if area.abs() < 1e-10 {
                return None;
            }
            let w = [
                orient(xy[1], xy[2], p) / area,
                orient(xy[2], xy[0], p) / area,
                orient(xy[0], xy[1], p) / area,
            ];
            w.iter()
                .all(|v| *v >= -1e-6)
                .then(|| (0..3).map(|i| w[i] * tri[i].position[2] as f64).sum())
        })
    }

    #[test]
    fn peak_between_all_old_probes_is_covered_by_the_actual_mesh() {
        let terrain =
            TerrainData::synthetic_with_levels(35, 138, 1, 1, vec![60, 3600], 6, |_, _| 0);
        let cells = terrain.chunk_cells(1);
        let mut grid = vec![0; (cells + 1) * (cells + 1)];
        grid[18 * (cells + 1) + 27] = 1200;
        terrain.insert_chunk_grid((35, 138), 1, 0, grid);
        terrain.set_chunk_level((35, 138), 0, 1);
        let origin = Origin {
            lat_deg: 35.0,
            lon_deg: 138.0,
        };
        let transform = EnuTransform::new(&origin, &Ellipsoid::WGS84);
        let ground = |lat, lon| heightmap::sample_surface_height(&terrain, lat, lon) as f64;
        let ctx = BuildContext {
            terrain: Some(&terrain),
            mesh_transform: &transform,
            ellipsoid: &Ellipsoid::WGS84,
            ground: &ground,
            viewport_px: (800.0, 600.0),
        };
        let at = |lat, lon| Position::world(lat, lon, Altitude::AboveGround(0.0));
        let points = [
            at(35.001, 138.001),
            at(35.001, 138.018),
            at(35.018, 138.001),
        ];
        let (frame, _) = Frame2d::at(&ctx, &points[0]);
        let geom = polygon_geom(
            &points
                .iter()
                .map(|p| frame.local_of(&ctx, p))
                .collect::<Vec<_>>(),
            STEPS_NONE,
        );
        let style = Style::filled(Color::rgb(1.0, 0.0, 0.0));
        let mut batch = Batch::default();
        emit_geom2d(&mut Sink::Split(&mut batch), &ctx, &frame, &geom, &style);
        let rendered =
            mesh::build_chunk_mesh(&terrain, terrain.tile((35, 138)).unwrap(), 0, 1, &transform)
                .unwrap();
        let peak = rendered.vertices[18 * (cells + 1) + 27].position;
        assert!(height_at(&batch.opaque, peak).unwrap() >= peak[2] as f64 + DRAWING_M - 0.01);
        // 旧方式は中点・重心がすべて平地だと山を見落とす。
        let legacy = BuildContext {
            terrain: None,
            ..ctx
        };
        let mut old = Batch::default();
        emit_geom2d(&mut Sink::Split(&mut old), &legacy, &frame, &geom, &style);
        assert!(height_at(&old.opaque, peak).unwrap() < peak[2] as f64 - 100.0);
        // 山の全斜面でも、GPUに渡す地形三角形の内部より上にある。
        for tri in rendered
            .indices
            .chunks_exact(3)
            .filter(|tri| tri.contains(&((18 * (cells + 1) + 27) as u32)))
        {
            let p = std::array::from_fn(|k| {
                tri.iter()
                    .map(|&i| rendered.vertices[i as usize].position[k])
                    .sum::<f32>()
                    / 3.0
            });
            assert!(height_at(&batch.opaque, p).unwrap() >= p[2] as f64 + DRAWING_M - 0.01);
        }
        terrain.set_whole_tile((35, 138));
        let mut coarse = Batch::default();
        emit_geom2d(&mut Sink::Split(&mut coarse), &ctx, &frame, &geom, &style);
        assert!(
            height_at(&coarse.opaque, peak).unwrap() < 100.0,
            "非表示の詳細LODを使わない"
        );
    }

    #[test]
    fn wide_concave_polygon_preserves_area_without_refinement_budget() {
        let terrain = TerrainData::synthetic_with_levels(35, 138, 1, 1, vec![60], 6, |_, _| 500);
        let transform = EnuTransform::new(
            &Origin {
                lat_deg: 35.0,
                lon_deg: 138.0,
            },
            &Ellipsoid::WGS84,
        );
        let ground = |_, _| 500.0;
        let ctx = BuildContext {
            terrain: Some(&terrain),
            mesh_transform: &transform,
            ellipsoid: &Ellipsoid::WGS84,
            ground: &ground,
            viewport_px: (800.0, 600.0),
        };
        let points: Vec<_> = [
            (35.1, 138.1),
            (35.1, 138.9),
            (35.3, 138.9),
            (35.3, 138.3),
            (35.9, 138.3),
            (35.9, 138.1),
        ]
        .into_iter()
        .map(|(lat, lon)| Position::world(lat, lon, Altitude::AboveGround(0.0)))
        .collect();
        let drawing = Drawing {
            id: 1,
            visible: true,
            shape: Shape::Polygon { points },
            style: Style::filled(Color::rgb(1.0, 0.0, 0.0)),
        };
        let batch = build(&ctx, &[drawing]);
        assert!(!batch.world.opaque.is_empty());
        assert!(
            batch.world.opaque.len() < MAX_REFINED_VERTICES,
            "地形LODに必要な面だけを生成する"
        );
        let inside = transform.transform(35.2, 138.2, 500.0);
        let outside = transform.transform(35.6, 138.6, 500.0);
        assert!(height_at(&batch.world.opaque, inside).is_some());
        assert!(
            height_at(&batch.world.opaque, outside).is_none(),
            "凹部を塗らない"
        );
        let expected = (0.8 * 0.2 + 0.6 * 0.2) * 111000.0 * 91000.0;
        let area: f64 = batch
            .world
            .opaque
            .as_chunks::<3>()
            .0
            .iter()
            .map(|tri| {
                let xy = tri.map(|v| [v.position[0] as f64, v.position[1] as f64]);
                orient(xy[0], xy[1], xy[2]).abs() * 0.5
            })
            .sum();
        assert!(
            (area / expected - 1.0).abs() < 0.02,
            "塗りの欠落・重複: {area}"
        );
    }

    #[test]
    fn missing_triangle_is_water_without_lowering_adjacent_land() {
        let terrain = TerrainData::synthetic_with_levels(35, 138, 1, 1, vec![60], 6, |lat, lon| {
            if lat > 35.5 && lon > 138.5 {
                NO_DATA
            } else {
                300
            }
        });
        let mut land = 0;
        let mut sea = 0;
        terrain.visit_surface_triangles(
            GeodeticBounds {
                min_lat: 35.5,
                max_lat: 35.51,
                min_lon: 138.5,
                max_lon: 138.51,
            },
            |tri| {
                if tri.iter().all(|p| p[2] == 300.0) {
                    land += 1;
                }
                if tri.iter().all(|p| p[2] == 0.0) {
                    sea += 1;
                }
            },
        );
        assert_eq!((land, sea), (1, 1));
    }
}
