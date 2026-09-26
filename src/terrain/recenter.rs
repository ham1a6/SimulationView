//! カメラの中心点(注視点)の移動を`TerrainView`へ通知するためのcontext。
//! - 表示メニュー「中心点を原点に戻す」: `request()`。中心点を原点の位置へ戻す。
//! - 右クリックメニュー「ここを中心点にする」等: `request_at(lat, lon)`。中心点をその地点へ移す。
//!
//! どちらもカメラの中心点だけを動かし、シミュレーション原点(`OriginState`)は変更しない。
//! 現在の状態ではなく「押された」という単発イベントを運ぶため、単調増加する要求カウンタにしてある。

use leptos::prelude::*;

/// 中心点(注視点)の移動要求を運ぶcontext。
#[derive(Clone, Copy, Default)]
pub struct RecenterRequestState {
    /// 要求のたびに増える(`TerrainView`はこの変化を見て中心点を動かす)。初期値0は「まだ要求なし」。
    pub count: RwSignal<u32>,
    /// 直近の要求の移動先(緯度, 経度)。`None`なら原点。
    target: RwSignal<Option<(f64, f64)>>,
}

impl RecenterRequestState {
    /// 要求なし(カウンタ0)の状態で作る。
    pub fn new() -> Self {
        Self::default()
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

    /// 直近の要求の移動先(`None`なら原点)。購読はしない(変化の通知は`count`で受け取る)。
    pub fn target(&self) -> Option<(f64, f64)> {
        self.target.get_untracked()
    }

    /// 要求カウンタを1進める。同じ移動先を続けて要求しても値が変わるので、毎回反応させられる
    /// (u32を使い切ったら0へ戻るが、0を「要求なし」と区別する必要があるのは初期状態だけ)。
    fn bump(&self) {
        self.count.update(|v| *v = v.wrapping_add(1));
    }
}
