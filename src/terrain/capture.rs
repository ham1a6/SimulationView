//! マップパネル(`ui::terrain_view::TerrainView`)のスクリーンショット保存・画面録画を
//! 要求するためのcontext。他の「要求を運ぶだけ」のcontext(`recenter::RecenterRequestState`)と
//! 同じパターンで、実際にcanvasからPNG/WebMを作ってダウンロードする処理は`TerrainView`側が持つ
//! (`ui::terrain_view::capture`、非公開)。ボタンをどこに置くかはこのライブラリの関知しない
//! アプリ固有のUIなので、呼び出し側が置く(`sample/sim_frontend`ではVABパネルに置いている)。

use leptos::prelude::*;

#[derive(Clone, Copy, Default)]
pub struct CaptureState {
    /// スクリーンショット要求のたびに増える(`TerrainView`はこの変化を見てPNGを保存する)。
    /// 初期値0は「まだ要求なし」を表す(`recenter::RecenterRequestState`と同じ約束)。
    pub screenshot_requests: RwSignal<u32>,
    /// 画面録画の開始/停止の要求(trueにすると`TerrainView`が録画を開始し、falseに戻すと
    /// 停止してWebMとして保存する)。
    pub recording_requested: RwSignal<bool>,
    /// 実際に録画中かどうか(`TerrainView`が録画を開始/停止できたときだけ更新する。
    /// ブラウザがMediaRecorder等に未対応で開始に失敗した場合は`recording_requested`を
    /// falseへ戻し、これもfalseのままになる)。ボタンの表示切り替えに使う想定。
    pub is_recording: RwSignal<bool>,
}

impl CaptureState {
    pub fn new() -> Self {
        Self::default()
    }

    /// スクリーンショットを1枚保存する。
    pub fn request_screenshot(&self) {
        self.screenshot_requests.update(|v| *v = v.wrapping_add(1));
    }

    /// 画面録画の開始/停止を切り替える。
    pub fn toggle_recording(&self) {
        self.recording_requested.update(|on| *on = !*on);
    }
}
