//! 地形データ(metadata.json/tile_index.json/base.bin。細かいレベルのタイルは`ui::terrain_view`が
//! 必要に応じて追加取得する)を全パネルで共有するためのストア。
//! DETAILED_DESIGN.md 6.0・6.5節: 中央の地図・右パネル下部の側面図は「同一の地形データ」に
//! 異なるカメラを適用する構成。フェッチ(起動時のmetadata・タイル索引・ベース約3MB)は1回だけ行い、
//! 各パネル(`ui::terrain_view`の地図・見通し図・断面図など)はこのストアから同じデータを参照する。
//!
//! `TerrainStore`自体は`Copy`なシグナルの束で、`viewer::ViewerState`がcontextとして登録する。
//! 取得は最初に`ensure_loaded`を呼んだパネルが始め、結果は`get()`を購読している全パネルへ届く。

use std::rc::Rc;

use leptos::prelude::*;

use super::fetch;
use super::loader::TerrainData;

pub use super::fetch::TerrainLoadProgress;

// Rc<TerrainData>はSend/Syncではないため、既定のSyncStorageではなくLocalStorageを使う
// (wasm32-unknown-unknownはシングルスレッドなので安全。ws.rsのWsConnectionと同じ理由)。
/// 地形データを1回だけ取得して、全パネルで共有するための入れ物。
#[derive(Clone, Copy)]
pub struct TerrainStore {
    /// `{base_url}/metadata.json`・`tile_index.json`・`base.bin`から取得する
    /// (`terrain::loader`参照)。サーバーのホスト名・ポート・ルートパスは呼び出し側が決める。
    base_url: RwSignal<String>,
    /// 取得済みのデータ。取得が終わるまで(または失敗したら)None。
    data: RwSignal<Option<Rc<TerrainData>>, LocalStorage>,
    /// 取得中か(`ensure_loaded`が重ねて呼ばれても、取得を二重に始めないため)。
    loading: RwSignal<bool>,
    /// 起動時の取得の進み具合(読み込み中の表示用)。
    progress: RwSignal<TerrainLoadProgress>,
    /// 取得・検証に失敗したときのエラー文(画面の状態表示に出す)。失敗しても自動では取り直さない
    /// (`ensure_loaded`を再び呼べば取り直すが、このエラー文は消さない)。
    pub error: RwSignal<Option<String>>,
}

impl TerrainStore {
    /// `base_url`(末尾スラッシュなし。例: `"http://localhost:9001/terrain"`)から取得するストアを作る。
    /// 作っただけでは取得を始めない(`ensure_loaded`で始める)。
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: RwSignal::new(base_url.into()),
            data: RwSignal::new_local(None),
            loading: RwSignal::new(false),
            progress: RwSignal::new(TerrainLoadProgress::default()),
            error: RwSignal::new(None),
        }
    }

    /// リアクティブに読む(データが届いたら依存側のEffectが再実行される)。
    pub fn get(&self) -> Option<Rc<TerrainData>> {
        self.data.get()
    }

    /// 購読せずに読む(イベントハンドラーなど、データの到着で再実行したくない場所で使う)。
    pub fn get_untracked(&self) -> Option<Rc<TerrainData>> {
        self.data.get_untracked()
    }

    /// 起動時の取得(`base.bin`の受信バイト数)の進み具合をリアクティブに読む。
    /// 取得が終わったかどうかは`get()`が`Some`かで判断する。
    pub fn progress(&self) -> TerrainLoadProgress {
        self.progress.get()
    }

    /// まだ取得していなければ地形データの取得を開始する。
    /// 中央の地図・側面図の両方から呼ばれるが、取得は1回だけ実行される。
    /// 失敗した後に呼んでも、`data`がNoneで`loading`がfalseなので取得をやり直す。
    pub fn ensure_loaded(&self) {
        if self.data.get_untracked().is_some() || self.loading.get_untracked() {
            return;
        }
        self.loading.set(true);
        // `TerrainStore`はシグナルのハンドルだけを持つ`Copy`なので、非同期タスクへ値ごと移してよい。
        let this = *self;
        let base_url = self.base_url.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let on_progress = move |p| this.progress.set(p);
            match fetch::load_terrain(&base_url, on_progress).await {
                Ok(data) => this.data.set(Some(Rc::new(data))),
                Err(e) => {
                    log::error!("[terrain] {e}");
                    this.error.set(Some(e));
                }
            }
            this.loading.set(false);
        });
    }
}
