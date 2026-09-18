//! 右パネル下部: 断面図(地形断面図)。中央地図の3D視点ではなく、マップ原点を起点に
//! スライダーで指定した方位角方向の地表断面を2D(距離 vs 標高の折れ線)で表示する。
//! 地形データ本体は`TerrainStore`を通じて中央地図と共有する(フェッチは1回だけ)。
//! メインパネル上で追加したレーダー観測点(`RadarMarkersState`)がいずれか1つでも見通せる
//! 断面上の区間は、地表トラックを緑の線で(`compute_coverage`)、さらに**上空を含めた
//! 探知可能領域**を緑の塗りつぶし(`compute_airspace_boundary`)で示す。塗りつぶしの下端は
//! 各点の「これ以上の高度なら見える」という下限高度(`terrain::los::min_visible_altitude`)で、
//! 上端はグラフ表示用の便宜的な上限(`SKY_MARGIN_M`)。

use leptos::prelude::*;

use crate::terrain::loader::TerrainData;
use crate::terrain::los::{is_visible, min_visible_altitude};
use crate::terrain::markers::{RadarMarker, RadarMarkersState};
use crate::terrain::mesh::Origin;
use crate::terrain::origin::OriginState;
use crate::terrain::profile::{build_profile, ProfilePoint};
use crate::terrain::store::TerrainStore;

const VIEW_W: f64 = 400.0;
const VIEW_H: f64 = 220.0;
const PAD_L: f64 = 46.0;
const PAD_R: f64 = 10.0;
const PAD_T: f64 = 10.0;
const PAD_B: f64 = 22.0;

/// グラフの上端(空側)をどこまで表示するか、地表断面の最高標高からの上乗せ分(m)。
/// 覆域(上空を含む)の塗りつぶしが「空へ抜けている」ことが視覚的に分かる程度の余裕を
/// 持たせた固定値(v1ではUIパラメータ化していない)。
const SKY_MARGIN_M: f32 = 10_000.0;

/// 断面上の点群から実際の標高範囲(最小・最大)を求める。
fn elevation_bounds(points: &[ProfilePoint]) -> (f32, f32) {
    let min_elev = points.iter().map(|p| p.elevation_m).fold(f32::INFINITY, f32::min);
    let max_elev = points.iter().map(|p| p.elevation_m).fold(f32::NEG_INFINITY, f32::max);
    (min_elev, max_elev)
}

/// 地表断面(折れ線・塗りつぶし)のSVGパスを作る。`scale_max_elev`はY軸の上端に使う値で、
/// 実際の地表最高標高とは限らない(覆域の空側を見せるため`min_elev..sky_ceiling`を使う)。
fn build_svg_paths(
    points: &[ProfilePoint],
    min_elev: f32,
    scale_max_elev: f32,
    max_distance: f64,
) -> Option<(String, String)> {
    if points.len() < 2 {
        return None;
    }
    let elev_range = (scale_max_elev - min_elev).max(1.0);
    let plot_w = VIEW_W - PAD_L - PAD_R;
    let plot_h = VIEW_H - PAD_T - PAD_B;

    let to_xy = |p: &ProfilePoint| -> (f64, f64) {
        let x = PAD_L + (p.distance_m / max_distance) * plot_w;
        let y = PAD_T + plot_h - ((p.elevation_m - min_elev) as f64 / elev_range as f64) * plot_h;
        (x, y)
    };

    let mut line_d = String::new();
    for (i, p) in points.iter().enumerate() {
        let (x, y) = to_xy(p);
        if i == 0 {
            line_d.push_str(&format!("M{x:.1},{y:.1}"));
        } else {
            line_d.push_str(&format!(" L{x:.1},{y:.1}"));
        }
    }
    let area_d = format!(
        "{line_d} L{:.1},{:.1} L{:.1},{:.1} Z",
        PAD_L + plot_w,
        PAD_T + plot_h,
        PAD_L,
        PAD_T + plot_h
    );

    Some((line_d, area_d))
}

/// 断面上の各点が、配置済みレーダーのいずれかから見えるか(地表トラックが覆域内か)を判定する。
fn compute_coverage(data: &TerrainData, points: &[ProfilePoint], markers: &[RadarMarker]) -> Vec<bool> {
    points
        .iter()
        .map(|p| {
            markers.iter().any(|m| {
                is_visible(data, m.lat_deg, m.lon_deg, m.height_m, m.max_range_m, p.lat_deg, p.lon_deg)
            })
        })
        .collect()
}

/// 覆域内(いずれかのレーダーから見える)の区間だけをつないだ地表トラックの折れ線パスを作る。
/// 覆域が途切れる箇所では線をつながず、新しい部分パスとして分ける。
fn build_coverage_path(
    points: &[ProfilePoint],
    covered: &[bool],
    min_elev: f32,
    scale_max_elev: f32,
    max_distance: f64,
) -> Option<String> {
    let elev_range = (scale_max_elev - min_elev).max(1.0);
    let plot_w = VIEW_W - PAD_L - PAD_R;
    let plot_h = VIEW_H - PAD_T - PAD_B;
    let to_xy = |p: &ProfilePoint| -> (f64, f64) {
        let x = PAD_L + (p.distance_m / max_distance) * plot_w;
        let y = PAD_T + plot_h - ((p.elevation_m - min_elev) as f64 / elev_range as f64) * plot_h;
        (x, y)
    };

    let mut d = String::new();
    let mut in_run = false;
    for (p, &is_covered) in points.iter().zip(covered) {
        if is_covered {
            let (x, y) = to_xy(p);
            if in_run {
                d.push_str(&format!(" L{x:.1},{y:.1}"));
            } else {
                d.push_str(&format!("M{x:.1},{y:.1}"));
                in_run = true;
            }
        } else {
            in_run = false;
        }
    }
    if d.is_empty() {
        None
    } else {
        Some(d)
    }
}

/// 断面上の各点について、配置済みレーダーのうちいずれか1つでも見える最低高度(標高、m)を
/// 求める(=それぞれのレーダーの`min_visible_altitude`の最小値。1つでも見えれば覆域内なので、
/// 最も緩い=低い下限高度を採用する)。どのレーダーの最大観測範囲にも入らない点はNone
/// (この断面表示の上限高度まで見ても覆域外)。
fn compute_airspace_boundary(
    data: &TerrainData,
    points: &[ProfilePoint],
    markers: &[RadarMarker],
) -> Vec<Option<f64>> {
    points
        .iter()
        .map(|p| {
            markers
                .iter()
                .filter_map(|m| {
                    min_visible_altitude(
                        data,
                        m.lat_deg,
                        m.lon_deg,
                        m.height_m,
                        m.max_range_m,
                        p.lat_deg,
                        p.lon_deg,
                    )
                })
                .fold(None, |acc: Option<f64>, h| Some(acc.map_or(h, |a| a.min(h))))
        })
        .collect()
}

/// 覆域(上空を含む)の塗りつぶしパスを作る。各点で「下限高度(boundary)から表示上限
/// (sky_ceiling)まで」の帯を塗る。覆域が途切れる区間(boundaryがNoneまたはsky_ceiling以上)
/// では別の部分ポリゴンに分ける。
fn build_airspace_path(
    points: &[ProfilePoint],
    boundary: &[Option<f64>],
    min_elev: f32,
    sky_ceiling: f32,
    max_distance: f64,
) -> Option<String> {
    let elev_range = (sky_ceiling - min_elev).max(1.0);
    let plot_w = VIEW_W - PAD_L - PAD_R;
    let plot_h = VIEW_H - PAD_T - PAD_B;
    let x_of = |distance: f64| PAD_L + (distance / max_distance) * plot_w;
    let y_of = |elev: f32| PAD_T + plot_h - ((elev - min_elev) as f64 / elev_range as f64) * plot_h;
    let y_top = PAD_T;

    let mut d = String::new();
    let mut run: Vec<(f64, f64)> = Vec::new();

    for (p, b) in points.iter().zip(boundary) {
        match b {
            Some(h) if (*h as f32) < sky_ceiling => {
                let y = y_of((*h as f32).max(min_elev));
                run.push((x_of(p.distance_m), y));
            }
            _ => {
                flush_airspace_run(&mut run, y_top, &mut d);
            }
        }
    }
    flush_airspace_run(&mut run, y_top, &mut d);

    if d.is_empty() {
        None
    } else {
        Some(d)
    }
}

/// 連続する覆域区間(run)を、上端(sky_ceiling)を天井とする閉じたポリゴンとして`d`に追記する。
fn flush_airspace_run(run: &mut Vec<(f64, f64)>, y_top: f64, d: &mut String) {
    if run.len() < 2 {
        run.clear();
        return;
    }
    let (x0, _) = run[0];
    d.push_str(&format!("M{x0:.1},{y_top:.1}"));
    for &(x, y) in run.iter() {
        d.push_str(&format!(" L{x:.1},{y:.1}"));
    }
    let (x_last, _) = *run.last().unwrap();
    d.push_str(&format!(" L{x_last:.1},{y_top:.1} Z"));
    run.clear();
}

#[component]
pub fn CrossSectionView() -> impl IntoView {
    let origin_state = use_context::<OriginState>().expect("OriginState context not found");
    let terrain_store = use_context::<TerrainStore>().expect("TerrainStore context not found");
    let radar_markers = use_context::<RadarMarkersState>().expect("RadarMarkersState context not found");
    terrain_store.ensure_loaded();

    // 方位角(度)。0=北、90=東、時計回り。
    let azimuth = RwSignal::new(0.0_f64);

    let chart = move || {
        let Some(data) = terrain_store.get() else {
            return view! { <p class="placeholder cross-section-status">"地形データを読み込み中..."</p> }
                .into_any();
        };
        let origin = origin_state.0.get().unwrap_or(Origin {
            lat_deg: data.metadata.default_origin.lat_deg,
            lon_deg: data.metadata.default_origin.lon_deg,
        });

        let points = build_profile(&data, &origin, azimuth.get());
        if points.len() < 2 {
            return view! { <p class="placeholder cross-section-status">"断面を計算できません"</p> }
                .into_any();
        }
        let max_distance = points.last().unwrap().distance_m.max(1.0);
        let (min_elev, max_elev) = elevation_bounds(&points);
        let sky_ceiling = max_elev + SKY_MARGIN_M;

        let Some((line_d, area_d)) = build_svg_paths(&points, min_elev, sky_ceiling, max_distance) else {
            return view! { <p class="placeholder cross-section-status">"断面を計算できません"</p> }
                .into_any();
        };

        let markers = radar_markers.markers.get();
        let (coverage_d, airspace_d) = if markers.is_empty() {
            (None, None)
        } else {
            let covered = compute_coverage(&data, &points, &markers);
            let ground_d = build_coverage_path(&points, &covered, min_elev, sky_ceiling, max_distance);
            let boundary = compute_airspace_boundary(&data, &points, &markers);
            let sky_d = build_airspace_path(&points, &boundary, min_elev, sky_ceiling, max_distance);
            (ground_d, sky_d)
        };

        let plot_h = VIEW_H - PAD_T - PAD_B;
        let elev_range = (sky_ceiling - min_elev).max(1.0);
        let max_elev_y = PAD_T + plot_h - ((max_elev - min_elev) as f64 / elev_range as f64) * plot_h;

        view! {
            <svg
                class="cross-section-svg"
                viewBox=format!("0 0 {VIEW_W} {VIEW_H}")
                preserveAspectRatio="none"
                role="img"
                aria-label="地形断面図(覆域は上空を含む)"
            >
                <line
                    x1=PAD_L
                    y1=PAD_T
                    x2=PAD_L
                    y2=PAD_T + (VIEW_H - PAD_T - PAD_B)
                    class="cs-axis"
                ></line>
                <line
                    x1=PAD_L
                    y1=PAD_T + (VIEW_H - PAD_T - PAD_B)
                    x2=VIEW_W - PAD_R
                    y2=PAD_T + (VIEW_H - PAD_T - PAD_B)
                    class="cs-axis"
                ></line>
                {airspace_d.map(|d| view! { <path d=d class="cs-airspace"></path> })}
                <path d=area_d class="cs-area"></path>
                <path d=line_d class="cs-line" fill="none"></path>
                {coverage_d.map(|d| view! { <path d=d class="cs-coverage" fill="none"></path> })}
                <line
                    x1=PAD_L - 3.0
                    y1=max_elev_y
                    x2=PAD_L
                    y2=max_elev_y
                    class="cs-axis"
                ></line>
                <text x=PAD_L - 4.0 y=max_elev_y + 3.0 text-anchor="end" class="cs-label">
                    {format!("{:.0}m", max_elev)}
                </text>
                <text x=PAD_L - 4.0 y=PAD_T + 8.0 text-anchor="end" class="cs-label">
                    {format!("+{:.0}m", sky_ceiling - max_elev)}
                </text>
                <text
                    x=PAD_L - 4.0
                    y=PAD_T + (VIEW_H - PAD_T - PAD_B)
                    text-anchor="end"
                    class="cs-label"
                >
                    {format!("{:.0}m", min_elev)}
                </text>
                <text
                    x=VIEW_W - PAD_R
                    y=VIEW_H - 4.0
                    text-anchor="end"
                    class="cs-label"
                >
                    {format!("{:.1}km", max_distance / 1000.0)}
                </text>
            </svg>
        }
        .into_any()
    };

    view! {
        <div class="cross-section-view">
            <div class="cross-section-chart">{chart}</div>
            <div class="cross-section-controls">
                <label class="cross-section-slider-label">
                    "方位角(北=0°・時計回り)"
                    <input
                        type="range"
                        min="0"
                        max="359"
                        step="1"
                        prop:value=move || azimuth.get().to_string()
                        on:input=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<f64>() {
                                azimuth.set(v);
                            }
                        }
                    />
                </label>
                <span class="cross-section-azimuth-value">
                    {move || format!("{:.0}°", azimuth.get())}
                </span>
            </div>
        </div>
    }
}
