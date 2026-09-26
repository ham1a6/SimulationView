//! 地形のLOD(解像度レベル)の更新。計画(`terrain::lod`)に従って、グリッドの取得・メッシュの作成と
//! 差し替え・不要なグリッドの解放を、画面が固まらないよう小分けにして進める。

use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;

use super::{frame::*, overlay::*, state::*};
use crate::terrain::fetch;
use crate::terrain::geodesy::EnuTransform;
use crate::terrain::heightmap;
use crate::terrain::loader::{self, TerrainData, TileEntry, TileKey, WHOLE_TILE};
use crate::terrain::lod::{self, TileLayout};
use crate::terrain::mesh;
use crate::terrain::origin::Origin;

/// カメラ操作が止まってからLODを更新するまでの待ち時間(ミリ秒)。操作が続く間はメッシュ生成・
/// 取得を繰り返さないためのデバウンスで、取得の完了やメッシュ反映の続きには使わない
/// (それらは`LOD_CONTINUE_MS`。ここを待つと、取得の合間が空いて全体が遅くなる)。
const LOD_DEBOUNCE_MS: u32 = 150;
/// 取得が終わった・メッシュ反映を続けたいときに、次のLOD更新まで待つ時間(ミリ秒)。同じ瞬間に
/// 終わった複数の取得を1回の更新にまとめる程度の短さで、入力イベントや描画に処理を譲る間でもある。
const LOD_CONTINUE_MS: u32 = 8;
/// 1回のLOD更新でメッシュを作ってGPUへ上げる時間の目安(ミリ秒)。これを超えたら残りは次の更新に
/// 回す(メッシュ生成は頂点数に比例して時間がかかるため、画面が固まらないようにする)。時間で区切るので、
/// 速い端末ほど1回で多く進み、遅い端末では自動的に細かく刻まれる。
const UPLOAD_TIME_BUDGET_MS: f64 = 12.0;
/// 1回のLOD更新でメッシュを作ってGPUへ上げる頂点数の安全上限(最細レベルのチャンク1個で約36万
/// 頂点)。時間の目安は次の1個の重さを予測できないので、その保険として頂点数でも区切る。
const MAX_UPLOAD_VERTICES_PER_ROUND: usize = 600_000;
/// 同時に取得するタイル数の上限。全タイルのレベル1(約69KBのファイルが390個)を起動後に
/// 取得していくので、6件だと1分ほどかかった。取得が終わるたびに`schedule_lod_soon`で補充するので、
/// ブラウザの同一ホストへの接続数(HTTP/1.1で通常6)を埋めるのに足りる数にしてある。
const MAX_CONCURRENT_TILE_FETCHES: usize = 16;
/// 取得済みタイルグリッド(細かいレベル)をメモリに残す上限。超えたら、いま使っていないものから
/// 古い順に捨てる。
const DETAIL_CACHE_LIMIT_BYTES: usize = 300 * 1024 * 1024;

/// 1つのグリッドの取得が失敗したとき、恒久的に諦める(`LodState::failed`へ入れる)までに
/// 許す再試行の回数。不安定な低速回線での一時的なタイムアウト・切断を自動的に取り直しつつ、
/// 存在しないファイル(404など)への無駄な再送はどこかで打ち切るためのもの。
const MAX_FETCH_RETRIES: u8 = 3;
/// 再試行までの待ち時間(ミリ秒)。失敗のたびに倍々に伸ばし(`sample/sim_frontend`のWebSocket
/// 再接続と同じ考え方)、`RETRY_BACKOFF_MAX_MS`で頭打ちにする。裏で進む地形取得の再試行なので、
/// WebSocket再接続(最大30秒)より短めにしてある。
const RETRY_BACKOFF_INITIAL_MS: u32 = 500;
/// 再試行までの待ち時間の上限(ミリ秒)。`RETRY_BACKOFF_INITIAL_MS`参照。
const RETRY_BACKOFF_MAX_MS: u32 = 4000;

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
    schedule(state, LOD_DEBOUNCE_MS, |lod| &mut lod.update_pending);
}

/// 取得の完了・メッシュ反映の続きなど、カメラ操作とは関係なく「すぐ続きをやりたい」ときの予約。
/// `schedule_lod`のデバウンス(150ms)を待たず、`LOD_CONTINUE_MS`のあいだに来た予約だけをまとめる。
fn schedule_lod_soon(state: &Rc<RefCell<ViewState>>) {
    schedule(state, LOD_CONTINUE_MS, |lod| &mut lod.update_soon_pending);
}

/// `delay_ms`待ってからLODを更新する。`pending`が指す予約中の印が立っている間の予約はまとめる。
fn schedule(
    state: &Rc<RefCell<ViewState>>,
    delay_ms: u32,
    pending: fn(&mut LodState) -> &mut bool,
) {
    {
        let mut s = state.borrow_mut();
        if s.terrain.is_none() || *pending(&mut s.lod) {
            return;
        }
        *pending(&mut s.lod) = true;
    }
    let state = state.clone();
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(delay_ms).await;
        *pending(&mut state.borrow_mut().lod) = false;
        update_lod(&state);
    });
}

/// 1回のLOD更新の進み具合。
struct Round {
    /// この更新を始めた時刻(`js_sys::Date::now()`、ミリ秒)。`UPLOAD_TIME_BUDGET_MS`の起点。
    start_ms: f64,
    /// この更新でGPUへ上げたメッシュの頂点数の合計(`MAX_UPLOAD_VERTICES_PER_ROUND`と比べる)。
    uploaded_vertices: usize,
    /// 地形のメッシュを1つ以上差し替えたか。
    changed: bool,
    /// 取得の同時数の上限で、取得を始められなかったものがあるか(取得が終われば補充される)。
    deferred: bool,
    /// メッシュ反映の時間・頂点数の目安を超えて、反映を次の更新に回したものがあるか。
    deferred_upload: bool,
    /// この更新で取得を始めるグリッド。
    to_fetch: Vec<FetchKey>,
}

impl Round {
    /// いまの時刻を起点に、何もしていない状態から始める。
    fn new() -> Self {
        Self {
            start_ms: js_sys::Date::now(),
            uploaded_vertices: 0,
            changed: false,
            deferred: false,
            deferred_upload: false,
            to_fetch: Vec::new(),
        }
    }

    /// 頂点数`cost`のメッシュの反映を次の更新に回すか(回すなら`deferred_upload`を立てる)。
    /// 1個は必ず進める(0個だと永遠に終わらない)。以後は、時間の目安か頂点数の上限を超えたら回す。
    fn over_budget(&mut self, cost: usize) -> bool {
        let over = self.uploaded_vertices > 0
            && (js_sys::Date::now() - self.start_ms >= UPLOAD_TIME_BUDGET_MS
                || self.uploaded_vertices + cost > MAX_UPLOAD_VERTICES_PER_ROUND);
        self.deferred_upload |= over;
        over
    }

    /// 取得が必要なグリッドを登録する(同じものは重複させず、同時取得数の上限を守る。
    /// 上限に達したら`deferred`を立てて、あとの更新に回す)。
    fn request(
        &mut self,
        state: &Rc<RefCell<ViewState>>,
        key: TileKey,
        level: usize,
        chunk: usize,
    ) {
        // 小さいレベルはタイル1枚分のファイルをまるごと取るので、チャンクを区別しない
        // (同じファイルをチャンクの数だけ重ねて取りに行かないため)。
        let fetch_key: FetchKey = if level <= loader::WHOLE_FILE_MAX_LEVEL {
            (key, level, None)
        } else {
            (key, level, Some(chunk))
        };
        let lod = &mut state.borrow_mut().lod;
        if lod.failed.contains(&fetch_key) || lod.loading.contains(&fetch_key) {
            return;
        }
        if lod
            .retry_after
            .get(&fetch_key)
            .is_some_and(|&at| js_sys::Date::now() < at)
        {
            return; // 再試行のバックオフ待ち中(取得が失敗し続けているグリッド)。
        }
        if lod.loading.len() >= MAX_CONCURRENT_TILE_FETCHES {
            self.deferred = true;
            return;
        }
        lod.loading.insert(fetch_key);
        self.to_fetch.push(fetch_key);
    }
}

/// カメラに合わせて各タイルの描き方(全体1枚か、チャンクごとのレベルか)を更新する: 必要な
/// グリッドが取得済みならメッシュを作ってGPUへ差し替え、未取得なら取得を始める(届いたら再度
/// この関数が走る)。取得済みの範囲で少しずつ細かくしていく(たとえば、レベル1→2→最細)。
pub(super) fn update_lod(state: &Rc<RefCell<ViewState>>) {
    let (terrain, origin, transform, plan) = {
        let s = state.borrow();
        if s.drag.is_active() {
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
        (terrain, origin, transform, plan)
    };

    // 計画をタイルごとに適用する。`state`の借用はメッシュ生成(重い)をまたがないよう、
    // 読み書きのたびに短く取り直す。
    let mut round = Round::new();
    for (key, tile_plan) in plan {
        let Some(tile) = terrain.tile(key) else {
            continue;
        };
        let current: Option<Vec<u8>> = state
            .borrow()
            .lod
            .resident
            .get(&key)
            .and_then(TileLayout::chunk_levels)
            .map(<[u8]>::to_vec);
        match tile_plan {
            TileLayout::Whole => {
                // いまチャンク表示なら全体1枚へ戻す(もともと全体表示なら何もしない)。
                if current.is_some() {
                    show_whole_tile(&mut round, state, &terrain, tile, &transform);
                }
            }
            TileLayout::Chunks(targets) => {
                update_chunks(
                    &mut round, state, &terrain, tile, &targets, current, &transform,
                );
            }
        }
    }

    // 取得は借用を手放したあとでまとめて始める(`spawn_fetch`の中で`state`を借用するため)。
    for fetch_key in round.to_fetch {
        spawn_fetch(state, &terrain, fetch_key);
    }
    if round.changed {
        after_terrain_changed(state, &terrain, &origin, &transform);
    }
    if round.deferred_upload {
        // メッシュ反映の続き。入力や描画に処理を譲ってから、すぐ次の分に進む。
        schedule_lod_soon(state);
    } else if round.deferred {
        // 取得の同時数が上限だった分。取得が終わるたびに`schedule_lod_soon`されるので、これは念のため。
        schedule_lod(state);
    }
}

/// チャンク表示のタイルを、タイル全体の1枚のメッシュへ戻す(小さいので頂点数の上限には数えない)。
fn show_whole_tile(
    round: &mut Round,
    state: &Rc<RefCell<ViewState>>,
    terrain: &TerrainData,
    tile: &TileEntry,
    transform: &EnuTransform,
) {
    let key = tile.key;
    let whole = mesh::build_whole_tile_mesh(terrain, tile, transform);
    let mut s = state.borrow_mut();
    if let Some(renderer) = s.renderer.as_mut() {
        renderer.set_mesh_faded((key.0, key.1, WHOLE_TILE), &whole);
        for c in 0..terrain.chunk_count() {
            renderer.remove_mesh_faded((key.0, key.1, c as u8));
        }
    }
    s.lod.resident.insert(key, TileLayout::Whole);
    drop(s);
    terrain.set_whole_tile(key);
    round.changed = true;
}

/// チャンク表示するタイルの各チャンクを、目標レベル`targets`へ近づける。`current`はいまのチャンクごとの
/// レベル(`None`なら全体表示中)。
fn update_chunks(
    round: &mut Round,
    state: &Rc<RefCell<ViewState>>,
    terrain: &TerrainData,
    tile: &TileEntry,
    targets: &[u8],
    current: Option<Vec<u8>>,
    transform: &EnuTransform,
) {
    let key = tile.key;
    let chunk_count = terrain.chunk_count();
    // 各チャンクの目標レベルのうち、取得済みで最も細かいレベル(無ければ0)。
    let available: Vec<usize> = (0..chunk_count)
        .map(|c| terrain.best_cached_level(tile, c, targets[c] as usize))
        .collect();

    let mut levels: Vec<u8> = match current {
        Some(levels) => levels,
        None => {
            // 全体表示からチャンク表示へ切り替えるには、全チャンクのレベル1が要る。
            if available.contains(&0) {
                round.request(state, key, 1, 0);
                return;
            }
            let cost: usize = available
                .iter()
                .map(|&l| lod::chunk_vertex_cost(terrain, l))
                .sum();
            if round.over_budget(cost) {
                return;
            }
            let mut s = state.borrow_mut();
            for (c, &level) in available.iter().enumerate() {
                if let Some(m) = mesh::build_chunk_mesh(terrain, tile, c, level, transform) {
                    if let Some(renderer) = s.renderer.as_mut() {
                        renderer.set_mesh_faded((key.0, key.1, c as u8), &m);
                    }
                    terrain.set_chunk_level(key, c, level);
                }
            }
            if let Some(renderer) = s.renderer.as_mut() {
                renderer.remove_mesh_faded((key.0, key.1, WHOLE_TILE));
            }
            let levels: Vec<u8> = available.iter().map(|&l| l as u8).collect();
            s.lod
                .resident
                .insert(key, TileLayout::Chunks(levels.clone()));
            round.uploaded_vertices += cost;
            round.changed = true;
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
        // 取得済みの細かいグリッドが無ければ据え置き。目標が今より粗いなら取得済みの最細
        // (目標以下)まで下げ、そうでなければ今より粗くはしない。
        let new_level = if now == 0 {
            have
        } else if want < have {
            now
        } else {
            now.max(have)
        };
        if !terrain.has_chunk_grid(tile, want, c) {
            round.request(state, key, want, c);
        }
        if new_level == have {
            continue;
        }
        let cost = lod::chunk_vertex_cost(terrain, new_level);
        if round.over_budget(cost) {
            continue;
        }
        if let Some(m) = mesh::build_chunk_mesh(terrain, tile, c, new_level, transform) {
            if let Some(renderer) = state.borrow_mut().renderer.as_mut() {
                renderer.set_mesh_faded((key.0, key.1, c as u8), &m);
            }
            terrain.set_chunk_level(key, c, new_level);
            levels[c] = new_level as u8;
            round.uploaded_vertices += cost;
            resident_changed = true;
            round.changed = true;
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

/// グリッド1つ(`fetch_key`)の取得を始める。終わったら(失敗なら再試行のバックオフの後に)LODの更新を予約する。
fn spawn_fetch(state: &Rc<RefCell<ViewState>>, terrain: &Rc<TerrainData>, fetch_key: FetchKey) {
    let state = state.clone();
    let terrain = terrain.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let (key, level, chunk) = fetch_key;
        let result = match chunk {
            None => fetch::fetch_tile_level(&terrain, key, level)
                .await
                .map(|all| terrain.insert_tile_level(key, level, &all)),
            Some(c) => fetch::fetch_chunk_grid(&terrain, key, level, c)
                .await
                .map(|grid| terrain.insert_chunk_grid(key, level, c, grid)),
        };
        // 取得中の印を外し、結果に応じて再試行の状態を更新する(成功なら消す・失敗なら数えて
        // 待ち時間を決める・上限を超えたら諦める)。
        let retry_backoff_ms = {
            let lod = &mut state.borrow_mut().lod;
            lod.loading.remove(&fetch_key);
            match result {
                Ok(()) => {
                    lod.clear_retry(&fetch_key);
                    None
                }
                Err(e) => {
                    let attempts = lod.retry_counts.entry(fetch_key).or_insert(0);
                    *attempts += 1;
                    let attempts = *attempts;
                    if attempts > MAX_FETCH_RETRIES {
                        log::warn!(
                            "[terrain] tile fetch failed after {attempts} attempts, giving up: {e}"
                        );
                        lod.clear_retry(&fetch_key);
                        lod.failed.insert(fetch_key);
                        None
                    } else {
                        let backoff_ms = retry_backoff_ms(attempts);
                        log::warn!(
                            "[terrain] tile fetch failed (attempt {attempts}/{MAX_FETCH_RETRIES}), retrying in {backoff_ms}ms: {e}"
                        );
                        lod.retry_after
                            .insert(fetch_key, js_sys::Date::now() + backoff_ms as f64);
                        Some(backoff_ms)
                    }
                }
            }
        };
        // バックオフ待ちのぶんは、待ち時間の後に専用のタイマーで取得を再度促す
        // (カメラ操作などの他の予約を待たずに、確実に再試行されるようにする)。
        // 成功・恒久失敗のいずれも、取得が終わったらデバウンスを待たず続き
        // (反映と、次の取得の補充)へ進む。
        if let Some(backoff_ms) = retry_backoff_ms {
            gloo_timers::future::TimeoutFuture::new(backoff_ms).await;
        }
        schedule_lod_soon(&state);
    });
}

/// 地形の高さが変わったので、注視点の高さ(ズームインしても地面に埋まらないように)と、
/// 地表に貼り付いている観測点・覆域・作図・航跡を合わせ直し、使っていないグリッドを解放する。
fn after_terrain_changed(
    state: &Rc<RefCell<ViewState>>,
    terrain: &TerrainData,
    origin: &Origin,
    transform: &EnuTransform,
) {
    {
        let mut s = state.borrow_mut();
        s.target_up = heightmap::sample_heightmap(terrain, origin.lat_deg, origin.lon_deg);
        let (tx, ty) = (s.camera.target.x as f64, s.camera.target.y as f64);
        s.camera.target.z = heightmap::ground_at_enu(terrain, transform, tx, ty).2;
        // いま表示に使っている(チャンクごとのレベルと一致する)グリッドは残し、それ以外を
        // 上限を超えたぶんだけ解放する。
        let chunks = terrain.chunk_count();
        let resident = &s.lod.resident;
        terrain.evict_unused(
            |key, chunk, level| {
                resident
                    .get(&key)
                    .and_then(TileLayout::chunk_levels)
                    .is_some_and(|levels| chunk < chunks && levels[chunk] as usize == level)
            },
            DETAIL_CACHE_LIMIT_BYTES,
        );
    }
    // 観測点(地表に立てたマーカー・覆域)も地形の高さに合わせて作り直す。
    let has_markers = !state
        .borrow()
        .radar_markers
        .markers
        .get_untracked()
        .is_empty();
    if has_markers {
        rebuild_markers_for_terrain(state);
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
