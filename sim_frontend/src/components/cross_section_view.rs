//! 右パネル下部: 側面図(地形断面図)。中央地図の3D視点ではなく、マップ原点を起点に
//! スライダーで指定した方位角方向の地表断面を2D(距離 vs 標高の折れ線)で表示する。
//! 地形データ本体は`TerrainStore`を通じて中央地図と共有する(フェッチは1回だけ)。

use leptos::prelude::*;

use crate::terrain::mesh::Origin;
use crate::terrain::profile::{build_profile, ProfilePoint};
use crate::terrain::store::TerrainStore;
use crate::ws::WsSignals;

const VIEW_W: f64 = 400.0;
const VIEW_H: f64 = 220.0;
const PAD_L: f64 = 46.0;
const PAD_R: f64 = 10.0;
const PAD_T: f64 = 10.0;
const PAD_B: f64 = 22.0;

fn build_svg_paths(points: &[ProfilePoint]) -> Option<(String, String, f32, f32, f64)> {
    if points.len() < 2 {
        return None;
    }
    let max_distance = points.last().unwrap().distance_m.max(1.0);
    let min_elev = points
        .iter()
        .map(|p| p.elevation_m)
        .fold(f32::INFINITY, f32::min);
    let max_elev = points
        .iter()
        .map(|p| p.elevation_m)
        .fold(f32::NEG_INFINITY, f32::max);
    let elev_range = (max_elev - min_elev).max(1.0);

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

    Some((line_d, area_d, min_elev, max_elev, max_distance))
}

#[component]
pub fn CrossSectionView() -> impl IntoView {
    let signals = use_context::<WsSignals>().expect("WsSignals context not found");
    let terrain_store = use_context::<TerrainStore>().expect("TerrainStore context not found");
    terrain_store.ensure_loaded();

    // 方位角(度)。0=北、90=東、時計回り。
    let azimuth = RwSignal::new(0.0_f64);

    let chart = move || {
        let Some(data) = terrain_store.get() else {
            return view! { <p class="placeholder cross-section-status">"地形データを読み込み中..."</p> }
                .into_any();
        };
        let origin = signals
            .origin
            .get()
            .map(|o| Origin { lat_deg: o.lat_deg, lon_deg: o.lon_deg })
            .unwrap_or(Origin {
                lat_deg: data.metadata.default_origin.lat_deg,
                lon_deg: data.metadata.default_origin.lon_deg,
            });

        let points = build_profile(&data, &origin, azimuth.get());
        let Some((line_d, area_d, min_elev, max_elev, max_distance)) = build_svg_paths(&points)
        else {
            return view! { <p class="placeholder cross-section-status">"断面を計算できません"</p> }
                .into_any();
        };

        view! {
            <svg
                class="cross-section-svg"
                viewBox=format!("0 0 {VIEW_W} {VIEW_H}")
                preserveAspectRatio="none"
                role="img"
                aria-label="地形断面図"
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
                <path d=area_d class="cs-area"></path>
                <path d=line_d class="cs-line" fill="none"></path>
                <text x=PAD_L - 4.0 y=PAD_T + 4.0 text-anchor="end" class="cs-label">
                    {format!("{:.0}m", max_elev)}
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
                    {move || format!("{:.0}\u{b0}", azimuth.get())}
                </span>
            </div>
        </div>
    }
}
