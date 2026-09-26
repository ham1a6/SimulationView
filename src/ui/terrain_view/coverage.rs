//! 覆域(3Dの半球ドーム・2Dの探知可能領域)の計算と、計算結果のキャッシュ。
//!
//! 覆域の計算は重い(方位ごとに1,000サンプルの標高を引く)。以前は、マーカーの操作・地形のレベルの切り替えの
//! たびに全部を同期で計算し直していたので、覆域を出している間はカメラ操作(ズームなど)のたびに画面が固まった。
//! そこで次のようにしてある。
//! - **計算結果をキャッシュ**する。同じ観測点・モード・高度・地形なら、原点の変更やドームの選択し直しでは計算し直さず、
//!   ジオメトリだけを作り直す(`CoverageKey`)。
//! - **地形が変わったときだけ**計算し直す。地形のキーは、観測点の最大観測範囲に重なるチャンクのレベルの組
//!   (`terrain_signature`)。範囲の外のチャンクが切り替わっても計算し直さない。
//! - 計算は**小分けにして非同期**で進める(`ui::util::run_in_slices`)。進行中に新しい要求が来たら、古い計算は捨てる。
//!   地形のレベルの切り替えは、続けて何度も起きるので、少し待ってから始める(`TERRAIN_DEBOUNCE_MS`)。
//! - 計算が終わるまでは、前の覆域をそのまま出す(地形だけが変わったとき)か、消す(観測点・モード・高度が変わったとき)。
//!
//! **複数の覆域を同時に出せる**(`RadarMarkersState::show_all_coverage`)。キャッシュ・計算・ジオメトリは
//! 観測点ごと(`MarkerCoverage`)に持ち、GPUには、表示する観測点のジオメトリをつないで1つのバッファとして載せる
//! (`sync_gpu`)。観測点が増えても、計算は観測点ごとに独立して小分けに進み、すでに計算済みのものは計算し直さない。

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use leptos::prelude::*;

use super::frame::render_frame;
use super::state::ViewState;
use crate::terrain::camera::ViewMode;
use crate::terrain::loader::{TerrainData, TileKey};
use crate::terrain::lod::TileLayout;
use crate::terrain::los::{DomeRing, LosPoint};
use crate::terrain::markers::{self, RadarMarker};
use crate::terrain::mesh::TerrainVertex;
use crate::terrain::origin::Origin;
use crate::terrain::vertex::DrawVertex;
use crate::ui::util::run_in_slices;

/// 地形のレベルの切り替えで計算し直すとき、始めるまでの待ち時間(ミリ秒)。切り替えは短い間に何度も起きるので、
/// 落ち着くまで待つ(待っている間に次の切り替えが来たら、待ちなおす)。
const TERRAIN_DEBOUNCE_MS: u32 = 300;
/// 計算を1回に進める方位の数(`ui::util::run_in_slices`の持ち時間の中で繰り返し呼ぶ)。
const AZIMUTHS_PER_STEP: usize = 4;

/// 覆域の計算結果。
enum CoverageData {
    Dome(Vec<DomeRing>),
    Area(Vec<LosPoint>),
}

/// 覆域の計算結果が使い回せる条件。原点(メッシュ)は含まない(結果は観測点を中心とした値なので)。
#[derive(Clone, PartialEq)]
struct CoverageKey {
    marker: RadarMarker,
    mode: ViewMode,
    /// 2Dの覆域高度(`f64::to_bits`)。3Dは0。
    altitude_bits: u64,
    /// 観測点の範囲の地形(`terrain_signature`)。
    terrain: u64,
}

impl CoverageKey {
    /// 地形以外(観測点・モード・高度)が同じか。地形だけが違うなら、計算し直している間も前の覆域を出す。
    fn same_request(&self, other: &Self) -> bool {
        self.marker == other.marker
            && self.mode == other.mode
            && self.altitude_bits == other.altitude_bits
    }
}

struct CoverageCache {
    key: CoverageKey,
    data: Rc<CoverageData>,
}

/// 計算結果から作った頂点(観測点の色つき)。
enum Geometry {
    Dome(Vec<TerrainVertex>),
    Area(Vec<DrawVertex>),
}

/// 作ったジオメトリと、その元(計算結果のキーと、頂点を作ったメッシュ原点)。
struct BuiltGeometry {
    key: CoverageKey,
    origin: Origin,
    geometry: Geometry,
}

/// 観測点1つぶんの覆域の状態。
#[derive(Default)]
struct MarkerCoverage {
    cache: Option<CoverageCache>,
    built: Option<BuiltGeometry>,
    /// 計算中(待ち時間を含む)のキー。
    pending: Option<CoverageKey>,
    /// 計算の世代。新しい要求・取り消しのたびに増やし、古い計算は自分の世代でなくなったら止まる。
    generation: u64,
}

/// `ViewState`が持つ覆域の状態。
#[derive(Default)]
pub(super) struct CoverageState {
    markers: HashMap<u64, MarkerCoverage>,
    /// 表示する観測点(表示の順)。
    visible: Vec<u64>,
    /// いまGPUに載っているジオメトリの元(観測点, キー, メッシュ原点)の並び。
    uploaded: Vec<(u64, CoverageKey, Origin)>,
}

/// 観測点の範囲(最大観測範囲)に重なる、いま出している地形のチャンクのレベルの組を1つの値にしたもの。
/// これが変わったら、標高のサンプリング結果が変わるので覆域を計算し直す。範囲の外のチャンクは含めない。
fn terrain_signature(
    terrain: &TerrainData,
    resident: &HashMap<TileKey, TileLayout>,
    marker: &RadarMarker,
) -> u64 {
    let lat_span = marker.max_range_m / 111_000.0;
    let lon_span = lat_span / marker.lat_deg.to_radians().cos().max(0.05);
    let (lat0, lat1) = (marker.lat_deg - lat_span, marker.lat_deg + lat_span);
    let (lon0, lon1) = (marker.lon_deg - lon_span, marker.lon_deg + lon_span);
    let k = terrain.chunks_per_tile();
    let step = 1.0 / k as f64;

    let mut hasher = DefaultHasher::new();
    for tile_lat in lat0.floor() as i32..=lat1.floor() as i32 {
        for tile_lon in lon0.floor() as i32..=lon1.floor() as i32 {
            match resident.get(&(tile_lat, tile_lon)) {
                Some(TileLayout::Chunks(levels)) => {
                    for (c, &level) in levels.iter().enumerate() {
                        let chunk_lat = tile_lat as f64 + (c / k) as f64 * step;
                        let chunk_lon = tile_lon as f64 + (c % k) as f64 * step;
                        let overlaps = chunk_lat < lat1
                            && chunk_lat + step > lat0
                            && chunk_lon < lon1
                            && chunk_lon + step > lon0;
                        if overlaps {
                            (tile_lat, tile_lon, c, level).hash(&mut hasher);
                        }
                    }
                }
                Some(TileLayout::Whole) => (tile_lat, tile_lon, u8::MAX).hash(&mut hasher),
                None => (tile_lat, tile_lon, u8::MAX - 1).hash(&mut hasher),
            }
        }
    }
    hasher.finish()
}

/// 計算結果から頂点を作る(すでに同じ内容(キーとメッシュ原点)のものがあれば何もしない)。
fn ensure_built(
    entry: &mut MarkerCoverage,
    terrain: &TerrainData,
    mesh_origin: Origin,
    key: &CoverageKey,
    data: &CoverageData,
) {
    if entry
        .built
        .as_ref()
        .is_some_and(|b| &b.key == key && b.origin == mesh_origin)
    {
        return;
    }
    let geometry = match data {
        CoverageData::Dome(rings) => Geometry::Dome(markers::dome_geometry(
            terrain,
            &mesh_origin,
            &key.marker,
            rings,
        )),
        CoverageData::Area(points) => Geometry::Area(markers::coverage_2d_geometry(
            terrain,
            &mesh_origin,
            &key.marker,
            points,
        )),
    };
    entry.built = Some(BuiltGeometry {
        key: key.clone(),
        origin: mesh_origin,
        geometry,
    });
}

/// 表示する観測点のジオメトリをつないで、GPUへ載せる(すでに同じ内容が載っていれば何もしない)。
/// 3Dはドーム、2Dは領域を載せ、使わない方は空にする。
fn sync_gpu(s: &mut ViewState) {
    let mode = s.camera.mode;
    let mut parts: Vec<(u64, CoverageKey, Origin)> = Vec::new();
    for id in &s.coverage.visible {
        let Some(built) = s.coverage.markers.get(id).and_then(|m| m.built.as_ref()) else {
            continue;
        };
        let matches_mode = matches!(
            (&built.geometry, mode),
            (Geometry::Dome(_), ViewMode::ThreeD) | (Geometry::Area(_), ViewMode::TwoD)
        );
        if matches_mode {
            parts.push((*id, built.key.clone(), built.origin));
        }
    }
    if parts == s.coverage.uploaded {
        return;
    }
    let mut dome: Vec<TerrainVertex> = Vec::new();
    let mut area: Vec<DrawVertex> = Vec::new();
    for (id, _, _) in &parts {
        match s
            .coverage
            .markers
            .get(id)
            .and_then(|m| m.built.as_ref())
            .map(|b| &b.geometry)
        {
            Some(Geometry::Dome(v)) => dome.extend_from_slice(v),
            Some(Geometry::Area(v)) => area.extend_from_slice(v),
            None => {}
        }
    }
    let Some(renderer) = s.renderer.as_mut() else {
        return;
    };
    renderer.update_dome(&dome);
    renderer.update_coverage_2d(&area);
    s.coverage.uploaded = parts;
}

/// 表示する観測点の覆域(3Dはドーム、2Dは探知可能領域)を、現在の状態に合わせる。表示する観測点は、選択中の1つ、
/// `show_all_coverage`ならすべて。計算済みならジオメトリを作り直すだけ、未計算なら小分けの計算を始める
/// (終わったら自動で反映して描き直す)。表示しない観測点の進行中の計算は取り消す。
/// `terrain_changed`は、地形のレベルの切り替えをきっかけとした呼び出しか(その場合は少し待ってから計算を始める)。
/// 描画自体は呼び出し側で`render_now`(または`render_frame`)すること。
pub(super) fn refresh_coverage(state: &Rc<RefCell<ViewState>>, terrain_changed: bool) {
    let mut guard = state.borrow_mut();
    let s = &mut *guard;
    let radar_markers = s.radar_markers;
    let (Some(terrain), Some(mesh_origin)) = (s.terrain.clone(), s.mesh_origin) else {
        return;
    };
    let mode = s.camera.mode;
    let all: Vec<RadarMarker> = radar_markers.markers.get_untracked();
    let wanted: Vec<RadarMarker> = if radar_markers.show_all_coverage.get_untracked() {
        all.clone()
    } else {
        let selected = radar_markers.selected.get_untracked();
        all.iter()
            .filter(|m| Some(m.id) == selected)
            .copied()
            .collect()
    };
    let altitude_m = match mode {
        ViewMode::ThreeD => 0.0,
        ViewMode::TwoD => radar_markers.coverage_altitude_m.get_untracked(),
    };

    // 削除された観測点の状態は捨てる(進行中の計算は、状態が無いので結果を捨てる)。表示しない観測点の計算は取り消す。
    s.coverage
        .markers
        .retain(|id, _| all.iter().any(|m| m.id == *id));
    s.coverage.visible = wanted.iter().map(|m| m.id).collect();
    for (id, entry) in s.coverage.markers.iter_mut() {
        if entry.pending.is_some() && !s.coverage.visible.contains(id) {
            entry.generation += 1;
            entry.pending = None;
        }
    }

    let mut jobs: Vec<(CoverageKey, u64)> = Vec::new();
    for marker in &wanted {
        let key = CoverageKey {
            marker: *marker,
            mode,
            altitude_bits: altitude_m.to_bits(),
            terrain: terrain_signature(&terrain, &s.lod.resident, marker),
        };
        let entry = s.coverage.markers.entry(marker.id).or_default();
        // 計算済み(同じ観測点・モード・高度・地形): ジオメトリだけ作り直す。進行中の計算は要らない。
        if let Some(data) = entry
            .cache
            .as_ref()
            .filter(|c| c.key == key)
            .map(|c| c.data.clone())
        {
            entry.generation += 1;
            entry.pending = None;
            ensure_built(entry, &terrain, mesh_origin, &key, &data);
            continue;
        }
        // 同じ要求の計算が進行中: そのまま待つ。
        if entry.pending.as_ref() == Some(&key) {
            continue;
        }
        // 新しい計算を始める。地形以外(観測点・モード・高度)が変わったなら、前の覆域は正しくないので出さない。
        // 地形だけが変わったなら、計算が終わるまで前の覆域を出しておく(ちらつかない)。
        if entry
            .built
            .as_ref()
            .is_some_and(|b| !b.key.same_request(&key))
        {
            entry.built = None;
        }
        entry.generation += 1;
        entry.pending = Some(key.clone());
        jobs.push((key, entry.generation));
    }
    sync_gpu(s);
    drop(guard);

    for (key, generation) in jobs {
        spawn_job(
            state.clone(),
            terrain.clone(),
            key,
            generation,
            altitude_m,
            terrain_changed,
        );
    }
}

/// 観測点1つの覆域を、小分けに非同期で計算し、終わったらキャッシュ・ジオメトリ・GPUへ反映して描き直す。
fn spawn_job(
    state: Rc<RefCell<ViewState>>,
    terrain: Rc<TerrainData>,
    key: CoverageKey,
    generation: u64,
    altitude_m: f64,
    terrain_changed: bool,
) {
    let id = key.marker.id;
    wasm_bindgen_futures::spawn_local(async move {
        if terrain_changed {
            gloo_timers::future::TimeoutFuture::new(TERRAIN_DEBOUNCE_MS).await;
        }
        let is_stale = {
            let state = state.clone();
            move || {
                state
                    .borrow()
                    .coverage
                    .markers
                    .get(&id)
                    .is_none_or(|m| m.generation != generation)
            }
        };
        if is_stale() {
            return;
        }
        let data = match key.mode {
            ViewMode::ThreeD => {
                let mut computation = markers::start_dome_computation(&terrain, &key.marker);
                let finished = run_in_slices(
                    || computation.advance(&terrain, AZIMUTHS_PER_STEP),
                    &is_stale,
                )
                .await;
                finished.then(|| CoverageData::Dome(computation.finish()))
            }
            ViewMode::TwoD => {
                let mut computation =
                    markers::start_coverage_computation(&terrain, &key.marker, altitude_m);
                let finished = run_in_slices(
                    || computation.advance(&terrain, AZIMUTHS_PER_STEP),
                    &is_stale,
                )
                .await;
                finished.then(|| CoverageData::Area(computation.finish()))
            }
        };
        let Some(data) = data else {
            return;
        };
        {
            let mut guard = state.borrow_mut();
            let s = &mut *guard;
            let mesh_origin = s.mesh_origin;
            let Some(entry) = s.coverage.markers.get_mut(&id) else {
                return;
            };
            if entry.generation != generation {
                return;
            }
            entry.pending = None;
            let data = Rc::new(data);
            entry.cache = Some(CoverageCache {
                key: key.clone(),
                data: data.clone(),
            });
            if let Some(mesh_origin) = mesh_origin {
                ensure_built(entry, &terrain, mesh_origin, &key, &data);
            }
            sync_gpu(s);
        }
        render_frame(&state);
    });
}
