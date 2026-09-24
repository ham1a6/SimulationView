//! 地形データ(metadata.json/tile_index.json/base.bin。細かいレベルのタイルは`ui::terrain_view`が
//! 必要に応じて追加取得する)を全パネルで共有するためのストア。
//! DETAILED_DESIGN.md 6.0・6.5節: 中央の地図・右パネル下部の側面図は「同一の地形データ」に
//! 異なるカメラを適用する構成。フェッチ(起動時のmetadata・タイル索引・ベース約3MB)は1回だけ行い、
//! `terrain_view.rs`が使う各パネルはこのストアから同じデータを参照する。

use std::rc::Rc;

use leptos::prelude::*;

use super::fetch;
use super::loader::TerrainData;

// Rc<TerrainData>はSend/Syncではないため、既定のSyncStorageではなくLocalStorageを使う
// (wasm32-unknown-unknownはシングルスレッドなので安全。ws.rsのWsConnectionと同じ理由)。
#[derive(Clone, Copy)]
pub struct TerrainStore {
    /// `{base_url}/metadata.json`・`tile_index.json`・`base.bin`から取得する
    /// (`terrain::loader`参照)。サーバーのホスト名・ポート・ルートパスは呼び出し側が決める。
    base_url: RwSignal<String>,
    data: RwSignal<Option<Rc<TerrainData>>, LocalStorage>,
    loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
}

impl TerrainStore {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: RwSignal::new(base_url.into()),
            data: RwSignal::new_local(None),
            loading: RwSignal::new(false),
            error: RwSignal::new(None),
        }
    }

    /// リアクティブに読む(データが届いたら依存側のEffectが再実行される)。
    pub fn get(&self) -> Option<Rc<TerrainData>> {
        self.data.get()
    }

    pub fn get_untracked(&self) -> Option<Rc<TerrainData>> {
        self.data.get_untracked()
    }

    /// まだ取得していなければ地形データの取得を開始する。
    /// 中央の地図・側面図の両方から呼ばれるが、取得は1回だけ実行される。
    pub fn ensure_loaded(&self) {
        if self.data.get_untracked().is_some() || self.loading.get_untracked() {
            return;
        }
        self.loading.set(true);
        let this = *self;
        let base_url = self.base_url.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            match fetch::load_terrain(&base_url).await {
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
