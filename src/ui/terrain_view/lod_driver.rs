//! 地形のLOD(解像度レベル)の更新。計画(`terrain::lod`)に従って、グリッドの取得・メッシュの作成と
//! 差し替え・不要なグリッドの解放を、画面が固まらないよう小分けにして進める。

use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;

use super::{frame::*, overlay::*, state::*};
use crate::terrain::fetch;
use crate::terrain::geodesy::EnuTransform;
use crate::terrain::heightmap;
use crate::terrain::loader::{self, MeshKey, TileKey, WHOLE_TILE};
use crate::terrain::lod::{self, TileLayout};
use crate::terrain::mesh;

/// カメラ操作が止まってからLODを更新するまでの待ち時間(ミリ秒)。操作が続く間はメッシュ生成・
/// 取得を繰り返さないためのデバウンスで、取得の完了やメッシュ反映の続きには使わない
/// (それらは`LOD_CONTINUE_MS`。ここを待つと、取得の合間が空いて全体が遅くなる)。
pub(super) const LOD_DEBOUNCE_MS: u32 = 150;
/// 取得が終わった・メッシュ反映を続けたいときに、次のLOD更新まで待つ時間(ミリ秒)。同じ瞬間に
/// 終わった複数の取得を1回の更新にまとめる程度の短さで、入力イベントや描画に処理を譲る間でもある。
pub(super) const LOD_CONTINUE_MS: u32 = 8;
/// 1回のLOD更新でメッシュを作ってGPUへ上げる時間の目安(ミリ秒)。これを超えたら残りは次の更新に
/// 回す(メッシュ生成は頂点数に比例して時間がかかるため、画面が固まらないようにする)。時間で区切るので、
/// 速い端末ほど1回で多く進み、遅い端末では自動的に細かく刻まれる。
pub(super) const UPLOAD_TIME_BUDGET_MS: f64 = 12.0;
/// 1回のLOD更新でメッシュを作ってGPUへ上げる頂点数の安全上限(最細レベルのチャンク1個で約36万
/// 頂点)。時間の目安は次の1個の重さを予測できないので、その保険として頂点数でも区切る。
pub(super) const MAX_UPLOAD_VERTICES_PER_ROUND: usize = 600_000;
/// 同時に取得するタイル数の上限。全タイルのレベル1(約69KBのファイルが390個)を起動後に
/// 取得していくので、6件だと1分ほどかかった。取得が終わるたびに`schedule_lod_soon`で補充するので、
/// ブラウザの同一ホストへの接続数(HTTP/1.1で通常6)を埋めるのに足りる数にしてある。
pub(super) const MAX_CONCURRENT_TILE_FETCHES: usize = 16;
/// 取得済みタイルグリッド(細かいレベル)をメモリに残す上限。超えたら、いま使っていないものから
/// 古い順に捨てる。
pub(super) const DETAIL_CACHE_LIMIT_BYTES: usize = 300 * 1024 * 1024;

/// 1つのグリッドの取得が失敗したとき、恒久的に諦める(`LodState::failed`へ入れる)までに
/// 許す再試行の回数。不安定な低速回線での一時的なタイムアウト・切断を自動的に取り直しつつ、
/// 存在しないファイル(404など)への無駄な再送はどこかで打ち切るためのもの。
pub(super) const MAX_FETCH_RETRIES: u8 = 3;
/// 再試行までの待ち時間(ミリ秒)。失敗のたびに倍々に伸ばし(`sample/sim_frontend`のWebSocket
/// 再接続と同じ考え方)、`RETRY_BACKOFF_MAX_MS`で頭打ちにする。裏で進む地形取得の再試行なので、
/// WebSocket再接続(最大30秒)より短めにしてある。
pub(super) const RETRY_BACKOFF_INITIAL_MS: u32 = 500;
pub(super) const RETRY_BACKOFF_MAX_MS: u32 = 4000;

/// `attempt`回目(1始まり)の失敗の後、次の再試行まで待つ時間(ミリ秒)。倍々に伸ばし、
/// `RETRY_BACKOFF_MAX_MS`で頭打ちにする。
fn retry_backoff_ms(attempt: u8) -> u32 {
    RETRY_BACKOFF_INITIAL_MS
        .saturating_mul(1 << (attempt.saturating_sub(1)).min(4))
        .min(RETRY_BACKOFF_MAX_MS)
}

/// 少し待ってからLODを更新する。待っている間に来た予約はまとめる(操作が続く間は
/// メッシュ生成・取得を繰り返さず、操作が落ち着いた時点の最新のカメラで一度だけ計画し直す)。
pub(super) fn schedule_lod(state: &Rc<RefCell<ViewState>>) {
    {
        let mut s = state.borrow_mut();
        if s.lod.update_pending || s.terrain.is_none() {
            return;
        }
        s.lod.update_pending = true;
    }
    let state = state.clone();
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(LOD_DEBOUNCE_MS).await;
        state.borrow_mut().lod.update_pending = false;
        update_lod(&state);
    });
}

/// 取得の完了・メッシュ反映の続きなど、カメラ操作とは関係なく「すぐ続きをやりたい」ときの予約。
/// `schedule_lod`のデバウンス(150ms)を待たず、`LOD_CONTINUE_MS`のあいだに来た予約だけをまとめる。
pub(super) fn schedule_lod_soon(state: &Rc<RefCell<ViewState>>) {
    {
        let mut s = state.borrow_mut();
        if s.lod.update_soon_pending || s.terrain.is_none() {
            return;
        }
        s.lod.update_soon_pending = true;
    }
    let state = state.clone();
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(LOD_CONTINUE_MS).await;
        state.borrow_mut().lod.update_soon_pending = false;
        update_lod(&state);
    });
}

/// カメラに合わせて各タイルの描き方(全体1枚か、チャンクごとのレベルか)を更新する: 必要な
/// グリッドが取得済みならメッシュを作ってGPUへ差し替え、未取得なら取得を始める(届いたら再度
/// この関数が走る)。取得済みの範囲で少しずつ細かくしていく(たとえば、レベル1→2→最細)。
pub(super) fn update_lod(state: &Rc<RefCell<ViewState>>) {
    let (terrain, origin, plan) = {
        let s = state.borrow();
        if s.interaction.drag.is_active() {
            drop(s);
            schedule_lod(state); // ドラッグ中は重い処理を避け、落ち着いてからやり直す。
            return;
        }
        let (Some(terrain), Some(origin), Some(renderer)) =
            (s.terrain.clone(), s.mesh_origin, s.renderer.as_ref())
        else {
            return;
        };
        let transform = EnuTransform::new(&origin, &terrain.metadata.ellipsoid);
        let camera = s.camera.to_camera(renderer.aspect_ratio());
        let plan = lod::plan_levels(
            &terrain,
            &transform,
            &camera,
            renderer.canvas_size_px().1 as f32,
            &s.lod.resident,
        );
        (terrain, origin, plan)
    };
    let transform = EnuTransform::new(&origin, &terrain.metadata.ellipsoid);
    let chunk_count = terrain.chunk_count();

    let round_start = js_sys::Date::now();
    let mut uploaded_vertices = 0usize;
    let mut changed = false;
    // 取得の同時数の上限で、取得を始められなかったものがあるか(取得が終われば補充される)。
    let mut deferred = false;
    // メッシュ反映の時間・頂点数の目安を超えて、反映を次の更新に回したものがあるか。
    let mut deferred_upload = false;
    // 1個は必ず進める(0個だと永遠に終わらない)。以後は、時間の目安か頂点数の上限を超えたら回す。
    let over_budget = |uploaded: usize, cost: usize| {
        uploaded > 0
            && (js_sys::Date::now() - round_start >= UPLOAD_TIME_BUDGET_MS
                || uploaded + cost > MAX_UPLOAD_VERTICES_PER_ROUND)
    };
    let mut to_fetch: Vec<FetchKey> = Vec::new();
    // 取得が必要なグリッドを登録する(同じものは重複させず、同時取得数の上限を守る)。
    let mut request = |state: &Rc<RefCell<ViewState>>, key: TileKey, level: usize, chunk: usize| {
        let fetch_key: FetchKey = if level <= loader::WHOLE_FILE_MAX_LEVEL {
            (key, level, None)
        } else {
            (key, level, Some(chunk))
        };
        let mut s = state.borrow_mut();
        if s.lod.failed.contains(&fetch_key) || s.lod.loading.contains(&fetch_key) {
            return false;
        }
        if s.lod
            .retry_after
            .get(&fetch_key)
            .is_some_and(|&at| js_sys::Date::now() < at)
        {
            return false; // 再試行のバックオフ待ち中(取得が失敗し続けているグリッド)。
        }
        if s.lod.loading.len() >= MAX_CONCURRENT_TILE_FETCHES {
            return true; // 上限に達したので、あとの更新に回す。
        }
        s.lod.loading.insert(fetch_key);
        to_fetch.push(fetch_key);
        false
    };

    for (key, tile_plan) in plan {
        let Some(tile) = terrain.tile(key) else {
            continue;
        };
        let whole_key: MeshKey = (key.0, key.1, WHOLE_TILE);
        let current: Option<Vec<u8>> = match state.borrow().lod.resident.get(&key) {
            Some(TileLayout::Chunks(v)) => Some(v.clone()),
            _ => None,
        };

        match tile_plan {
            TileLayout::Whole => {
                if current.is_some() {
                    // タイル全体の1枚のメッシュへ戻す(小さいので頂点数の上限には数えない)。
                    let whole = mesh::build_whole_tile_mesh(&terrain, tile, &transform);
                    let mut s = state.borrow_mut();
                    if let Some(renderer) = s.renderer.as_mut() {
                        renderer.set_mesh_faded(whole_key, &whole);
                        for c in 0..chunk_count {
                            renderer.remove_mesh_faded((key.0, key.1, c as u8));
                        }
                    }
                    s.lod.resident.insert(key, TileLayout::Whole);
                    drop(s);
                    terrain.set_whole_tile(key);
                    changed = true;
                }
            }
            TileLayout::Chunks(targets) => {
                // 各チャンクの目標レベルのうち、取得済みで最も細かいレベル(無ければ0)。
                let available: Vec<usize> = (0..chunk_count)
                    .map(|c| terrain.best_cached_level(tile, c, targets[c] as usize))
                    .collect();

                let mut levels: Vec<u8> = match current {
                    Some(levels) => levels,
                    None => {
                        // 全体表示からチャンク表示へ切り替えるには、全チャンクのレベル1が要る。
                        if available.contains(&0) {
                            if request(state, key, 1, 0) {
                                deferred = true;
                            }
                            continue;
                        }
                        let cost: usize = available
                            .iter()
                            .map(|&l| lod::chunk_vertex_cost(&terrain, l))
                            .sum();
                        if over_budget(uploaded_vertices, cost) {
                            deferred_upload = true;
                            continue;
                        }
                        let mut s = state.borrow_mut();
                        for (c, &level) in available.iter().enumerate() {
                            if let Some(m) =
                                mesh::build_chunk_mesh(&terrain, tile, c, level, &transform)
                            {
                                if let Some(renderer) = s.renderer.as_mut() {
                                    renderer.set_mesh_faded((key.0, key.1, c as u8), &m);
                                }
                                terrain.set_chunk_level(key, c, level);
                            }
                        }
                        if let Some(renderer) = s.renderer.as_mut() {
                            renderer.remove_mesh_faded(whole_key);
                        }
                        let levels: Vec<u8> = available.iter().map(|&l| l as u8).collect();
                        s.lod
                            .resident
                            .insert(key, TileLayout::Chunks(levels.clone()));
                        uploaded_vertices += cost;
                        changed = true;
                        levels
                    }
                };

                // 目標レベルへ近づける。目標のグリッドが取得済みならそのレベルへ(上げ下げとも)、
                // 未取得なら取得を始め、取得済みの範囲でより細かければ先にそこまで上げる。
                let mut resident_changed = false;
                for c in 0..chunk_count {
                    let want = targets[c] as usize;
                    let have = levels[c] as usize;
                    let now = available[c];
                    let new_level = if now == 0 {
                        have
                    } else if want < have {
                        now
                    } else {
                        now.max(have)
                    };
                    if !terrain.has_chunk_grid(tile, want, c) && request(state, key, want, c) {
                        deferred = true;
                    }
                    if new_level == have {
                        continue;
                    }
                    let cost = lod::chunk_vertex_cost(&terrain, new_level);
                    if over_budget(uploaded_vertices, cost) {
                        deferred_upload = true;
                        continue;
                    }
                    if let Some(m) =
                        mesh::build_chunk_mesh(&terrain, tile, c, new_level, &transform)
                    {
                        let mut s = state.borrow_mut();
                        if let Some(renderer) = s.renderer.as_mut() {
                            renderer.set_mesh_faded((key.0, key.1, c as u8), &m);
                        }
                        drop(s);
                        terrain.set_chunk_level(key, c, new_level);
                        levels[c] = new_level as u8;
                        uploaded_vertices += cost;
                        resident_changed = true;
                        changed = true;
                    }
                }
                if resident_changed {
                    state
                        .borrow_mut()
                        .lod
                        .resident
                        .insert(key, TileLayout::Chunks(levels));
                }
            }
        }
    }

    for fetch_key in to_fetch {
        let state = state.clone();
        let terrain = terrain.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let (key, level, chunk) = fetch_key;
            let result = match chunk {
                None => fetch::fetch_tile_level(
                    &terrain.base_url,
                    key,
                    level,
                    terrain.chunk_cells(level),
                    terrain.chunk_count(),
                )
                .await
                .map(|all| terrain.insert_tile_level(key, level, &all)),
                Some(c) => fetch::fetch_chunk_grid(
                    &terrain.base_url,
                    key,
                    level,
                    c,
                    terrain.chunk_cells(level),
                )
                .await
                .map(|grid| terrain.insert_chunk_grid(key, level, c, grid)),
            };
            let retry_backoff_ms = {
                let mut s = state.borrow_mut();
                s.lod.loading.remove(&fetch_key);
                match result {
                    Ok(()) => {
                        s.lod.retry_counts.remove(&fetch_key);
                        s.lod.retry_after.remove(&fetch_key);
                        None
                    }
                    Err(e) => {
                        let attempts = s.lod.retry_counts.entry(fetch_key).or_insert(0);
                        *attempts += 1;
                        if *attempts > MAX_FETCH_RETRIES {
                            log::warn!(
                                "[terrain] tile fetch failed after {attempts} attempts, giving up: {e}"
                            );
                            s.lod.retry_counts.remove(&fetch_key);
                            s.lod.retry_after.remove(&fetch_key);
                            s.lod.failed.insert(fetch_key);
                            None
                        } else {
                            let backoff_ms = retry_backoff_ms(*attempts);
                            log::warn!(
                                "[terrain] tile fetch failed (attempt {attempts}/{MAX_FETCH_RETRIES}), retrying in {backoff_ms}ms: {e}"
                            );
                            s.lod
                                .retry_after
                                .insert(fetch_key, js_sys::Date::now() + backoff_ms as f64);
                            Some(backoff_ms)
                        }
                    }
                }
            };
            match retry_backoff_ms {
                // バックオフ待ちのぶんは、待ち時間の後に専用のタイマーで取得を再度促す
                // (カメラ操作などの他の予約を待たずに、確実に再試行されるようにする)。
                Some(backoff_ms) => {
                    gloo_timers::future::TimeoutFuture::new(backoff_ms).await;
                    schedule_lod_soon(&state);
                }
                // 成功・恒久失敗のいずれも、取得が終わったらデバウンスを待たず続き
                // (反映と、次の取得の補充)へ進む。
                None => schedule_lod_soon(&state),
            }
        });
    }

    if changed {
        // 地形の高さが変わったので、注視点の高さ(ズームインしても地面に埋まらないように)と、
        // 地表に貼り付いている観測点・覆域を合わせ直す。
        {
            let mut s = state.borrow_mut();
            s.target_up = heightmap::sample_heightmap(&terrain, origin.lat_deg, origin.lon_deg);
            let (tx, ty) = (s.camera.target.x as f64, s.camera.target.y as f64);
            s.camera.target.z = heightmap::ground_at_enu(&terrain, &transform, tx, ty).2;
            let chunks = terrain.chunks_per_tile() * terrain.chunks_per_tile();
            let resident = &s.lod.resident;
            terrain.evict_unused(
                |key, chunk, level| match resident.get(&key) {
                    Some(TileLayout::Chunks(levels)) => {
                        chunk < chunks && levels[chunk] as usize == level
                    }
                    _ => false,
                },
                DETAIL_CACHE_LIMIT_BYTES,
            );
        }
        let has_markers = !state
            .borrow()
            .radar_markers
            .markers
            .get_untracked()
            .is_empty();
        if has_markers {
            let radar_markers = state.borrow().radar_markers;
            rebuild_markers_for_terrain(state, radar_markers);
        }
        // 地表に貼り付けた作図も、地形の高さが変わったので作り直す。
        let follows_terrain = state.borrow().drawings.items.with_untracked(|list| {
            list.iter()
                .any(|d| d.visible && d.shape.depends_on_terrain())
        });
        if follows_terrain {
            rebuild_drawings(state);
        }
        // 航跡(地表基準のトラック・高度線)も地形の高さが変わったので作り直す。
        if !state
            .borrow()
            .tracks
            .entries
            .with_untracked(|list| list.is_empty())
        {
            rebuild_tracks(state);
        }
        render_frame(state);
    }
    if deferred_upload {
        // メッシュ反映の続き。入力や描画に処理を譲ってから、すぐ次の分に進む。
        schedule_lod_soon(state);
    } else if deferred {
        // 取得の同時数が上限だった分。取得が終わるたびに`schedule_lod_soon`されるので、これは念のため。
        schedule_lod(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_each_attempt_and_caps_at_the_max() {
        assert_eq!(retry_backoff_ms(1), RETRY_BACKOFF_INITIAL_MS);
        assert_eq!(retry_backoff_ms(2), RETRY_BACKOFF_INITIAL_MS * 2);
        assert_eq!(retry_backoff_ms(3), RETRY_BACKOFF_INITIAL_MS * 4);
        assert_eq!(retry_backoff_ms(20), RETRY_BACKOFF_MAX_MS);
    }

    #[test]
    fn backoff_never_exceeds_the_max_even_for_attempt_zero() {
        // `attempt`は1始まりの想定だが、0でもパニックしないことを確認する
        // (`saturating_sub`で下限を守っている)。
        assert_eq!(retry_backoff_ms(0), RETRY_BACKOFF_INITIAL_MS);
    }
}
