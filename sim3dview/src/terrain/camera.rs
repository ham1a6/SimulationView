//! カメラ(ビュー・射影行列)。
//!
//! ENU座標系(東=X, 北=Y, 上=Z)はZ-upの右手系。wgpu/glamの一般的な慣習であるY-upとは
//! 軸の意味が異なるが、メッシュの頂点データ自体は変換せず、ここ(ビュー行列側)で
//! up方向として`Vec3::Z`を明示的に渡すことで吸収する(頂点を並べ替えるより単純で、
//! ENUがそもそも右手系なのでlook_at_rh/perspective_rhの右手系前提ともそのまま整合する。
//! DETAILED_DESIGN.md 6.6節)。
//!
//! フェーズ10: 自由視点カメラ(ズーム・回転・視点プリセット)。BASIC_DESIGN.md 6節フェーズ10。

use glam::camera::rh::{proj::directx, view::look_at_mat4};
use glam::{Mat4, Vec3, Vec4};

/// レンズの種類。3D(自由視点)は透視投影、2D(地図モード)は真上からの正射影を使う
/// (`ViewMode`参照)。
#[derive(Debug, Clone, Copy)]
pub enum Projection {
    Perspective { fov_y_radians: f32 },
    /// `view_height_m`: 画面の縦幅が表すワールド空間上の高さ(メートル)。
    /// ズームレベルに相当する(2Dモードでは`OrbitCamera::distance`をそのまま使う)。
    Orthographic { view_height_m: f32 },
}

/// レンダラーに渡す、計算済みのカメラ(視点位置・注視点・レンズ設定)。
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    /// look_atの上方向。3Dモードは常にENUのUp軸(Z)。2Dモード(真上から見下ろす)では
    /// 視線方向自体がZ軸と平行になり特異点になるため、代わりに北(Y軸)を上として使う
    /// (画面上で北が上になる、通常の地図と同じ向き)。
    pub up: Vec3,
    pub projection: Projection,
    pub aspect: f32,
    pub z_near: f32,
    pub z_far: f32,
}

impl Camera {
    pub fn view_proj_matrix(&self) -> Mat4 {
        let view = look_at_mat4(self.eye, self.target, self.up);
        // wgpuの正規化デバイス座標は深度[0,1](OpenGL流の[-1,1]ではない)なので
        // directx::perspective/orthographic(DirectX/WebGPU互換、深度[0,1])を使う。
        let proj = match self.projection {
            Projection::Perspective { fov_y_radians } => {
                directx::perspective(fov_y_radians, self.aspect, self.z_near, self.z_far)
            }
            Projection::Orthographic { view_height_m } => {
                let half_h = view_height_m * 0.5;
                let half_w = half_h * self.aspect;
                directx::orthographic(-half_w, half_w, -half_h, half_h, self.z_near, self.z_far)
            }
        };
        proj * view
    }

    /// 画面上の点(canvas内のCSSピクセル座標、左上原点)を通る視線をENU座標系のレイ
    /// (origin, direction)として返す。地図上での右クリック→緯度経度変換(`terrain/pick.rs`)に使う。
    pub fn screen_to_ray(&self, x: f32, y: f32, width: f32, height: f32) -> (Vec3, Vec3) {
        let ndc_x = (x / width) * 2.0 - 1.0;
        let ndc_y = 1.0 - (y / height) * 2.0;
        let inv_vp = self.view_proj_matrix().inverse();
        let near = inv_vp * Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
        let far = inv_vp * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
        let near = near.truncate() / near.w;
        let far = far.truncate() / far.w;
        (near, far - near)
    }
}

/// 視点プリセット(BASIC_DESIGN.md 6節フェーズ10)。3Dモードの初期カメラアングルを与える
/// (以後はズーム・回転の自由操作ができる)。当初は俯瞰・側面の2種類をワンクリックで
/// 切り替えるボタンがあったが、「俯瞰ボタンと側面ボタンはいらない」との要望により
/// ボタン自体を削除し(`components/terrain_view.rs`)、`Side`バリアントも不要になったため
/// 削除した。現在は初期表示用の`Overview`のみ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraPreset {
    /// 俯瞰: 斜め上から見下ろす(中央の地図パネルの既定・唯一の初期アングル)。
    Overview,
}

/// メインパネルの表示モード(2D/3D切り替え)。3Dは従来通りの自由視点(透視投影・
/// ドラッグで回転)。2Dは真上からの正射影(地図のように北が上で常に固定、
/// ドラッグは回転ではなく平行移動)で、覆域表示は「指定した海抜高度での探知可能領域」
/// (`terrain::los::compute_coverage_area`)に切り替わる(3Dの半球ドーム表示とは別物)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    ThreeD,
    TwoD,
}

// 現在のheightmapは5°四方(約555km)を2048×2048へダウンサンプリングしており、
// 1グリッドセルが約271m(元のALOS 30mデータよりはまだ粗い)。MIN_DISTANCEまで
// ズームすればグリッドの凹凸が見える程度にはなったが、さらなる解像度向上(LOD化等)は
// 引き続き別タスクの余地があるため、余裕を持って近くまでズームできるようにしておく。
const MIN_DISTANCE: f32 = 100.0;
const MAX_DISTANCE: f32 = 500_000.0;
// z_farはMAX_DISTANCEちょうどだと、カメラが最大までズームアウトした際に地形データの
// 外接矩形(5°四方、対角線で約785km)の遠い側がクリッピングされて見えなくなってしまう
// (カメラ自体がMAX_DISTANCE分target から離れているため、そこからさらに785km先までを
// 描画範囲に含める必要がある)。MAX_DISTANCE + 対角線長に十分な余裕を持たせておく。
const Z_FAR: f32 = 1_500_000.0;
// 真上・真下ぎりぎりまで見えるが、ちょうど90度だとlook_atのup方向と視線が一致し
// 特異点になるため少し余裕を持たせる。
const MIN_PITCH: f32 = -1.5;
const MAX_PITCH: f32 = 1.5;
// 2Dモード(正射影)のカメラ高度。データの最高標高(3710m程度)より十分高く、
// 一定値でよい(正射影なので見た目の大きさはこの高さに依存しない。近接/遠方
// クリップ面の範囲を決めるためだけに使う)。
const ORTHO_EYE_HEIGHT_M: f32 = 400_000.0;

/// 自由視点カメラの内部状態。ドラッグ(回転)・ホイール(ズーム)で更新し、
/// 都度`to_camera()`で描画用の`Camera`(視点位置・注視点)へ変換する。
/// BASIC_DESIGN.md 6節フェーズ10: 「カメラの状態(位置・角度・ズーム)はLeptosのSignalで保持し、
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
    /// 2D/3D表示モード。切り替えボタン(`components/terrain_view.rs`)で直接書き換える。
    pub mode: ViewMode,
}

impl OrbitCamera {
    /// `target_up`: 注視点のENU上座標(メートル)。原点の実際の地表標高を渡すこと
    /// (`Vec3::ZERO`=楕円体高0mを渡すと、原点が高山の斜面にある場合に注視点が
    /// 地表よりずっと下(地中)になってしまい、ズームインした際にカメラが地面に
    /// 埋まって真っ黒になる。`components/terrain_view.rs`参照)。
    pub fn preset(preset: CameraPreset, target_up: f32) -> Self {
        match preset {
            CameraPreset::Overview => Self {
                target: Vec3::new(0.0, 0.0, target_up),
                // データが存在する領域(5°四方、対角線で約785km)全体が画面内に収まる距離
                // (実機で確認して調整した値。MAX_DISTANCE=500,000mとの間に少し余裕を残す)。
                distance: 400_000.0,
                yaw: -std::f32::consts::FRAC_PI_4,
                pitch: 0.6,
                fov_y_radians: 50f32.to_radians(),
                z_near: 1.0,
                z_far: Z_FAR,
                mode: ViewMode::ThreeD,
            },
        }
    }

    /// ドラッグ操作による水平・垂直回転(オービットカメラ、3Dモードのみ)。
    pub fn orbit(&mut self, delta_yaw: f32, delta_pitch: f32) {
        self.yaw -= delta_yaw;
        self.pitch = (self.pitch + delta_pitch).clamp(MIN_PITCH, MAX_PITCH);
    }

    /// ドラッグ操作による平行移動(2Dモードのみ。地図を掴んで動かす操作感にするため、
    /// 注視点はドラッグ方向と反対に動かす)。`delta_east_m`/`delta_north_m`は
    /// ワールド座標(ENU)上の移動量。
    pub fn pan(&mut self, delta_east_m: f32, delta_north_m: f32) {
        self.target.x -= delta_east_m;
        self.target.y -= delta_north_m;
    }

    /// マウスホイール/ピンチによるズーム。`factor`>1で遠ざかる、<1で近づく。
    /// 2Dモードでは`distance`は正射影の画面縦幅(メートル)として使う(`to_camera`参照)。
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
        match self.mode {
            ViewMode::ThreeD => Camera {
                eye: self.eye(),
                target: self.target,
                up: Vec3::Z,
                projection: Projection::Perspective { fov_y_radians: self.fov_y_radians },
                aspect,
                z_near: self.z_near,
                z_far: self.z_far,
            },
            ViewMode::TwoD => Camera {
                // 注視点の真上、固定高度から北を上にして見下ろす(地図と同じ向き)。
                eye: self.target + Vec3::new(0.0, 0.0, ORTHO_EYE_HEIGHT_M),
                target: self.target,
                up: Vec3::Y,
                projection: Projection::Orthographic { view_height_m: self.distance },
                aspect,
                z_near: 1.0,
                // 地形の標高差(最大でも数千m)に対しORTHO_EYE_HEIGHT_Mは十分大きいが、
                // 覆域表示の高さバイアス等も含め余裕を持たせる。
                z_far: ORTHO_EYE_HEIGHT_M + 50_000.0,
            },
        }
    }
}
