//! カメラ(ビュー・射影行列)。DESIGN.md 5.1節。
//!
//! ENU座標系(東=X, 北=Y, 上=Z)はZ-upの右手系。wgpu/glamの一般的な慣習であるY-upとは
//! 軸の意味が異なるが、メッシュの頂点データ自体は変換せず、ここ(ビュー行列側)で
//! up方向として`Vec3::Z`を明示的に渡すことで吸収する(頂点を並べ替えるより単純で、
//! ENUがそもそも右手系なのでlook_at_rh/perspective_rhの右手系前提ともそのまま整合する)。
//!
//! フェーズ10: 自由視点カメラ(ズーム・回転・視点プリセット)。DESIGN.md 5.2節。

use glam::{Mat4, Vec3};

/// レンダラーに渡す、計算済みのカメラ(視点位置・注視点・レンズ設定)。
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    pub fov_y_radians: f32,
    pub aspect: f32,
    pub z_near: f32,
    pub z_far: f32,
}

impl Camera {
    pub fn view_proj_matrix(&self) -> Mat4 {
        // ENU座標系のUp軸(Z)をそのまま「上」として渡す。
        let view = Mat4::look_at_rh(self.eye, self.target, Vec3::Z);
        // wgpuの正規化デバイス座標は深度[0,1](OpenGL流の[-1,1]ではない)なので
        // perspective_rh(wgpu/Vulkan/Metal互換)を使う。perspective_rh_glは使わない。
        let proj = Mat4::perspective_rh(self.fov_y_radians, self.aspect, self.z_near, self.z_far);
        proj * view
    }
}

/// 視点プリセット(DESIGN.md 5.2節: 俯瞰・側面のワンクリック切り替え)。
/// 切り替え後もズーム・回転の自由操作は継続できる(初期アングルの提供に過ぎない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraPreset {
    /// 俯瞰: 斜め上から見下ろす(中央の地図パネルの既定)。
    Overview,
    /// 側面: ほぼ水平から見る(右パネル下部・側面図の既定)。
    Side,
}

const MIN_DISTANCE: f32 = 300.0;
const MAX_DISTANCE: f32 = 400_000.0;
// 真上・真下ぎりぎりまで見えるが、ちょうど90度だとlook_atのup方向と視線が一致し
// 特異点になるため少し余裕を持たせる。
const MIN_PITCH: f32 = -1.5;
const MAX_PITCH: f32 = 1.5;

/// 自由視点カメラの内部状態。ドラッグ(回転)・ホイール(ズーム)で更新し、
/// 都度`to_camera()`で描画用の`Camera`(視点位置・注視点)へ変換する。
/// DESIGN.md 5.2節: 「カメラの状態(位置・角度・ズーム)はLeptosのSignalで保持し、
/// パネルごとに独立させる」— 本構造体自体は素のRust構造体だが、
/// `components/terrain_view.rs`側で各パネルごとに独立したインスタンスとして保持することで
/// この方針を満たす。
#[derive(Debug, Clone, Copy)]
pub struct OrbitCamera {
    /// 注視点(ENU座標)。原点付近を見るため既定は原点(0,0,0)。
    pub target: Vec3,
    /// 注視点からの距離(ズーム)。
    pub distance: f32,
    /// 水平回転角(ラジアン、East軸基準・反時計回り)。
    pub yaw: f32,
    /// 仰角(ラジアン、水平面から上向きが正)。
    pub pitch: f32,
    pub fov_y_radians: f32,
    pub z_near: f32,
    pub z_far: f32,
}

impl OrbitCamera {
    pub fn preset(preset: CameraPreset) -> Self {
        match preset {
            CameraPreset::Overview => Self {
                target: Vec3::ZERO,
                distance: 35_000.0,
                yaw: -std::f32::consts::FRAC_PI_4,
                pitch: 0.6,
                fov_y_radians: 50f32.to_radians(),
                z_near: 10.0,
                z_far: 400_000.0,
            },
            CameraPreset::Side => Self {
                target: Vec3::ZERO,
                distance: 60_000.0,
                yaw: 0.0,
                pitch: 0.08,
                fov_y_radians: 45f32.to_radians(),
                z_near: 10.0,
                z_far: 400_000.0,
            },
        }
    }

    /// ドラッグ操作による水平・垂直回転(オービットカメラ)。
    pub fn orbit(&mut self, delta_yaw: f32, delta_pitch: f32) {
        self.yaw -= delta_yaw;
        self.pitch = (self.pitch + delta_pitch).clamp(MIN_PITCH, MAX_PITCH);
    }

    /// マウスホイール/ピンチによるズーム。`factor`>1で遠ざかる、<1で近づく。
    pub fn zoom(&mut self, factor: f32) {
        self.distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    fn eye(&self) -> Vec3 {
        // 球面座標(distance, yaw, pitch) → ENU直交座標。
        let horizontal = self.distance * self.pitch.cos();
        let offset = Vec3::new(
            horizontal * self.yaw.cos(),
            horizontal * self.yaw.sin(),
            self.distance * self.pitch.sin(),
        );
        self.target + offset
    }

    pub fn to_camera(&self, aspect: f32) -> Camera {
        Camera {
            eye: self.eye(),
            target: self.target,
            fov_y_radians: self.fov_y_radians,
            aspect,
            z_near: self.z_near,
            z_far: self.z_far,
        }
    }
}
