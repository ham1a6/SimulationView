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
use crate::terrain::los::{DomeRing, LosPoint};
use crate::terrain::lod::TileLayout;
use crate::terrain::markers::{self, RadarMarker, RadarMarkersState};
use crate::terrain::origin::Origin;
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
        self.marker == other.marker && self.mode == other.mode && self.altitude_bits == other.altitude_bits
    }
}

struct CoverageCache {
    key: CoverageKey,
    data: Rc<CoverageData>,
}

/// `ViewState`が持つ覆域の状態。
#[derive(Default)]
pub(super) struct CoverageState {
    cache: Option<CoverageCache>,
    /// いまGPUに載っているジオメトリの元(計算結果のキーと、頂点を作ったメッシュ原点)。
    shown: Option<(CoverageKey, Origin)>,
    /// 計算中(待ち時間を含む)のキー。
    pending: Option<CoverageKey>,
    /// 計算の世代。新しい要求・取り消しのたびに増やし、古い計算は自分の世代でなくなったら止まる。
    generation: u64,
}

/// 観測点の範囲(最大観測範囲)に重なる、いま出している地形のチャンクのレベルの組を1つの値にしたもの。
/// これが変わったら、標高のサンプリング結果が変わるので覆域を計算し直す。範囲の外のチャンクは含めない。
fn terrain_signature(terrain: &TerrainData, resident: &HashMap<TileKey, TileLayout>, marker: &RadarMarker) -> u64 {
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

/// 覆域をGPUから外す。進行中の計算も取り消す。
fn clear_coverage(s: &mut ViewState) {
    s.coverage.generation += 1;
    s.coverage.pending = None;
    if s.coverage.shown.take().is_some() {
        if let Some(renderer) = s.renderer.as_mut() {
            renderer.update_dome(&[]);
            renderer.update_coverage_2d(&[]);
        }
    }
}

/// 計算結果から頂点を作ってGPUへ載せる(すでに同じ内容が載っていれば何もしない)。
fn show_coverage(
    s: &mut ViewState,
    terrain: &TerrainData,
    mesh_origin: Origin,
    key: &CoverageKey,
    data: &CoverageData,
) {
    if s.coverage.shown.as_ref().is_some_and(|(k, o)| k == key && *o == mesh_origin) {
        return;
    }
    let Some(renderer) = s.renderer.as_mut() else {
        return;
    };
    match data {
        CoverageData::Dome(rings) => {
            renderer.update_dome(&markers::dome_geometry(terrain, &mesh_origin, &key.marker, rings));
            renderer.update_coverage_2d(&[]);
        }
        CoverageData::Area(points) => {
            renderer.update_dome(&[]);
            renderer.update_coverage_2d(&markers::coverage_2d_geometry(terrain, &mesh_origin, &key.marker, points));
        }
    }
    s.coverage.shown = Some((key.clone(), mesh_origin));
}

/// 選択中の観測点の覆域(3Dはドーム、2Dは探知可能領域)を、現在の状態に合わせる。計算済みならジオメトリを作り直すだけ、
/// 未計算なら小分けの計算を始める(終わったら自動で反映して描き直す)。観測点が選択されていなければ覆域を消す。
/// `terrain_changed`は、地形のレベルの切り替えをきっかけとした呼び出しか(その場合は少し待ってから計算を始める)。
/// 描画自体は呼び出し側で`render_now`(または`render_frame`)すること。
pub(super) fn refresh_coverage(state: &Rc<RefCell<ViewState>>, radar_markers: RadarMarkersState, terrain_changed: bool) {
    let mut guard = state.borrow_mut();
    let s = &mut *guard;
    let (Some(terrain), Some(mesh_origin)) = (s.terrain.clone(), s.mesh_origin) else {
        return;
    };
    let mode = s.camera.mode;
    let marker = radar_markers.selected.get_untracked().and_then(|id| {
        radar_markers.markers.with_untracked(|list| list.iter().find(|m| m.id == id).copied())
    });
    let Some(marker) = marker else {
        clear_coverage(s);
        return;
    };
    let altitude_m = match mode {
        ViewMode::ThreeD => 0.0,
        ViewMode::TwoD => radar_markers.coverage_altitude_m.get_untracked(),
    };
    let key = CoverageKey {
        marker,
        mode,
        altitude_bits: altitude_m.to_bits(),
        terrain: terrain_signature(&terrain, &s.resident, &marker),
    };

    // 計算済み(同じ観測点・モード・高度・地形): ジオメトリだけ作り直す。進行中の計算は要らない。
    if let Some(cache) = s.coverage.cache.as_ref().filter(|c| c.key == key) {
        let data = cache.data.clone();
        s.coverage.generation += 1;
        s.coverage.pending = None;
        show_coverage(s, &terrain, mesh_origin, &key, &data);
        return;
    }
    // 同じ要求の計算が進行中: そのまま待つ。
    if s.coverage.pending.as_ref() == Some(&key) {
        return;
    }

    // 新しい計算を始める。地形以外(観測点・モード・高度)が変わったなら、前の覆域は正しくないので消す。
    // 地形だけが変わったなら、計算が終わるまで前の覆域を出しておく(ちらつかない)。
    if s.coverage.shown.as_ref().is_some_and(|(shown, _)| !shown.same_request(&key)) {
        clear_coverage(s);
    }
    s.coverage.generation += 1;
    s.coverage.pending = Some(key.clone());
    let generation = s.coverage.generation;
    drop(guard);

    let state = state.clone();
    wasm_bindgen_futures::spawn_local(async move {
        if terrain_changed {
            gloo_timers::future::TimeoutFuture::new(TERRAIN_DEBOUNCE_MS).await;
        }
        let is_stale = {
            let state = state.clone();
            move || state.borrow().coverage.generation != generation
        };
        if is_stale() {
            return;
        }
        let data = match key.mode {
            ViewMode::ThreeD => {
                let mut computation = markers::start_dome_computation(&terrain, &key.marker);
                let finished = run_in_slices(|| computation.advance(&terrain, AZIMUTHS_PER_STEP), &is_stale).await;
                finished.then(|| CoverageData::Dome(computation.finish()))
            }
            ViewMode::TwoD => {
                let mut computation = markers::start_coverage_computation(&terrain, &key.marker, altitude_m);
                let finished = run_in_slices(|| computation.advance(&terrain, AZIMUTHS_PER_STEP), &is_stale).await;
                finished.then(|| CoverageData::Area(computation.finish()))
            }
        };
        let Some(data) = data else {
            return;
        };
        {
            let mut guard = state.borrow_mut();
            let s = &mut *guard;
            if s.coverage.generation != generation {
                return;
            }
            s.coverage.pending = None;
            let data = Rc::new(data);
            s.coverage.cache = Some(CoverageCache { key: key.clone(), data: data.clone() });
            if let Some(mesh_origin) = s.mesh_origin {
                show_coverage(s, &terrain, mesh_origin, &key, &data);
            }
        }
        render_frame(&state);
    });
}
