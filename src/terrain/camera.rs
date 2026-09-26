//! カメラ(ビュー・射影行列)。
//!
//! ENU座標系(東=X, 北=Y, 上=Z)はZ-upの右手系。wgpu/glamの一般的な慣習であるY-upとは
//! 軸の意味が異なるが、メッシュの頂点データ自体は変換せず、ここ(ビュー行列側)で
//! up方向として`Vec3::Z`を明示的に渡すことで吸収する(頂点を並べ替えるより単純で、
//! ENUがそもそも右手系なのでlook_at_rh/perspective_rhの右手系前提ともそのまま整合する。
//! DETAILED_DESIGN.md 6.6節)。
//!
//! 自由視点カメラ(ズーム・回転・視点プリセット)。DETAILED_DESIGN.md 6.6・9.6節。

use glam::camera::rh::{proj::directx, view::look_at_mat4};
use glam::{Mat4, Vec3};

/// レンズの種類。3D(自由視点)は透視投影、2D(地図モード)は真上からの正射影を使う
/// (`ViewMode`参照)。
#[derive(Debug, Clone, Copy)]
pub enum Projection {
    Perspective {
        fov_y_radians: f32,
    },
    /// `view_height_m`: 画面の縦幅が表すワールド空間上の高さ(メートル)。
    /// ズームレベルに相当する(2Dモードでは`OrbitCamera::distance`をそのまま使う)。
    Orthographic {
        view_height_m: f32,
    },
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
    /// 視線の基底(前・右・上)。`look_at`と同じ右手系で、`water_ray_basis`と`screen_to_ray`が
    /// 同じ式でレイを求めるための共通部分。
    fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let forward = (self.target - self.eye).normalize();
        let right = forward.cross(self.up).normalize();
        let up = right.cross(forward);
        (forward, right, up)
    }

    pub fn view_proj_matrix(&self) -> Mat4 {
        let view = look_at_mat4(self.eye, self.target, self.up);
        self.projection_matrix() * view
    }

    /// 射影行列だけ(ビュー行列を掛けない)。カメラ固定の作図(`terrain::drawing`の視点空間)は、
    /// カメラから見た座標(右・上・-前方)をそのまま射影するのでこれを使う。
    pub fn projection_matrix(&self) -> Mat4 {
        // wgpuの正規化デバイス座標は深度[0,1](OpenGL流の[-1,1]ではない)なので
        // directx::perspective/orthographic(DirectX/WebGPU互換、深度[0,1])を使う。
        //
        // **反転Z(reversed-Z)を採用**(near→深度1, far→深度0。renderer.rsのdepth_compareも
        // Greaterに揃えてある): z_near=1m・z_far=8,000,000m(`Z_FAR`)という非常に広いレンジを
        // 通常の(near→0, far→1の)深度バッファで扱うと、遠方(見た目上はほとんどの地形が
        // 該当)でdepth値の実効精度がほぼ失われ、地形の行ごとにデプステストの勝敗が
        // 不安定になる(カメラ操作のたびに結果が変わる) z-fighting(「地表面で所々透けている
        // /カメラ操作時に描画が安定しない」)の原因になっていた。Depth32Float+反転Zの組み合わせは
        // この種の広域(惑星規模)地形描画における標準的な対策(浮動小数点は0付近ほど密に
        // 値を表現できるため、遠方をdepth=0付近に割り当てる反転Zの方が実効精度を稼げる)。
        //
        // 透視投影は当初`perspective_infinite_reverse`(far=無限遠)を使っていたが、
        // 「原点から遠いところで地表面の描画が省略される(zoomに依存せず、原点から遠い
        // ほど発生)」という不具合が報告された。無限遠射影はclip.z成分が(view座標のzに
        // 依存しない)定数near値になる特殊な行列形状になり(実際に手計算で確認済み)、
        // これ自体は数学的には正しいが、通常のクリップ行列とは異なる不慣れな形であるため、
        // 環境によってはGPU/ドライバのクリッピング処理が想定外の挙動をする可能性を排除
        // できなかった。より保守的な、near/farとも有限の反転Z(`directx::perspective`に
        // near/farを入れ替えて渡すだけで反転Zになる。正射影と同じトリックが透視投影でも
        // 成り立つことを手計算で確認済み)に変更した。z_farは3Dでは`Z_FAR`(最大ズームアウトの
        // 距離+データ範囲の対角線に余裕を持たせた値)、2Dでは`ORTHO_EYE_HEIGHT_M`+`ORTHO_DEPTH_RANGE_M`。
        match self.projection {
            Projection::Perspective { fov_y_radians } => {
                directx::perspective(fov_y_radians, self.aspect, self.z_far, self.z_near)
            }
            Projection::Orthographic { .. } => {
                let (half_w, half_h) = self.half_extents();
                // 正射影も深度がzに対して線形なので、near/farを入れ替えて渡すだけで
                // 深度マッピングが反転する(near→1, far→0)。
                directx::orthographic(-half_w, half_w, -half_h, half_h, self.z_far, self.z_near)
            }
        }
    }

    /// 画面の中心から右端・上端までの広がり(横, 縦)。透視投影は視線方向の距離1あたり、正射影はメートル。
    fn half_extents(&self) -> (f32, f32) {
        let half_h = match self.projection {
            Projection::Perspective { fov_y_radians } => (fov_y_radians * 0.5).tan(),
            Projection::Orthographic { view_height_m } => view_height_m * 0.5,
        };
        (half_h * self.aspect, half_h)
    }

    /// 水域レイヤー(`terrain.wgsl`の`fs_water`)が、各画素(正規化デバイス座標ndc_x,ndc_y)の視線を
    /// 求めるための値: [視点(w=1なら透視投影、0なら正射影), 視線方向, 右(画面端までの長さ倍),
    /// 上(同)]。視線は`screen_to_ray`と同じ式で、透視投影は原点=視点・向き=前+右*ndc_x+上*ndc_y、
    /// 正射影は原点=視点+右*ndc_x+上*ndc_y・向き=前。逆VP行列で求めないのは、`screen_to_ray`の
    /// コメントにある通りf32の丸め誤差で向きが大きくずれるため。
    pub fn water_ray_basis(&self) -> [[f32; 4]; 4] {
        let (forward, right, up) = self.basis();
        let (half_w, half_h) = self.half_extents();
        let perspective = match self.projection {
            Projection::Perspective { .. } => 1.0,
            Projection::Orthographic { .. } => 0.0,
        };
        let (right, up) = (right * half_w, up * half_h);
        [
            [self.eye.x, self.eye.y, self.eye.z, perspective],
            [forward.x, forward.y, forward.z, 0.0],
            [right.x, right.y, right.z, 0.0],
            [up.x, up.y, up.z, 0.0],
        ]
    }

    /// 画面上の点(canvas内のCSSピクセル座標、左上原点)を通る視線をENU座標系のレイ
    /// (origin, direction)として返す。地図上での右クリック→緯度経度変換(`terrain/pick.rs`)に使う。
    pub fn screen_to_ray(&self, x: f32, y: f32, width: f32, height: f32) -> (Vec3, Vec3) {
        let ndc_x = (x / width) * 2.0 - 1.0;
        let ndc_y = 1.0 - (y / height) * 2.0;
        // カメラの基底(look_atと同じ右手系)から直接レイを求める。以前は逆VP行列でnear点と
        // 中間点(反転Zでnearの約2倍)を逆変換してその差を向きにしていたが、この2点の差は
        // 約1mしかなく、カメラが数百km〜2,000km離れるとf32の丸め誤差(0.1m超)で向きが
        // 大きくずれ、ズームアウト時にクリック位置と別の地点を拾う不具合になっていた。
        let (forward, right, up) = self.basis();
        let (half_w, half_h) = self.half_extents();
        let (dx, dy) = (right * (ndc_x * half_w), up * (ndc_y * half_h));
        match self.projection {
            Projection::Perspective { .. } => (self.eye, forward + dx + dy),
            Projection::Orthographic { .. } => (self.eye + dx + dy, forward),
        }
    }
}

/// 視点プリセット(DETAILED_DESIGN.md 6.6節)。3Dモードの初期カメラアングルを与える
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

// 地形は1度タイル・チャンク単位のLODで、近づくほど細かいレベル(最細は元のALOSデータと同じ
// 約30m/セル)に切り替わる(`terrain::lod`)。これより近づいても凹凸は増えない。
const MIN_DISTANCE: f32 = 100.0;
// 2,000,000mまでズームアウトできる(初期距離400,000mの5倍。データ全体は30°四方で
// 約3,300kmあるので、この距離でも一度に見える範囲はその一部)。
const MAX_DISTANCE: f32 = 2_000_000.0;
// z_farはMAX_DISTANCEちょうどだと、カメラが最大までズームアウトした際に地形データの
// 外接矩形の遠い側がクリッピングされて見えなくなってしまう(カメラ自体がMAX_DISTANCE分
// targetから離れているため、そこからさらに対角線長(現在のデータは30°四方で約4,000km)
// 先までを描画範囲に含める必要がある)。MAX_DISTANCE + 対角線長に余裕を持たせた値。
const Z_FAR: f32 = 8_000_000.0;
// 真上・真下ぎりぎりまで見えるが、ちょうど90度だとlook_atのup方向と視線が一致し
// 特異点になるため少し余裕を持たせる。
const MIN_PITCH: f32 = -1.5;
const MAX_PITCH: f32 = 1.5;
/// 3Dモードで、視点(カメラ位置)が真下の地面(地形の表面、海・データ範囲外は海抜0m)から最低限
/// 離れる高さ(メートル)。最細の地形のセル(約30m)と同程度にして、視点が近くの斜面の三角形に
/// めり込んで画面が地形の内側の色で埋まらないようにする。
pub const MIN_EYE_CLEARANCE_M: f32 = 30.0;
// 2Dモード(正射影)のカメラ高度。データの最高標高(3710m程度)より十分高く、
// 一定値でよい(正射影なので見た目の大きさはこの高さに依存しない。近接/遠方
// クリップ面の範囲を決めるためだけに使う)。
const ORTHO_EYE_HEIGHT_M: f32 = 100_000.0;
// 2Dモードの奥行き方向の描画範囲(カメラ高度から下方向、メートル)。地形は地球の丸みで
// 原点から遠いほど下がる(原点から1,000kmで約80km、データの端の約3,300kmで約850km)。
// これが足りないと、原点から離れた地域(九州・朝鮮半島など)が奥行き方向のクリッピングで
// 描画されなくなる(以前は450kmで、原点から約1,000km以上離れた地域が消えていた)。
const ORTHO_DEPTH_RANGE_M: f32 = 1_200_000.0;

/// 自由視点カメラの内部状態。ドラッグ(回転)・ホイール(ズーム)で更新し、
/// 都度`to_camera()`で描画用の`Camera`(視点位置・注視点)へ変換する。
/// DETAILED_DESIGN.md 6.5〜6.6節: 「カメラの状態(位置・角度・ズーム)はLeptosのSignalで保持し、
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
                // 原点まわり(5°四方ほど、対角線で約785km)が画面内に収まる距離(実機で確認して
                // 調整した値。地形データ全体は30°四方でこれより広く、ここからさらに
                // MAX_DISTANCE=2,000,000mまでズームアウトできる)。
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

    /// 3Dモードで、Shift+ドラッグにより注視点(中心点)を水平面上で平行移動する
    /// (通常のドラッグは`orbit`による回転)。画面の右方向・奥行き方向をカメラの
    /// yaw基準で求め、`eye()`のoffset計算と同じ基底(水平面上、yaw基準)を使うことで、
    /// yawが0とは限らない自由視点カメラでも「掴んで動かす」操作感になるようにしてある
    /// (2Dモードの`pan()`は常に北=画面上で固定なので、この変換は不要)。
    /// `x`/`y`だけを動かし、標高(`target.z`)は呼び出し側(`components/terrain_view.rs`)が
    /// 移動先の実際の地表標高へ更新すること(camera.rs自体はheightmapを知らないため)。
    /// シミュレーション原点(`terrain::origin::OriginState`)・地形メッシュには一切触れない。
    pub fn pan_orbit_target(&mut self, dx_px: f32, dy_px: f32, canvas_height_px: f32) {
        let world_per_px =
            2.0 * self.distance * (self.fov_y_radians * 0.5).tan() / canvas_height_px.max(1.0);
        let delta_right = dx_px * world_per_px;
        let delta_forward = -dy_px * world_per_px;
        let right = Vec3::new(-self.yaw.sin(), self.yaw.cos(), 0.0);
        let forward_h = Vec3::new(-self.yaw.cos(), -self.yaw.sin(), 0.0);
        let delta = right * delta_right + forward_h * delta_forward;
        self.target.x -= delta.x;
        self.target.y -= delta.y;
    }

    /// 3Dモードで、視点が地面より下にもぐらないようにする。`ground_up(east, north)`は、その水平位置の
    /// 地面のENU上座標(地形の表面・地球の丸み込み。海・データ範囲外は海抜0mの面)を返す関数
    /// (camera.rs自体はheightmapを知らないため、呼び出し側(`ui::terrain_view`)が渡す)。
    ///
    /// 視点の真下の地面から`MIN_EYE_CLEARANCE_M`未満なら、まず**距離を保ったまま仰角を上げる**
    /// (ドラッグで下へ回したとき、地面の高さで止まる操作感になる)。仰角は視点の水平位置と一緒に
    /// 変わり、真下の地面の高さも変わるので数回繰り返す。仰角を最大まで上げても届かない場合
    /// (ズームインで距離が近すぎる・注視点の周りが高い地形など)は、視点を真上へ持ち上げ、
    /// 注視点との距離と仰角をそこから求め直す(水平位置は変えない)。
    /// 仰角は水平より下向き(注視点を見上げる)にもなりうる。谷底から高い所を見上げるのは地面の
    /// 上なので許す。2Dモード(正射影)は視点の高さが見た目に関係しないので何もしない。
    pub fn keep_above_ground(&mut self, ground_up: impl Fn(f32, f32) -> f32) {
        if self.mode != ViewMode::ThreeD {
            return;
        }
        for _ in 0..6 {
            let eye = self.eye();
            let min_z = ground_up(eye.x, eye.y) + MIN_EYE_CLEARANCE_M;
            if eye.z >= min_z {
                return;
            }
            let sin_pitch = (min_z - self.target.z) / self.distance;
            if sin_pitch >= MAX_PITCH.sin() {
                break; // 仰角を上げても届かない。
            }
            self.pitch = sin_pitch.asin().max(self.pitch).clamp(MIN_PITCH, MAX_PITCH);
        }
        let eye = self.eye();
        let min_z = ground_up(eye.x, eye.y) + MIN_EYE_CLEARANCE_M;
        if eye.z < min_z {
            let offset = Vec3::new(
                eye.x - self.target.x,
                eye.y - self.target.y,
                min_z - self.target.z,
            );
            let distance = offset.length();
            self.pitch = (offset.z / distance).asin().clamp(MIN_PITCH, MAX_PITCH);
            self.distance = distance.clamp(MIN_DISTANCE, MAX_DISTANCE);
        }
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
                projection: Projection::Perspective {
                    fov_y_radians: self.fov_y_radians,
                },
                aspect,
                z_near: self.z_near,
                z_far: self.z_far,
            },
            ViewMode::TwoD => Camera {
                // 注視点の真上、固定高度から北を上にして見下ろす(地図と同じ向き)。
                eye: self.target + Vec3::new(0.0, 0.0, ORTHO_EYE_HEIGHT_M),
                target: self.target,
                up: Vec3::Y,
                projection: Projection::Orthographic {
                    view_height_m: self.distance,
                },
                aspect,
                z_near: 1.0,
                z_far: ORTHO_EYE_HEIGHT_M + ORTHO_DEPTH_RANGE_M,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(mode: ViewMode, distance: f32) -> Camera {
        let mut o = OrbitCamera::preset(CameraPreset::Overview, 0.0);
        o.mode = mode;
        o.distance = distance;
        o.to_camera(1.0)
    }

    /// 視点から視線方向へ`dist`進んだ点の、クリップ空間での深度(z/w)。
    fn depth_at(c: &Camera, dist: f32) -> f32 {
        let forward = (c.target - c.eye).normalize();
        let clip = c.view_proj_matrix() * (c.eye + forward * dist).extend(1.0);
        clip.z / clip.w
    }

    // 反転Z: 近い面が深度1、遠い面が深度0(`renderer.rs`の`depth_compare: Greater`・クリア値0.0の前提)。
    #[test]
    fn depth_is_reversed_for_perspective() {
        let c = cam(ViewMode::ThreeD, 400_000.0);
        assert!((depth_at(&c, c.z_near) - 1.0).abs() < 1e-3);
        assert!(depth_at(&c, c.z_far).abs() < 1e-3);
        // 遠いほど小さく、遠方でも(f32の)値が潰れず単調に減る。
        let depths: Vec<f32> = [10.0, 100.0, 1e4, 1e5, 1e6, 4e6]
            .iter()
            .map(|&d| depth_at(&c, d))
            .collect();
        assert!(depths.windows(2).all(|w| w[0] > w[1]), "{depths:?}");
    }

    #[test]
    fn depth_is_reversed_for_orthographic() {
        let c = cam(ViewMode::TwoD, 100_000.0);
        assert!((depth_at(&c, c.z_near) - 1.0).abs() < 1e-4);
        assert!(depth_at(&c, c.z_far).abs() < 1e-4);
        // 正射影は深度が距離に対して線形。
        let mid = depth_at(&c, 0.5 * (c.z_near + c.z_far));
        assert!((mid - 0.5).abs() < 1e-3, "{mid}");
    }

    // `screen_to_ray`のレイ上の点を`view_proj_matrix`で射影し直すと、元の画面座標(NDC)に戻る。
    // ピッキング(`pick.rs`)と描画の座標系がずれていないことの確認。
    #[test]
    fn screen_to_ray_is_consistent_with_the_projection() {
        let pixels = [
            (0.0, 0.0),
            (350.0, 350.0),
            (700.0, 700.0),
            (123.0, 456.0),
            (600.0, 50.0),
        ];
        for c in [
            cam(ViewMode::ThreeD, 30_000.0),
            cam(ViewMode::ThreeD, 400_000.0),
            cam(ViewMode::ThreeD, 2_000_000.0),
            cam(ViewMode::TwoD, 100_000.0),
        ] {
            let vp = c.view_proj_matrix();
            for &(x, y) in &pixels {
                let (origin, dir) = c.screen_to_ray(x, y, 700.0, 700.0);
                let clip = vp * (origin + dir * 1000.0).extend(1.0);
                let (ndc_x, ndc_y) = (clip.x / clip.w, clip.y / clip.w);
                let (want_x, want_y) = (x / 700.0 * 2.0 - 1.0, 1.0 - y / 700.0 * 2.0);
                assert!(
                    (ndc_x - want_x).abs() < 2e-3,
                    "({x},{y}): {ndc_x} vs {want_x}"
                );
                assert!(
                    (ndc_y - want_y).abs() < 2e-3,
                    "({x},{y}): {ndc_y} vs {want_y}"
                );
            }
            // 画面中央のレイは視線方向(正射影なら視点が中心から動かない)。
            let (origin, dir) = c.screen_to_ray(350.0, 350.0, 700.0, 700.0);
            let forward = (c.target - c.eye).normalize();
            assert!(dir.normalize().dot(forward) > 0.9999);
            assert!((origin - c.eye).length() < 1e-2 * c.eye.length().max(1.0));
        }
    }

    #[test]
    fn water_ray_basis_matches_screen_to_ray() {
        for c in [
            cam(ViewMode::ThreeD, 400_000.0),
            cam(ViewMode::TwoD, 100_000.0),
        ] {
            let [eye, forward, right, up] = c.water_ray_basis();
            let (eye, forward, right, up) = (
                Vec3::from_slice(&eye[..3]),
                Vec3::from_slice(&forward[..3]),
                Vec3::from_slice(&right[..3]),
                Vec3::from_slice(&up[..3]),
            );
            let perspective = c.water_ray_basis()[0][3] > 0.5;
            let (ndc_x, ndc_y) = (0.5_f32, -0.25_f32);
            let (origin, dir) = c.screen_to_ray(
                (ndc_x + 1.0) * 0.5 * 700.0,
                (1.0 - ndc_y) * 0.5 * 700.0,
                700.0,
                700.0,
            );
            let (rebuilt_origin, rebuilt_dir) = if perspective {
                (eye, forward + right * ndc_x + up * ndc_y)
            } else {
                (eye + right * ndc_x + up * ndc_y, forward)
            };
            assert!(
                (dir - rebuilt_dir).length() < 1e-4 * dir.length().max(1.0),
                "{dir} vs {rebuilt_dir}"
            );
            assert!((origin - rebuilt_origin).length() < 1e-3 * origin.length().max(1.0));
        }
    }

    #[test]
    fn orbit_zoom_and_pan_clamp_and_move_as_documented() {
        let mut o = OrbitCamera::preset(CameraPreset::Overview, 12.0);
        assert_eq!(o.target, Vec3::new(0.0, 0.0, 12.0));
        assert_eq!(o.mode, ViewMode::ThreeD);

        o.orbit(0.1, 100.0);
        assert!((o.yaw - (-std::f32::consts::FRAC_PI_4 - 0.1)).abs() < 1e-6);
        assert_eq!(o.pitch, MAX_PITCH);
        o.orbit(0.0, -100.0);
        assert_eq!(o.pitch, MIN_PITCH);

        o.zoom(1e-9);
        assert_eq!(o.distance, MIN_DISTANCE);
        o.zoom(1e12);
        assert_eq!(o.distance, MAX_DISTANCE);

        // 2Dのpanは、掴んで動かす向き(注視点は移動量と反対)。
        o.pan(100.0, -50.0);
        assert_eq!((o.target.x, o.target.y), (-100.0, 50.0 + 0.0));
    }

    #[test]
    fn pan_orbit_target_moves_along_the_screen_axes() {
        let mut o = OrbitCamera::preset(CameraPreset::Overview, 0.0);
        o.yaw = -std::f32::consts::FRAC_PI_2; // 視点は南側にあり、画面の右=東
        o.distance = 10_000.0;
        let per_px = 2.0 * o.distance * (o.fov_y_radians * 0.5).tan() / 1000.0;
        o.pan_orbit_target(10.0, 0.0, 1000.0);
        // 右へ掴んで動かす→注視点は西へ。
        assert!((o.target.x + 10.0 * per_px).abs() < 1e-2, "{:?}", o.target);
        assert!(o.target.y.abs() < 1e-2);
    }

    #[test]
    fn eye_is_kept_above_the_ground() {
        // 平らな地面(z=0)。仰角が負(視点が地面の下)でも、30m上まで持ち上げる。
        let mut o = OrbitCamera::preset(CameraPreset::Overview, 0.0);
        o.distance = 1000.0;
        o.pitch = -1.0;
        o.keep_above_ground(|_, _| 0.0);
        assert!(o.eye().z >= MIN_EYE_CLEARANCE_M - 1e-2, "{}", o.eye().z);
        // 距離は保ったまま仰角を上げる(ドラッグで下へ回したとき、地面の高さで止まる操作感)。
        assert!((o.distance - 1000.0).abs() < 1.0);

        // 地面が十分高い所(注視点の周りが高地)では、仰角を最大にしても届かないので視点を持ち上げる。
        let mut o = OrbitCamera::preset(CameraPreset::Overview, 0.0);
        o.distance = 300.0;
        o.pitch = 0.5;
        o.keep_above_ground(|_, _| 500.0);
        assert!(
            o.eye().z >= 500.0 + MIN_EYE_CLEARANCE_M - 1e-1,
            "{}",
            o.eye().z
        );
        assert!(o.pitch <= MAX_PITCH);

        // すでに十分上にあれば何も変えない。2Dモードは視点の高さが無関係なので何もしない。
        let before = OrbitCamera::preset(CameraPreset::Overview, 0.0);
        let mut o = before;
        o.keep_above_ground(|_, _| 0.0);
        assert_eq!((o.pitch, o.distance), (before.pitch, before.distance));
        let mut o2d = OrbitCamera {
            mode: ViewMode::TwoD,
            pitch: -1.0,
            ..before
        };
        o2d.keep_above_ground(|_, _| 1e9);
        assert_eq!(o2d.pitch, -1.0);
    }

    #[test]
    fn two_d_camera_looks_straight_down_with_north_up() {
        let c = cam(ViewMode::TwoD, 50_000.0);
        assert_eq!(c.up, Vec3::Y);
        assert!(
            matches!(c.projection, Projection::Orthographic { view_height_m } if view_height_m == 50_000.0)
        );
        assert!((c.target - c.eye).normalize().dot(-Vec3::Z) > 0.9999);
        assert_eq!(c.z_far, ORTHO_EYE_HEIGHT_M + ORTHO_DEPTH_RANGE_M);
    }
}
