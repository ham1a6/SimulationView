//! 表示メニュー「中心点を原点に戻す」ボタンから`TerrainView`へ通知するためのcontext。
//! カメラの中心点(注視点)だけを原点位置へ戻し、シミュレーション原点(`OriginState`)は変更しない。
//! 現在の状態ではなく「押された」という単発イベントを運ぶため、単調増加する要求カウンタにしてある。

use leptos::prelude::*;

#[derive(Clone, Copy)]
pub struct RecenterRequestState(pub RwSignal<u32>);

impl RecenterRequestState {
    pub fn new() -> Self {
        Self(RwSignal::new(0))
    }

    pub fn request(&self) {
        self.0.update(|v| *v = v.wrapping_add(1));
    }
}

impl Default for RecenterRequestState {
    fn default() -> Self {
        Self::new()
    }
}
