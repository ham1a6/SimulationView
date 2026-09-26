//! 標高のサンプリングと、地表点のENU座標。DETAILED_DESIGN.md 3.2節・6.8節。
//! `TerrainData`(いま画面に出しているレベルのグリッド)から標高を引き、楕円体(`EnuTransform`)で
//! ENU座標へ変換する。観測点・見通し計算・クリック判定・注視点の高さ合わせが使う。

use super::geodesy::EnuTransform;
use super::loader::TerrainData;

/// 標高(メートル)を双線形補間でサンプリングする。地形データの範囲外・タイルが無い・海域(周辺4ノードの
/// いずれかがデータなし)は標高0mとして扱う(欠損をそのまま返すと呼び出し側の計算が破綻するため)。
/// 各タイル(のチャンク)は、いま画面に出しているレベルのグリッド(`TerrainData::set_chunk_level`)
/// で引くので、描画されている地形と観測点・見通し計算・クリック判定の標高が一致する。
/// `terrain/los.rs`(見通し)・`terrain/markers.rs`(観測点)・`terrain/profile.rs`(断面図)・
/// `terrain/pick.rs`(クリック判定)・`ui/terrain_view.rs`(カメラ注視点の高さ)から使う。
pub fn sample_heightmap(data: &TerrainData, lat_deg: f64, lon_deg: f64) -> f32 {
    sample(data, lat_deg, lon_deg, false).unwrap_or(0.0)
}

/// 地表に貼り付ける描画用。表示LODの三角形と同じ補間で標高を求める(範囲外は0m)。
pub(crate) fn sample_surface_height(data: &TerrainData, lat_deg: f64, lon_deg: f64) -> f32 {
    sample(data, lat_deg, lon_deg, true).unwrap_or(0.0)
}

/// 範囲外ならNone。
fn sample(data: &TerrainData, lat_deg: f64, lon_deg: f64, surface: bool) -> Option<f32> {
    let b = &data.metadata.geodetic_bounds;
    if lat_deg < b.min_lat || lat_deg > b.max_lat || lon_deg < b.min_lon || lon_deg > b.max_lon {
        return None;
    }

    // 東端・北端ちょうどは、その内側のタイルの端として扱う。
    let lat0 = (lat_deg.floor() as i32).min(b.max_lat as i32 - 1);
    let lon0 = (lon_deg.floor() as i32).min(b.max_lon as i32 - 1);
    let Some(tile) = data.tile((lat0, lon0)) else {
        return Some(0.0);
    };
    let u = (lon_deg - lon0 as f64).clamp(0.0, 1.0);
    let v = (lat_deg - lat0 as f64).clamp(0.0, 1.0);
    Some(if surface {
        data.sample_surface(tile, u, v)
    } else {
        data.sample_bilinear(tile, u, v)
    })
}

/// ENUの水平位置(東, 北)の真上/真下にある地表点の(緯度, 経度, ENU上座標)。
/// 地形メッシュの頂点は楕円体上の地表をENUへ変換したもの(遠方ほど丸みで下がる)なので、
/// 視線との交差判定・注視点の高さ合わせには、標高そのものではなくこの上座標を使うこと。
/// 「(東, 北)を通る鉛直線」と地表の交点は、上座標を仮定→測地座標へ戻す→その地点の地表の
/// 上座標で更新、を数回繰り返して求める(地表の傾きが小さいので速やかに収束する)。
/// 地形データ範囲外・海域(NaN)は標高0mとして扱う。
pub fn ground_at_enu(
    data: &TerrainData,
    transform: &EnuTransform,
    east: f64,
    north: f64,
) -> (f64, f64, f32) {
    // 丸みによる低下量の第一近似を初期値にする(0から始めるより収束が速い)。
    let mut up = -(east * east + north * north) / (2.0 * transform.ellipsoid_params().0);
    let mut lat_lon = (0.0, 0.0);
    for _ in 0..4 {
        let (lat, lon, _h) = transform.enu_to_geodetic(east, north, up);
        let elevation = sample_heightmap(data, lat, lon);
        up = transform.transform_f64(lat, lon, elevation as f64)[2];
        lat_lon = (lat, lon);
    }
    (lat_lon.0, lat_lon.1, up as f32)
}

/// 地表上の(緯度, 経度)のENU座標(東, 北, 上)。`ground_at_enu`の逆向き。
/// 地形データ範囲外・海域(NaN)は標高0mとして扱う。
pub fn ground_at_geodetic(
    data: &TerrainData,
    transform: &EnuTransform,
    lat_deg: f64,
    lon_deg: f64,
) -> (f32, f32, f32) {
    let elevation = sample_heightmap(data, lat_deg, lon_deg);
    let [east, north, up] = transform.transform(lat_deg, lon_deg, elevation as f64);
    (east, north, up)
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
    fn heightmap_is_sampled_inside_bounds_only() {
        let data = TerrainData::synthetic(30, 120, 1, 1, east_slope);
        assert!((sample(&data, 30.5, 120.25, false).unwrap() - 150.0).abs() < 1e-3);
        // 東端・北端ちょうどは範囲内(内側のタイルの端として扱う)。
        assert!((sample(&data, 31.0, 121.0, false).unwrap() - 600.0).abs() < 1e-3);
        assert!(sample(&data, 29.99, 120.5, false).is_none());
        assert!(sample(&data, 30.5, 121.01, false).is_none());
    }

    #[test]
    fn ground_curves_down_like_d2_over_2r() {
        let data = TerrainData::synthetic(30, 130, 10, 10, |_, _| 0);
        let t = transform_at(35.0, 135.0);
        for &(east, north) in &[(400_000.0, 0.0), (0.0, -400_000.0), (300_000.0, 300_000.0)] {
            let (lat, lon, up) = ground_at_enu(&data, &t, east, north);
            let d2 = east * east + north * north;
            let drop = d2 / (2.0 * 6_378_137.0);
            // 楕円体の曲率半径は場所で1%ほど違うので、球の概算とは数%以内で一致すればよい。
            assert!(
                (up as f64 + drop).abs() < 0.03 * drop,
                "east={east} north={north} up={up} drop={drop}"
            );
            // 返した緯度経度の地表(標高0m)を変換し直すと、同じENU位置に戻る。
            let [e2, n2, u2] = t.transform_f64(lat, lon, 0.0);
            assert!(
                (e2 - east).abs() < 1.0 && (n2 - north).abs() < 1.0,
                "{e2},{n2}"
            );
            assert!((u2 - up as f64).abs() < 1.0, "{u2} vs {up}");
        }
    }

    #[test]
    fn ground_follows_terrain_height_far_away() {
        let flat = TerrainData::synthetic(30, 130, 10, 10, |_, _| 0);
        let hill = TerrainData::synthetic(30, 130, 10, 10, |_, _| 1000);
        let t = transform_at(35.0, 135.0);
        let (_, _, up0) = ground_at_enu(&flat, &t, 400_000.0, 0.0);
        let (_, _, up1) = ground_at_enu(&hill, &t, 400_000.0, 0.0);
        // 遠方では鉛直方向が傾くので、1000m高い地表の上座標の差は1000mよりわずかに小さい。
        assert!((up1 - up0 - 1000.0).abs() < 10.0, "{up0} -> {up1}");
    }

    #[test]
    fn same_ground_point_regardless_of_origin() {
        let data = TerrainData::synthetic(30, 130, 10, 10, |lat, lon| ((lat + lon) * 3.0) as i16);
        let (lat, lon) = (33.3, 137.7);
        let expected = sample_heightmap(&data, lat, lon);
        for origin in [(35.0, 135.0), (31.0, 139.0), (39.0, 131.0)] {
            let t = transform_at(origin.0, origin.1);
            let (east, north, _) = ground_at_geodetic(&data, &t, lat, lon);
            let (lat2, lon2, _) = ground_at_enu(&data, &t, east as f64, north as f64);
            // f32のENU座標(数百km)の丸めは0.1m程度なので、緯度経度は1e-5度(約1m)以内で戻る。
            assert!(
                (lat2 - lat).abs() < 1e-5 && (lon2 - lon).abs() < 1e-5,
                "{origin:?}: {lat2},{lon2}"
            );
            assert!((sample_heightmap(&data, lat2, lon2) - expected).abs() < 0.05);
        }
    }
}
