//! 3Dモデルの配置(位置・向き・大きさ)と、モデルで描くかシンボルで描くかの判定。GPUにもDOMにも触れない純粋な計算。
//!
//! - **向き**: 機体座標(x=右、y=前、z=上)を、ヘディング・ピッチ・ロール(航空機の一般的なZ-Y-X回転)でその地点の
//!   東・北・上(`EnuTransform::local_frame`。地球の丸みで原点の「上」からかたむく)へ回す。
//!   ヘディングは北から時計回り、ピッチは機首上げが正、ロールは右翼が下がるのが正。
//! - **表示方式**(`ModelDisplayMode`): 距離で切り替える(`SwitchToSymbol`)か、最小の画面サイズを保証する
//!   (`MinScreenSize`)か。カメラからの奥行きは、2D(正射影)なら視野角50度の透視投影で同じ縦幅が映る距離に換算する。

use std::collections::{HashMap, HashSet};

use glam::{Mat3, Mat4, Vec3};

use super::types::ModelInstance;
use super::{ModelDisplayMode, ModelSource};
use crate::terrain::camera::{Camera, Projection};
use crate::terrain::drawing_geometry::{height_of, BuildContext};
use crate::terrain::tracks::{SymbolKind, TrackEntry, TrackId};

/// 地表基準(`AboveGround`)の高度のとき、モデルの基準点を地表から持ち上げる高さ(メートル)。粗い地形LODとの
/// 高さのずれで足元が地面に埋まるのを避ける最小限の値(シンボルの`TRACK_M`は画面サイズ固定の図なので大きい)。
pub(crate) const GROUND_LIFT_M: f64 = 2.0;
/// 所属の色をモデルの色へ混ぜる割合。
const AFFILIATION_TINT: f32 = 0.35;
/// モデルからシンボルへ切り替える距離に付けるヒステリシス(モデル表示中は、この倍率まで距離が伸びても切り替えない)。
/// 距離が境目のあたりで揺れたとき、モデルとシンボルが行き来してちらつかないようにする。
const SWITCH_HYSTERESIS: f32 = 1.1;
/// 「最小画面サイズ」で大きくする倍率の上限(極端に遠いとき、巨大すぎるモデルで描画が壊れないように)。
const MAX_MIN_SIZE_SCALE: f32 = 100_000.0;
/// 2D(正射影)の見かけの距離を、透視投影の距離に換算するときの縦の視野角(3Dカメラの既定と同じ50度)。
const ORTHO_EQUIVALENT_FOV_Y_DEG: f32 = 50.0;

/// トラック1つの、モデルを置くための情報(トラックが更新されるたびに作り直す)。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ModelPlacement {
    pub id: TrackId,
    pub kind: SymbolKind,
    /// 所属の色(rgb)。
    pub tint: [f32; 3],
    /// 基準点の位置(ENU座標。地形メッシュの原点基準)。
    pub position: [f32; 3],
    /// その地点の東・北・上(同じENU座標系の成分)。
    pub frame: [[f32; 3]; 3],
    pub heading_deg: f32,
    pub pitch_deg: f32,
    pub roll_deg: f32,
}

/// トラック一覧から、モデルの配置を作る。
pub(crate) fn build_placements(ctx: &BuildContext, entries: &[TrackEntry]) -> Vec<ModelPlacement> {
    entries
        .iter()
        .map(|entry| {
            let t = &entry.track;
            let height = height_of(ctx, t.lat_deg, t.lon_deg, t.altitude, GROUND_LIFT_M);
            let color = t.affiliation.color();
            ModelPlacement {
                id: t.id,
                kind: t.kind,
                tint: [color.r, color.g, color.b],
                position: ctx.mesh_transform.transform(t.lat_deg, t.lon_deg, height),
                frame: ctx.mesh_transform.local_frame(t.lat_deg, t.lon_deg),
                heading_deg: t.heading_deg as f32,
                pitch_deg: t.pitch_deg as f32,
                roll_deg: t.roll_deg as f32,
            }
        })
        .collect()
}

/// 機体座標→ENU座標の回転(地点の東・北・上と、ヘディング・ピッチ・ロールから)。
pub(crate) fn attitude(
    frame: [[f32; 3]; 3],
    heading_deg: f32,
    pitch_deg: f32,
    roll_deg: f32,
) -> Mat3 {
    let local = Mat3::from_rotation_z(-heading_deg.to_radians())
        * Mat3::from_rotation_x(pitch_deg.to_radians())
        * Mat3::from_rotation_y(roll_deg.to_radians());
    Mat3::from_cols(
        Vec3::from(frame[0]),
        Vec3::from(frame[1]),
        Vec3::from(frame[2]),
    ) * local
}

/// モデル1つの変換行列(機体座標→ENU座標)。モデルの前が、機体の前から(上から見て)時計回りに`source.yaw_offset_deg`だけ
/// ずれて作られているとき、その分を打ち消して前へ合わせる。`source.scale`と`extra_scale`(最小画面サイズの拡大)を掛けてから、
/// 向きと位置を与える。
pub(crate) fn instance_matrix(
    placement: &ModelPlacement,
    source: &ModelSource,
    extra_scale: f32,
) -> Mat4 {
    let rotation = attitude(
        placement.frame,
        placement.heading_deg,
        placement.pitch_deg,
        placement.roll_deg,
    ) * Mat3::from_rotation_z(source.yaw_offset_deg.to_radians());
    Mat4::from_translation(Vec3::from(placement.position))
        * Mat4::from_mat3(rotation)
        * Mat4::from_scale(Vec3::splat(source.scale * extra_scale))
}

/// 画面の大きさの見積もりに使う、カメラの情報。
pub(crate) struct ViewMetrics {
    eye: Vec3,
    forward: Vec3,
    viewport_height_px: f32,
    /// 正射影のとき、画面の縦幅が表す長さ(メートル)。透視投影なら`None`。
    ortho_view_height_m: Option<f32>,
    tan_half_fov_y: f32,
}

impl ViewMetrics {
    pub(crate) fn new(camera: &Camera, viewport_height_px: f32) -> Self {
        let (ortho_view_height_m, tan_half_fov_y) = match camera.projection {
            Projection::Perspective { fov_y_radians } => (None, (fov_y_radians * 0.5).tan()),
            Projection::Orthographic { view_height_m } => (
                Some(view_height_m),
                (ORTHO_EQUIVALENT_FOV_Y_DEG.to_radians() * 0.5).tan(),
            ),
        };
        Self {
            eye: camera.eye,
            forward: (camera.target - camera.eye).normalize_or_zero(),
            viewport_height_px: viewport_height_px.max(1.0),
            ortho_view_height_m,
            tan_half_fov_y,
        }
    }

    /// 点`position`のカメラからの奥行き(視線方向の距離。メートル)。正射影は、同じ縦幅が映る透視投影の距離に換算する。
    pub(crate) fn depth_m(&self, position: Vec3) -> f32 {
        match self.ortho_view_height_m {
            Some(height) => height / (2.0 * self.tan_half_fov_y),
            None => (position - self.eye).dot(self.forward),
        }
    }

    /// 奥行き`depth_m`の位置で、1メートルが画面で何pxになるか。
    pub(crate) fn pixels_per_meter(&self, depth_m: f32) -> f32 {
        self.viewport_height_px / (2.0 * depth_m.max(1e-3) * self.tan_half_fov_y)
    }
}

/// 表示方式の設定(`ModelsState`のシグナルの値のコピー)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DisplaySettings {
    pub mode: ModelDisplayMode,
    pub switch_distance_m: f32,
    pub min_screen_px: f32,
}

/// トラック1つをどう描くか。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Representation {
    Symbol,
    /// モデルで描く。`scale`は「最小画面サイズ」で実寸より大きくする倍率(1なら実寸)。
    Model {
        scale: f32,
    },
}

/// モデルで描くかシンボルで描くかを決める。`radius_m`はモデルの実寸の半径(`source.scale`込み)、
/// `was_model`は直前のフレームでモデルだったか(ヒステリシス用)。
pub(crate) fn choose_representation(
    settings: DisplaySettings,
    metrics: &ViewMetrics,
    depth_m: f32,
    radius_m: f32,
    was_model: bool,
) -> Representation {
    // カメラの後ろにある(見えない)ものはシンボルのまま。
    if depth_m <= 0.0 {
        return Representation::Symbol;
    }
    match settings.mode {
        ModelDisplayMode::Off => Representation::Symbol,
        ModelDisplayMode::SwitchToSymbol => {
            let limit =
                settings.switch_distance_m * if was_model { SWITCH_HYSTERESIS } else { 1.0 };
            if depth_m <= limit {
                Representation::Model { scale: 1.0 }
            } else {
                Representation::Symbol
            }
        }
        ModelDisplayMode::MinScreenSize => {
            let size_px = 2.0 * radius_m * metrics.pixels_per_meter(depth_m);
            let scale = (settings.min_screen_px / size_px.max(1e-9)).clamp(1.0, MAX_MIN_SIZE_SCALE);
            Representation::Model { scale }
        }
    }
}

/// このフレームの描画計画。
#[derive(Debug, Default, PartialEq)]
pub(crate) struct ModelPlan {
    /// モデルのURLごとの、描くインスタンス。
    pub instances: HashMap<String, Vec<ModelInstance>>,
    /// モデルで描くトラック(この間、シンボルは描かない)。
    pub shown: HashSet<TrackId>,
}

/// 配置の一覧から、このフレームの描画計画を作る。`radius_of`はURLのモデルの半径(読み込み済みでなければ`None`。
/// その間はシンボルで描く)、`previous`は直前のフレームでモデルだったトラック。
pub(crate) fn plan_models(
    placements: &[ModelPlacement],
    sources: &HashMap<SymbolKind, ModelSource>,
    radius_of: &dyn Fn(&str) -> Option<f32>,
    settings: DisplaySettings,
    metrics: &ViewMetrics,
    previous: &HashSet<TrackId>,
) -> ModelPlan {
    let mut plan = ModelPlan::default();
    if settings.mode == ModelDisplayMode::Off {
        return plan;
    }
    for placement in placements {
        let Some(source) = sources.get(&placement.kind) else {
            continue;
        };
        let Some(radius) = radius_of(&source.url) else {
            continue;
        };
        let depth = metrics.depth_m(Vec3::from(placement.position));
        let radius_m = radius * source.scale;
        let Representation::Model { scale } = choose_representation(
            settings,
            metrics,
            depth,
            radius_m,
            previous.contains(&placement.id),
        ) else {
            continue;
        };
        let [r, g, b] = placement.tint;
        plan.instances
            .entry(source.url.clone())
            .or_default()
            .push(ModelInstance {
                model: instance_matrix(placement, source, scale).to_cols_array_2d(),
                tint: [r, g, b, AFFILIATION_TINT],
            });
        plan.shown.insert(placement.id);
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::camera::{CameraPreset, OrbitCamera, ViewMode};
    use crate::terrain::drawing::Altitude;
    use crate::terrain::geodesy::{Ellipsoid, EnuTransform};
    use crate::terrain::origin::Origin;
    use crate::terrain::tracks::{Affiliation, Track};

    const EAST: [f32; 3] = [1.0, 0.0, 0.0];
    const NORTH: [f32; 3] = [0.0, 1.0, 0.0];
    const UP: [f32; 3] = [0.0, 0.0, 1.0];
    const FLAT: [[f32; 3]; 3] = [EAST, NORTH, UP];

    fn close(a: Vec3, b: [f32; 3]) -> bool {
        (a - Vec3::from(b)).length() < 1e-5
    }

    #[test]
    fn heading_turns_clockwise_from_north() {
        // 機体の前(0,1,0)が、ヘディングの向きを指す。
        let forward = |h| attitude(FLAT, h, 0.0, 0.0) * Vec3::Y;
        assert!(close(forward(0.0), [0.0, 1.0, 0.0]), "北");
        assert!(close(forward(90.0), [1.0, 0.0, 0.0]), "東");
        assert!(close(forward(180.0), [0.0, -1.0, 0.0]), "南");
        // 右(1,0,0)は、進行方向の右手(北向きなら東)。
        assert!(close(
            attitude(FLAT, 0.0, 0.0, 0.0) * Vec3::X,
            [1.0, 0.0, 0.0]
        ));
        assert!(
            close(attitude(FLAT, 90.0, 0.0, 0.0) * Vec3::X, [0.0, -1.0, 0.0]),
            "東向きの右は南"
        );
    }

    #[test]
    fn positive_pitch_raises_the_nose_and_positive_roll_drops_the_right_wing() {
        let nose = attitude(FLAT, 0.0, 30.0, 0.0) * Vec3::Y;
        assert!(nose.z > 0.49 && nose.z < 0.51 && nose.y > 0.8, "{nose}");
        let right_wing = attitude(FLAT, 0.0, 0.0, 30.0) * Vec3::X;
        assert!(right_wing.z < -0.49 && right_wing.z > -0.51, "{right_wing}");
        // ロールは機首の向きを変えない。
        assert!(close(
            attitude(FLAT, 0.0, 0.0, 30.0) * Vec3::Y,
            [0.0, 1.0, 0.0]
        ));
        // 機体の上は、水平飛行なら地点の上。
        assert!(close(attitude(FLAT, 45.0, 0.0, 0.0) * Vec3::Z, UP));
    }

    #[test]
    fn attitude_follows_the_local_frame_of_the_place() {
        // 上が東へ30度かたむいた地点(東=(cos30,0,-sin30)、上=(sin30,0,cos30))の機体の上は、その地点の上を向く。
        let (s, c) = 30f32.to_radians().sin_cos();
        let frame = [[c, 0.0, -s], NORTH, [s, 0.0, c]];
        assert!(close(attitude(frame, 0.0, 0.0, 0.0) * Vec3::Z, [s, 0.0, c]));
        assert!(close(attitude(frame, 0.0, 0.0, 0.0) * Vec3::Y, NORTH));
    }

    fn source(scale: f32, yaw_offset_deg: f32) -> ModelSource {
        ModelSource {
            url: "m.glb".to_string(),
            scale,
            yaw_offset_deg,
        }
    }

    fn placement() -> ModelPlacement {
        ModelPlacement {
            id: 1,
            kind: SymbolKind::Aircraft,
            tint: [1.0, 0.0, 0.0],
            position: [100.0, 200.0, 300.0],
            frame: FLAT,
            heading_deg: 90.0,
            pitch_deg: 0.0,
            roll_deg: 0.0,
        }
    }

    #[test]
    fn instance_matrix_places_scales_and_turns_the_model() {
        let m = instance_matrix(&placement(), &source(2.0, 0.0), 3.0);
        // 基準点は位置へ。機体の前1m→東へ6m(scale 2 x 3)。
        assert!(close(m.transform_point3(Vec3::ZERO), [100.0, 200.0, 300.0]));
        assert!(close(m.transform_point3(Vec3::Y), [106.0, 200.0, 300.0]));
        // モデルの前が機体の右(+x。前から時計回りに90度)に作られているとき、yaw_offset=90でその向きが機体の前へ戻る。
        let offset = instance_matrix(
            &ModelPlacement {
                heading_deg: 0.0,
                ..placement()
            },
            &source(1.0, 90.0),
            1.0,
        );
        assert!(
            close(offset.transform_point3(Vec3::X), [100.0, 201.0, 300.0]),
            "モデルの+x側が機体の前(北)へ"
        );
    }

    fn metrics(mode: ViewMode, distance: f32) -> ViewMetrics {
        let mut orbit = OrbitCamera::preset(CameraPreset::Overview, 0.0);
        orbit.mode = mode;
        orbit.distance = distance;
        orbit.target = Vec3::ZERO;
        ViewMetrics::new(&orbit.to_camera(1.5), 1000.0)
    }

    fn settings(mode: ModelDisplayMode) -> DisplaySettings {
        DisplaySettings {
            mode,
            switch_distance_m: 3000.0,
            min_screen_px: 32.0,
        }
    }

    #[test]
    fn metrics_give_depth_and_pixel_size() {
        let m = metrics(ViewMode::ThreeD, 10_000.0);
        // 注視点(原点)は視点から距離10,000m。縦の視野50度・画面1000px: 1mは 1000 / (2 x 10000 x tan25°) px。
        let depth = m.depth_m(Vec3::ZERO);
        assert!((depth - 10_000.0).abs() < 1.0, "{depth}");
        let px = m.pixels_per_meter(depth);
        assert!(
            (px - 1000.0 / (2.0 * 10_000.0 * 25f32.to_radians().tan())).abs() < 1e-4,
            "{px}"
        );
        // 正射影: 縦幅10,000mが1000px。1mは0.1px。奥行きは透視投影で同じ縦幅が映る距離に換算される。
        let ortho = metrics(ViewMode::TwoD, 10_000.0);
        let depth = ortho.depth_m(Vec3::new(500.0, 0.0, 0.0));
        assert!((ortho.pixels_per_meter(depth) - 0.1).abs() < 1e-5);
    }

    #[test]
    fn switch_mode_uses_models_only_within_the_distance_with_hysteresis() {
        let m = metrics(ViewMode::ThreeD, 1000.0);
        let s = settings(ModelDisplayMode::SwitchToSymbol);
        let choose = |depth, was| choose_representation(s, &m, depth, 10.0, was);
        assert_eq!(choose(2999.0, false), Representation::Model { scale: 1.0 });
        assert_eq!(choose(3001.0, false), Representation::Symbol);
        // モデル表示中は、切替距離の1.1倍までモデルのまま(境目でちらつかない)。
        assert_eq!(choose(3200.0, true), Representation::Model { scale: 1.0 });
        assert_eq!(choose(3400.0, true), Representation::Symbol);
        // カメラの後ろ・モード無効はシンボル。
        assert_eq!(choose(-5.0, false), Representation::Symbol);
        assert_eq!(
            choose_representation(settings(ModelDisplayMode::Off), &m, 10.0, 10.0, true),
            Representation::Symbol
        );
    }

    #[test]
    fn min_screen_size_mode_scales_up_so_the_model_is_at_least_that_big() {
        let m = metrics(ViewMode::ThreeD, 1000.0);
        let s = settings(ModelDisplayMode::MinScreenSize);
        let radius = 10.0;
        // 近くて実寸で十分大きければ、実寸のまま。
        let near = 100.0;
        let size_px = 2.0 * radius * m.pixels_per_meter(near);
        assert!(size_px > 32.0);
        assert_eq!(
            choose_representation(s, &m, near, radius, false),
            Representation::Model { scale: 1.0 }
        );
        // 遠くて小さければ、画面で最小サイズ(32px)になる倍率へ。
        let far = 100_000.0;
        let Representation::Model { scale } = choose_representation(s, &m, far, radius, false)
        else {
            panic!()
        };
        assert!(scale > 1.0);
        let shown_px = 2.0 * radius * scale * m.pixels_per_meter(far);
        assert!((shown_px - 32.0).abs() < 0.01, "shown_px={shown_px}");
        // 倍率には上限がある。
        let Representation::Model { scale } = choose_representation(s, &m, 1e12, radius, false)
        else {
            panic!()
        };
        assert_eq!(scale, MAX_MIN_SIZE_SCALE);
    }

    #[test]
    fn plan_groups_instances_by_model_and_skips_unloaded_or_unassigned_kinds() {
        let m = metrics(ViewMode::ThreeD, 1000.0);
        let near = ModelPlacement {
            position: [0.0, 0.0, 0.0],
            ..placement()
        };
        let far = ModelPlacement {
            id: 2,
            position: [50_000.0, 0.0, 0.0],
            ..placement()
        };
        let ship = ModelPlacement {
            id: 3,
            kind: SymbolKind::Ship,
            ..near.clone()
        };
        let vehicle = ModelPlacement {
            id: 4,
            kind: SymbolKind::Vehicle,
            ..near.clone()
        };
        let sources = HashMap::from([
            (SymbolKind::Aircraft, source(1.0, 0.0)),
            (
                SymbolKind::Ship,
                ModelSource {
                    url: "ship.glb".to_string(),
                    scale: 1.0,
                    yaw_offset_deg: 0.0,
                },
            ),
            (
                SymbolKind::Vehicle,
                ModelSource {
                    url: "vehicle.glb".to_string(),
                    scale: 1.0,
                    yaw_offset_deg: 0.0,
                },
            ),
        ]);
        // 船のモデルは読み込み前(None)、地上車両は読み込み済み。
        let radius_of = |url: &str| (url != "ship.glb").then_some(10.0);
        let plan = plan_models(
            &[near, far, ship, vehicle],
            &sources,
            &radius_of,
            settings(ModelDisplayMode::SwitchToSymbol),
            &m,
            &HashSet::new(),
        );
        assert_eq!(
            plan.shown,
            HashSet::from([1, 4]),
            "遠い機(2)と読み込み前の船(3)はシンボルのまま"
        );
        assert_eq!(plan.instances["m.glb"].len(), 1);
        assert_eq!(plan.instances["vehicle.glb"].len(), 1);
        let tint = plan.instances["m.glb"][0].tint;
        assert_eq!(tint, [1.0, 0.0, 0.0, AFFILIATION_TINT]);
        // モードOffなら何も描かない。
        let off = plan_models(
            &[placement()],
            &sources,
            &radius_of,
            settings(ModelDisplayMode::Off),
            &m,
            &HashSet::new(),
        );
        assert_eq!(off, ModelPlan::default());
    }

    #[test]
    fn placements_use_the_track_position_attitude_and_local_frame() {
        let transform = EnuTransform::new(
            &Origin {
                lat_deg: 35.0,
                lon_deg: 135.0,
            },
            &Ellipsoid::WGS84,
        );
        let ground = |_: f64, _: f64| 100.0;
        let ctx = BuildContext {
            terrain: None,
            mesh_transform: &transform,
            ellipsoid: &Ellipsoid::WGS84,
            ground: &ground,
            viewport_px: (800.0, 600.0),
        };
        let track = |id, altitude| TrackEntry {
            track: Track {
                id,
                kind: SymbolKind::Aircraft,
                affiliation: Affiliation::Hostile,
                label: String::new(),
                lat_deg: 35.0,
                lon_deg: 135.0,
                altitude,
                heading_deg: 45.0,
                speed_mps: 0.0,
                pitch_deg: 5.0,
                roll_deg: -20.0,
            },
            trail: vec![],
        };
        let placements = build_placements(
            &ctx,
            &[
                track(1, Altitude::Msl(1000.0)),
                track(2, Altitude::AboveGround(0.0)),
            ],
        );
        assert!(
            close(Vec3::from(placements[0].position), [0.0, 0.0, 1000.0]),
            "原点の真上1000m"
        );
        assert!(
            (placements[1].position[2] - (100.0 + GROUND_LIFT_M as f32)).abs() < 1e-3,
            "地表+持ち上げ"
        );
        assert_eq!(
            (
                placements[0].heading_deg,
                placements[0].pitch_deg,
                placements[0].roll_deg
            ),
            (45.0, 5.0, -20.0)
        );
        assert_eq!(
            placements[0].tint,
            [
                Affiliation::Hostile.color().r,
                Affiliation::Hostile.color().g,
                Affiliation::Hostile.color().b
            ]
        );
        assert!(close(Vec3::from(placements[0].frame[2]), UP), "原点の上");
    }
}
