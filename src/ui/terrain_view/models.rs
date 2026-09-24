//! 3Dモデル(`terrain::models`)の、`TerrainView`側の受け持ち: モデルファイルの取得・GPUへの登録と、毎フレームの
//! 「どのトラックをモデルで描くか」の決定。決定した結果は、レンダラーへのインスタンス(`update_model_instances`)と、
//! シンボルを描かないトラックの集合(`ModelsView::shown`。変わったら`rebuild_tracks`がシンボルを作り直す)になる。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use leptos::prelude::*;

use super::{frame::render_frame, overlay::rebuild_tracks, state::*};
use crate::terrain::fetch::fetch_binary;
use crate::terrain::models::placement::{
    plan_models, DisplaySettings, ModelPlacement, ViewMetrics,
};
use crate::terrain::models::{import_glb, ModelDisplayMode, ModelsState};
use crate::terrain::tracks::TrackId;

/// モデルファイル1つの取得状況。
enum ModelLoad {
    Loading,
    /// 読み込んでGPUへ登録済み。`radius_m`はモデルの実寸の半径(`source.scale`を掛ける前)。
    Ready {
        radius_m: f32,
    },
    /// 取得・解析に失敗した(同じ取得を繰り返さないよう覚えておく。そのモデルのトラックはシンボルで描く)。
    Failed,
}

/// `ViewState`が持つ、3Dモデルの状態。
pub(super) struct ModelsView {
    pub(super) state: ModelsState,
    /// URLごとの取得状況。
    loads: HashMap<String, ModelLoad>,
    /// 最新のトラックの配置(トラックの更新・原点変更のたびに`rebuild_tracks`が作り直す)。
    pub(super) placements: Vec<ModelPlacement>,
    /// いまモデルで描いているトラック(シンボルは描かない)。
    pub(super) shown: HashSet<TrackId>,
}

impl ModelsView {
    pub(super) fn new(state: ModelsState) -> Self {
        Self {
            state,
            loads: HashMap::new(),
            placements: Vec::new(),
            shown: HashSet::new(),
        }
    }
}

/// いまのカメラ・設定・配置から、モデルで描くトラックとそのインスタンスを決めてレンダラーへ渡す。
/// 必要なモデルファイルの取得もここで始める。モデルで描くトラックの集合が変わったら、シンボルを描き直す。
/// 毎フレーム(`render_frame`の描画の前)に呼ぶ。
pub(super) fn update_models(state: &Rc<RefCell<ViewState>>) {
    let (to_load, changed) = {
        let mut guard = state.borrow_mut();
        let s = &mut *guard;
        let Some(renderer) = s.renderer.as_mut() else {
            return;
        };
        let settings = DisplaySettings {
            mode: s.models.state.mode.get_untracked(),
            switch_distance_m: s.models.state.switch_distance_m.get_untracked() as f32,
            min_screen_px: s.models.state.min_screen_px.get_untracked() as f32,
        };
        let sources = s.models.state.sources.get_untracked();

        // 登録から外れた(URLが変わった・種別の登録が消えた)モデルは、GPUから外す。
        let wanted: HashSet<&str> = sources.values().map(|source| source.url.as_str()).collect();
        let stale: Vec<String> = s
            .models
            .loads
            .keys()
            .filter(|url| !wanted.contains(url.as_str()))
            .cloned()
            .collect();
        for url in stale {
            s.models.loads.remove(&url);
            renderer.remove_model(&url);
        }

        // まだ取得していないモデルを取得し始める(表示方式がOffの間や、使われない種別のモデルは取得しない)。
        let mut to_load: Vec<String> = Vec::new();
        if settings.mode != ModelDisplayMode::Off {
            for placement in &s.models.placements {
                let Some(source) = sources.get(&placement.kind) else {
                    continue;
                };
                if !s.models.loads.contains_key(&source.url) {
                    s.models
                        .loads
                        .insert(source.url.clone(), ModelLoad::Loading);
                    to_load.push(source.url.clone());
                }
            }
        }

        let camera = s.camera.to_camera(renderer.aspect_ratio());
        let metrics = ViewMetrics::new(&camera, renderer.canvas_size_px().1 as f32);
        let loads = &s.models.loads;
        let radius_of = |url: &str| match loads.get(url) {
            Some(ModelLoad::Ready { radius_m }) => Some(*radius_m),
            _ => None,
        };
        let plan = plan_models(
            &s.models.placements,
            &sources,
            &radius_of,
            settings,
            &metrics,
            &s.models.shown,
        );
        renderer.update_model_instances(&plan.instances);
        let changed = plan.shown != s.models.shown;
        s.models.shown = plan.shown;
        (to_load, changed)
    };

    for url in to_load {
        wasm_bindgen_futures::spawn_local(load_model(state.clone(), url));
    }
    if changed {
        // シンボルを描くトラックが変わった。
        rebuild_tracks(state);
    }
}

/// モデルファイルを取得・解析してGPUへ登録し、描き直す。失敗したらログに残してシンボルのままにする。
async fn load_model(state: Rc<RefCell<ViewState>>, url: String) {
    let result = match fetch_binary(&url, None).await {
        Ok(bytes) => import_glb(&bytes),
        Err(e) => Err(e),
    };
    {
        let mut guard = state.borrow_mut();
        let s = &mut *guard;
        // 取得している間に登録が外れていたら、捨てる(`update_models`が`loads`から外している)。
        if !s.models.loads.contains_key(&url) {
            return;
        }
        let load = match (result, s.renderer.as_mut()) {
            (Ok(mesh), Some(renderer)) => {
                renderer.set_model(&url, &mesh);
                ModelLoad::Ready {
                    radius_m: mesh.radius_m,
                }
            }
            (Err(e), _) => {
                log::warn!("[models] {url}を読み込めません(シンボルで描きます): {e}");
                ModelLoad::Failed
            }
            (Ok(_), None) => ModelLoad::Failed,
        };
        s.models.loads.insert(url, load);
    }
    render_frame(&state);
}
