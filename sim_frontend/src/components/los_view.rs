//! ボトムステータスパネルの「見通し範囲」タブ。原点を観測点として、全方位角の
//! 見通し限界距離(地形遮蔽 + 等価地球半径を考慮)を2D極座標図(レーダー覆域図)として表示する。
//! アンテナ高・最大観測範囲はパラメータとして入力欄から指定できる。

use leptos::prelude::*;

use crate::terrain::los::{compute_los, LosParams, LosPoint};
use crate::terrain::mesh::Origin;
use crate::terrain::store::TerrainStore;
use crate::ws::WsSignals;

const VIEW_SIZE: f64 = 300.0;
const PAD: f64 = 26.0;
const RADIUS: f64 = (VIEW_SIZE - PAD * 2.0) / 2.0;
const CENTER: f64 = VIEW_SIZE / 2.0;

fn build_boundary_path(points: &[LosPoint], max_range_m: f64) -> String {
    let max_range = max_range_m.max(1.0);
    let mut d = String::new();
    for (i, p) in points.iter().enumerate() {
        let r = (p.range_m / max_range).clamp(0.0, 1.0) * RADIUS;
        let az_rad = p.azimuth_deg.to_radians();
        let x = CENTER + r * az_rad.sin();
        let y = CENTER - r * az_rad.cos();
        if i == 0 {
            d.push_str(&format!("M{x:.1},{y:.1}"));
        } else {
            d.push_str(&format!(" L{x:.1},{y:.1}"));
        }
    }
    d.push_str(" Z");
    d
}

#[component]
pub fn LosView() -> impl IntoView {
    let signals = use_context::<WsSignals>().expect("WsSignals context not found");
    let terrain_store = use_context::<TerrainStore>().expect("TerrainStore context not found");
    terrain_store.ensure_loaded();

    let observer_height_m = RwSignal::new(10.0_f64);
    let max_range_km = RwSignal::new(50.0_f64);

    let chart = move || {
        let Some(data) = terrain_store.get() else {
            return view! { <p class="placeholder los-status">"地形データを読み込み中..."</p> }
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

        let max_range_m = max_range_km.get().max(0.001) * 1000.0;
        let params = LosParams { observer_height_m: observer_height_m.get(), max_range_m };
        let points = compute_los(&data, &origin, &params);
        if points.is_empty() {
            return view! { <p class="placeholder los-status">"計算できません"</p> }.into_any();
        }
        let boundary_d = build_boundary_path(&points, max_range_m);

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
                        view! {
                            <circle cx=CENTER cy=CENTER r=RADIUS * frac class="los-grid-circle"></circle>
                        }
                    })
                    .collect::<Vec<_>>()}
                <line x1=CENTER y1=CENTER - RADIUS x2=CENTER y2=CENTER + RADIUS class="los-axis"></line>
                <line x1=CENTER - RADIUS y1=CENTER x2=CENTER + RADIUS y2=CENTER class="los-axis"></line>
                <path d=boundary_d.clone() class="los-area"></path>
                <path d=boundary_d class="los-outline" fill="none"></path>
                <circle cx=CENTER cy=CENTER r=3.0 class="los-observer"></circle>
                <text x=CENTER y=CENTER - RADIUS - 6.0 text-anchor="middle" class="los-label">
                    "N"
                </text>
                <text x=CENTER + RADIUS + 8.0 y=CENTER + 4.0 class="los-label">
                    "E"
                </text>
                <text x=CENTER y=CENTER + RADIUS + 14.0 text-anchor="middle" class="los-label">
                    "S"
                </text>
                <text x=CENTER - RADIUS - 8.0 y=CENTER + 4.0 text-anchor="end" class="los-label">
                    "W"
                </text>
                <text x=CENTER + 4.0 y=CENTER - RADIUS + 11.0 class="los-label">
                    {move || format!("{:.0}km", max_range_km.get())}
                </text>
            </svg>
        }
        .into_any()
    };

    view! {
        <div class="los-view">
            <div class="los-chart">{chart}</div>
            <div class="los-controls">
                <label class="los-param-label">
                    "アンテナ高(m)"
                    <input
                        type="number"
                        min="0"
                        step="1"
                        prop:value=move || observer_height_m.get().to_string()
                        on:input=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<f64>() {
                                observer_height_m.set(v.max(0.0));
                            }
                        }
                    />
                </label>
                <label class="los-param-label">
                    "最大観測範囲(km)"
                    <input
                        type="number"
                        min="1"
                        step="1"
                        prop:value=move || max_range_km.get().to_string()
                        on:input=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<f64>() {
                                max_range_km.set(v.max(1.0));
                            }
                        }
                    />
                </label>
            </div>
        </div>
    }
}
