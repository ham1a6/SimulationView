//! レーダー観測点(見通し範囲)の一覧・選択・削除・パラメータ(アンテナ高/最大観測範囲)編集と、
//! 選択中のレーダーの見通し範囲(2D極座標図)の表示を行うタブコンポーネント。観測点自体は
//! ここでは追加せず、`ui::terrain_view::TerrainView`上での右クリック(右クリックメニューがあればその項目)で追加する想定。

use std::cell::Cell;
use std::rc::Rc;

use leptos::prelude::*;

use crate::terrain::los::{LosPoint, RangeComputation, RangeKind};
use crate::terrain::markers::{coverage_colors, RadarMarker, RadarMarkersState};
use crate::terrain::store::TerrainStore;
use crate::ui::util::{event_f64, push_path_point, run_in_slices};

/// 極座標図のSVGの一辺(`viewBox`の単位。正方形)。
const VIEW_SIZE: f64 = 300.0;
/// 図の外周から`viewBox`の端までの余白(方位の文字を置く)。
const PAD: f64 = 26.0;
/// 外周の円の半径(最大観測範囲に当たる)。
const RADIUS: f64 = (VIEW_SIZE - PAD * 2.0) / 2.0;
/// 図の中心(観測点)の座標(x・yとも)。
const CENTER: f64 = VIEW_SIZE / 2.0;
/// 極座標図の外周の上端(以下、下端・左端・右端)。
const TOP: f64 = CENTER - RADIUS;
/// 外周の下端。
const BOTTOM: f64 = CENTER + RADIUS;
/// 外周の左端。
const LEFT: f64 = CENTER - RADIUS;
/// 外周の右端。
const RIGHT: f64 = CENTER + RADIUS;
/// 方位の文字(N・E・S・W)と、最大観測範囲の文字の位置。これは「N」のy。
const N_LABEL_Y: f64 = TOP - 6.0;
/// 「S」のy。
const S_LABEL_Y: f64 = BOTTOM + 14.0;
/// 「E」のx。
const E_LABEL_X: f64 = RIGHT + 8.0;
/// 「W」のx。
const W_LABEL_X: f64 = LEFT - 8.0;
/// 「E」「W」のy(ベースラインなので中心より少し下)。
const EW_LABEL_Y: f64 = CENTER + 4.0;
/// 最大観測範囲の文字のx。
const RANGE_LABEL_X: f64 = CENTER + 4.0;
/// 最大観測範囲の文字のy(外周の上端のすぐ内側)。
const RANGE_LABEL_Y: f64 = TOP + 11.0;
/// 極座標図の方位の刻み(mil)。2なら3,200方位(50km先で約98m間隔)。図の大きさ(300px)に対して十分細かく、計算量が半分になる。
const CHART_AZIMUTH_STEP: usize = 2;
/// 計算を1回に進める方位の数(`run_in_slices`の持ち時間の中で繰り返し呼ぶ)。
const AZIMUTHS_PER_STEP: usize = 4;

/// 観測点の覆域の色の見本(左半分=3Dドーム、右半分=2D覆域)のCSS。
fn swatch_style(marker_id: u64) -> String {
    let (dome, area, _) = coverage_colors(marker_id);
    let rgb = |c: [f32; 3]| {
        format!(
            "rgb({:.0},{:.0},{:.0})",
            c[0] * 255.0,
            c[1] * 255.0,
            c[2] * 255.0
        )
    };
    format!(
        "background: linear-gradient(90deg, {} 50%, {} 50%)",
        rgb(dome),
        rgb(area)
    )
}

/// 方位ごとの見通し範囲の端を結んだ閉じたSVGパス。中心から北=上・東=右に、距離を最大観測範囲に
/// 対する割合で外周の半径へ当てはめる(外周より外は外周に収める)。
fn build_boundary_path(points: &[LosPoint], max_range_m: f64) -> String {
    let max_range = max_range_m.max(1.0);
    let mut d = String::new();
    for (i, p) in points.iter().enumerate() {
        let r = (p.range_m / max_range).clamp(0.0, 1.0) * RADIUS;
        let az_rad = p.azimuth_deg.to_radians();
        let point = (CENTER + r * az_rad.sin(), CENTER - r * az_rad.cos());
        push_path_point(&mut d, i == 0, point);
    }
    d.push_str(" Z");
    d
}

/// レーダー観測点の一覧・編集と、選択中の観測点の見通し範囲の極座標図のタブ。
/// `TerrainStore`・`RadarMarkersState`のcontextが必要。
#[component]
pub fn LosView() -> impl IntoView {
    let terrain_store = use_context::<TerrainStore>().expect("TerrainStore context not found");
    let radar_markers =
        use_context::<RadarMarkersState>().expect("RadarMarkersState context not found");
    terrain_store.ensure_loaded();

    // 図・一覧の代わりに出す状態の文言。
    let status = |message: &'static str| {
        view! { <p class="placeholder los-status">{message}</p> }.into_any()
    };
    // 観測点`id`のパラメータの入力欄の`input`ハンドラ(数値なら`set(観測点, 値)`で書き換える)。
    let param_input = move |id: u64, set: fn(&mut RadarMarker, f64)| {
        move |ev: leptos::ev::Event| {
            if let Some(v) = event_f64(&ev) {
                radar_markers.update(id, |marker| set(marker, v));
            }
        }
    };

    let list_view = move || {
        let list = radar_markers.markers.get();
        let selected = radar_markers.selected.get();
        if list.is_empty() {
            return status(
                "メインパネル(中央の地図)を右クリックして、レーダー観測点を追加してください",
            );
        }
        list.into_iter()
            .map(|m| {
                let id = m.id;
                let is_selected = selected == Some(id);
                let row_class = if is_selected {
                    "los-marker-row los-marker-row-selected"
                } else {
                    "los-marker-row"
                };
                let range_km = m.max_range_m / 1000.0;
                view! {
                    <div class=row_class on:click=move |_| radar_markers.selected.set(Some(id))>
                        <span class="los-swatch" title="覆域の色(左=3D、右=2D)" style=swatch_style(id)></span>
                        <span class="los-marker-label">
                            {format!("緯度{:.4} 経度{:.4}", m.lat_deg, m.lon_deg)}
                        </span>
                        <label class="los-param-label">
                            "高(m)"
                            <input
                                type="number"
                                min="0"
                                step="1"
                                prop:value=m.height_m.to_string()
                                // 入力欄のクリックで行の選択が動かないようにする。
                                on:click=move |ev| ev.stop_propagation()
                                on:input=param_input(id, |marker, v| marker.height_m = v.max(0.0))
                            />
                        </label>
                        <label class="los-param-label">
                            "範囲(km)"
                            <input
                                type="number"
                                min="1"
                                step="1"
                                prop:value=range_km.to_string()
                                on:click=move |ev| ev.stop_propagation()
                                on:input=param_input(id, |marker, v| marker.max_range_m = v.max(1.0) * 1000.0)
                            />
                        </label>
                        <button
                            class="los-delete-btn"
                            title="このレーダーを削除"
                            on:click=move |ev| {
                                ev.stop_propagation();
                                radar_markers.remove(id);
                            }
                        >
                            "削除"
                        </button>
                    </div>
                }
            })
            .collect::<Vec<_>>()
            .into_any()
    };

    // 選択中のレーダー(パラメータの編集も含めて、変わったときだけ計算し直す)。
    let selected_marker = Memo::new(move |_| {
        let id = radar_markers.selected.get()?;
        radar_markers
            .markers
            .with(|list| list.iter().find(|m| m.id == id).copied())
    });
    // 見通し範囲の計算結果。計算は重いので、小分けにして非同期で進める(画面が固まらないように)。
    let result: RwSignal<Option<(RadarMarker, Vec<LosPoint>)>> = RwSignal::new(None);
    // 計算の世代。選択・地形が変わるたびに増やし、古い計算は自分の世代でなくなったら止まる。
    let generation = Rc::new(Cell::new(0u64));
    Effect::new(move |_| {
        let marker = selected_marker.get();
        let data = terrain_store.get();
        // 進行中の計算は取り消す(世代が変わったら止まる)。
        let this_generation = generation.get() + 1;
        generation.set(this_generation);
        result.set(None);
        let (Some(marker), Some(data)) = (marker, data) else {
            return;
        };
        let generation = generation.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let mut computation = RangeComputation::new(
                &data,
                &marker.origin(),
                &marker.los_params(),
                RangeKind::Visible,
                CHART_AZIMUTH_STEP,
            );
            let finished = run_in_slices(
                || computation.advance(&data, AZIMUTHS_PER_STEP),
                || generation.get() != this_generation,
            )
            .await;
            if finished {
                result.set(Some((marker, computation.finish())));
            }
        });
    });

    let chart = move || {
        if terrain_store.get().is_none() {
            return status("地形データを読み込み中...");
        }
        let Some(marker) = selected_marker.get() else {
            return status("レーダーが選択されていません");
        };
        // 選択やパラメータが変わった直後は、前の結果が残っていることがある(計算し直している間)ので、
        // いまの選択の結果だけを使う。
        let points = match result.get() {
            Some((computed, points)) if computed == marker => points,
            _ => return status("見通し範囲を計算中..."),
        };
        if points.is_empty() {
            return status("計算できません");
        }
        let boundary_d = build_boundary_path(&points, marker.max_range_m);
        let max_range_km = marker.max_range_m / 1000.0;

        view! {
            <svg
                class="los-svg"
                viewBox=format!("0 0 {VIEW_SIZE} {VIEW_SIZE}")
                role="img"
                aria-label="見通し範囲図(レーダー覆域図)"
            >
                {[0.25, 0.5, 0.75, 1.0]
                    .into_iter()
                    .map(|frac| {
                        let r = RADIUS * frac;
                        view! { <circle cx=CENTER cy=CENTER r=r class="los-grid-circle"></circle> }
                    })
                    .collect::<Vec<_>>()}
                <line x1=CENTER y1=TOP x2=CENTER y2=BOTTOM class="los-axis"></line>
                <line x1=LEFT y1=CENTER x2=RIGHT y2=CENTER class="los-axis"></line>
                <path d=boundary_d.clone() class="los-area"></path>
                <path d=boundary_d class="los-outline" fill="none"></path>
                <circle cx=CENTER cy=CENTER r=3.0 class="los-observer"></circle>
                <text x=CENTER y=N_LABEL_Y text-anchor="middle" class="los-label">
                    "N"
                </text>
                <text x=E_LABEL_X y=EW_LABEL_Y class="los-label">
                    "E"
                </text>
                <text x=CENTER y=S_LABEL_Y text-anchor="middle" class="los-label">
                    "S"
                </text>
                <text x=W_LABEL_X y=EW_LABEL_Y text-anchor="end" class="los-label">
                    "W"
                </text>
                <text x=RANGE_LABEL_X y=RANGE_LABEL_Y class="los-label">
                    {format!("{:.0}km", max_range_km)}
                </text>
            </svg>
        }
        .into_any()
    };

    view! {
        <div class="los-view">
            <div class="los-chart">{chart}</div>
            <div class="los-controls">
                <label class="los-option">
                    <input
                        id="los-show-all-coverage"
                        type="checkbox"
                        prop:checked=move || radar_markers.show_all_coverage.get()
                        on:change=move |ev| radar_markers.show_all_coverage.set(event_target_checked(&ev))
                    />
                    "すべての観測点の覆域を同時に表示"
                </label>
                <div class="los-marker-list">{list_view}</div>
            </div>
        </div>
    }
}
