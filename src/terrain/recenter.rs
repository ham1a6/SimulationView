//! カメラの中心点(注視点)の移動を`TerrainView`へ通知するためのcontext。
//! - 表示メニュー「中心点を原点に戻す」: `request()`。中心点を原点の位置へ戻す。
//! - 右クリックメニュー「ここを中心点にする」等: `request_at(lat, lon)`。中心点をその地点へ移す。
//!
//! どちらもカメラの中心点だけを動かし、シミュレーション原点(`OriginState`)は変更しない。
//! 現在の状態ではなく「押された」という単発イベントを運ぶため、単調増加する要求カウンタにしてある。

use leptos::prelude::*;

#[derive(Clone, Copy)]
pub struct RecenterRequestState {
    /// 要求のたびに増える(`TerrainView`はこの変化を見て中心点を動かす)。初期値0は「まだ要求なし」。
    pub count: RwSignal<u32>,
    /// 直近の要求の移動先(緯度, 経度)。`None`なら原点。
    target: RwSignal<Option<(f64, f64)>>,
}

impl RecenterRequestState {
    pub fn new() -> Self {
        Self {
            count: RwSignal::new(0),
            target: RwSignal::new(None),
        }
    }

    /// 中心点を原点の位置へ戻す。
    pub fn request(&self) {
        self.target.set(None);
        self.bump();
    }

    /// 中心点を、緯度経度の地点(その地表の高さ)へ移す。
    pub fn request_at(&self, lat_deg: f64, lon_deg: f64) {
        self.target.set(Some((lat_deg, lon_deg)));
        self.bump();
    }

    /// 直近の要求の移動先(`None`なら原点)。
    pub fn target(&self) -> Option<(f64, f64)> {
        self.target.get_untracked()
    }

    fn bump(&self) {
        self.count.update(|v| *v = v.wrapping_add(1));
    }
}

impl Default for RecenterRequestState {
    fn default() -> Self {
        Self::new()
    }
}
