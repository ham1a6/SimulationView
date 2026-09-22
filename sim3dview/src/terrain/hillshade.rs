//! 陰影(ヒルシェード)表示のON/OFF。表示メニューの「陰影表示」から`TerrainView`へ伝えるためのcontext。
//! 陰影は地形の法線と固定の光源(北西・仰角45度)から頂点シェーダーで掛けるので、切り替えても
//! メッシュの作り直しはなく、uniformの値が変わるだけで即座に反映される。

use leptos::prelude::*;

#[derive(Clone, Copy)]
pub struct HillshadeState {
    /// trueなら陰影を付ける。
    pub enabled: RwSignal<bool>,
}

impl HillshadeState {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: RwSignal::new(enabled),
        }
    }
}

impl Default for HillshadeState {
    /// 既定はON(陰影を付ける)。
    fn default() -> Self {
        Self::new(true)
    }
}
