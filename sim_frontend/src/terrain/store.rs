//! 地形データ(heightmap.bin/metadata.json)を全パネルで共有するためのストア。
//! DESIGN.md 5.2節: 中央の地図・右パネル下部の側面図は「同一の地形メッシュ」に
//! 異なるカメラを適用する構成。フェッチ(4MBのheightmap.bin)は1回だけ行い、
//! `terrain_view.rs`が使う各パネルはこのストアから同じデータを参照する。

use std::rc::Rc;

use leptos::prelude::*;

use super::loader::{self, TerrainData};

// Rc<TerrainData>はSend/Syncではないため、既定のSyncStorageではなくLocalStorageを使う
// (wasm32-unknown-unknownはシングルスレッドなので安全。ws.rsのWsConnectionと同じ理由)。
#[derive(Clone, Copy)]
pub struct TerrainStore {
    data: RwSignal<Option<Rc<TerrainData>>, LocalStorage>,
    loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
}

impl TerrainStore {
    pub fn new() -> Self {
        Self {
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
        wasm_bindgen_futures::spawn_local(async move {
            match loader::load_terrain().await {
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

impl Default for TerrainStore {
    fn default() -> Self {
        Self::new()
    }
}
