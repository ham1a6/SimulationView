//! メインパネル(3D地形)への右クリックで、クリックされた画面上の位置に対応する地表の
//! 緯度経度を求めるレイキャスト(マウスピッキング)。`components/terrain_view.rs`から使う。
//! GPU側の読み戻しは行わず、CPU側で保持しているheightmapに対してレイを直接マーチングする
//! (地形メッシュの三角形と厳密に一致するわけではないが、見た目上は十分な精度)。

use super::camera::Camera;
use super::loader::TerrainData;
use super::mesh::{ground_at_enu, EnuTransform, Origin};

/// レイをマーチングする最大距離(メートル)。カメラは最大ズームアウト(`camera.rs`の
/// MAX_DISTANCE=2,000,000m)まで地形から離れうるため、そこから地形データ範囲(現在は30°四方、
/// 対角線で約4,000km)の遠端まで届く値にしておく。刻み幅は約1.3kmで、従来(約2.25km)以下。
/// 視線が最高標高より上にある間は地表の評価を省略するので、ステップ数を増やしても軽い。
const MAX_MARCH_DISTANCE: f32 = 8_000_000.0;
const NUM_MARCH_STEPS: usize = 6000;
const NUM_BISECT_STEPS: usize = 24;

/// 画面上の点(canvas内のCSSピクセル座標)から出るレイを地形(heightmap)に対して
/// マーチングし、最初に地表と交差する点の緯度経度を返す。地形データ範囲外・交差なしの
/// 場合はNone。
pub fn pick_lat_lon(
    data: &TerrainData,
    mesh_origin: &Origin,
    camera: &Camera,
    screen_x: f32,
    screen_y: f32,
    canvas_w: f32,
    canvas_h: f32,
) -> Option<(f64, f64)> {
    let (ray_origin, ray_dir) = camera.screen_to_ray(screen_x, screen_y, canvas_w, canvas_h);
    if ray_dir.length_squared() < 1e-12 {
        return None;
    }
    let dir = ray_dir.normalize();
    let transform = EnuTransform::new(mesh_origin, &data.metadata.ellipsoid);

    let diff_at = |t: f32| -> f32 {
        let p = ray_origin + dir * t;
        p.z - ground_at_enu(data, &transform, p.x as f64, p.y as f64).2
    };

    let mut prev_t = 0.0_f32;
    let prev_diff0 = diff_at(0.0);
    if prev_diff0 < 0.0 {
        // カメラ自体が地表の下(通常は発生しない異常系)。
        return None;
    }

    // 地表のENU上座標は(丸みで下がる分も含め)常に最高標高以下なので、視線がそれより上に
    // ある間は交差しえない(高価な地表の評価を省ける)。
    let max_ground_up = data.metadata.elevation_max;
    for i in 1..=NUM_MARCH_STEPS {
        let t = MAX_MARCH_DISTANCE * (i as f32) / (NUM_MARCH_STEPS as f32);
        if (ray_origin + dir * t).z > max_ground_up {
            prev_t = t;
            continue;
        }
        let diff = diff_at(t);
        if diff <= 0.0 {
            let mut lo = prev_t;
            let mut hi = t;
            for _ in 0..NUM_BISECT_STEPS {
                let mid = (lo + hi) * 0.5;
                if diff_at(mid) > 0.0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let hit = ray_origin + dir * hi;
            // 地表の(緯度, 経度)は丸みを考慮して厳密に求める(接平面近似だと、原点から
            // 数百km以上離れた地点で数十kmずれる)。
            let (lat, lon, _) = ground_at_enu(data, &transform, hit.x as f64, hit.y as f64);
            let b = &data.metadata.geodetic_bounds;
            if lat < b.min_lat || lat > b.max_lat || lon < b.min_lon || lon > b.max_lon {
                return None;
            }
            return Some((lat, lon));
        }
        prev_t = t;
    }
    None
}
