//! 「地図をクリックして原点を指定する」モードの状態。メニュー等(アプリ側)が`active`を立てると、
//! `ui::terrain_view::TerrainView`が次の左クリック(ドラッグではない単発クリック)の地点の
//! 緯度経度を求めて`on_pick`を呼び、モードを自動で解除する。
//!
//! 実際に原点をどう変更するか(通信プロトコル)はライブラリの関心事ではないため、
//! `on_pick`は呼び出し側(アプリ)が渡す。`origin_dialog::OriginDialog`の`on_submit`と同じ考え方。

use leptos::prelude::*;

#[derive(Clone, Copy)]
pub struct OriginPickState {
    /// trueの間、地図の左クリックが原点指定として扱われる。
    pub active: RwSignal<bool>,
    /// クリックされた地点の(緯度, 経度)を受け取る。
    pub on_pick: UnsyncCallback<(f64, f64)>,
}

impl OriginPickState {
    pub fn new(on_pick: UnsyncCallback<(f64, f64)>) -> Self {
        Self {
            active: RwSignal::new(false),
            on_pick,
        }
    }
}
