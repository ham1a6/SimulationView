//! ボトムステータスパネルの「見通し範囲」タブ。レーダー観測点はこのタブでは追加せず、
//! メインパネル(3D地形)上での右クリックで追加する(`components/terrain_view.rs`)。
//! このタブでは、追加済みのレーダー一覧の選択・削除・パラメータ(アンテナ高/最大観測範囲)編集と、
//! 選択中のレーダーの見通し範囲(2D極座標図)の表示を行う。

use leptos::prelude::*;

use crate::terrain::los::{compute_los, LosParams, LosPoint};
use crate::terrain::mesh::Origin;
use crate::terrain::store::TerrainStore;
use crate::ui_state::RadarMarkersState;

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
    let terrain_store = use_context::<TerrainStore>().expect("TerrainStore context not found");
    let radar_markers = use_context::<RadarMarkersState>().expect("RadarMarkersState context not found");
    terrain_store.ensure_loaded();

    let list_view = move || {
        let list = radar_markers.markers.get();
        let selected = radar_markers.selected.get();
        if list.is_empty() {
            return view! {
                <p class="placeholder los-status">
                    "メインパネル(中央の地図)上で右クリックしてレーダーを追加してください"
                </p>
            }
            .into_any();
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
                view! {
                    <div class=row_class on:click=move |_| radar_markers.selected.set(Some(id))>
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
                                on:click=move |ev| ev.stop_propagation()
                                on:input=move |ev| {
                                    if let Ok(v) = event_target_value(&ev).parse::<f64>() {
                                        radar_markers.markers.update(|list| {
                                            if let Some(marker) = list.iter_mut().find(|marker| marker.id == id) {
                                                marker.height_m = v.max(0.0);
                                            }
                                        });
                                    }
                                }
                            />
                        </label>
                        <label class="los-param-label">
                            "範囲(km)"
                            <input
                                type="number"
                                min="1"
                                step="1"
                                prop:value=(m.max_range_m / 1000.0).to_string()
                                on:click=move |ev| ev.stop_propagation()
                                on:input=move |ev| {
                                    if let Ok(v) = event_target_value(&ev).parse::<f64>() {
                                        radar_markers.markers.update(|list| {
                                            if let Some(marker) = list.iter_mut().find(|marker| marker.id == id) {
                                                marker.max_range_m = v.max(1.0) * 1000.0;
                                            }
                                        });
                                    }
                                }
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

    let chart = move || {
        let Some(data) = terrain_store.get() else {
            return view! { <p class="placeholder los-status">"地形データを読み込み中..."</p> }
                .into_any();
        };
        let Some(selected_id) = radar_markers.selected.get() else {
            return view! { <p class="placeholder los-status">"レーダーが選択されていません"</p> }
                .into_any();
        };
        let list = radar_markers.markers.get();
        let Some(marker) = list.into_iter().find(|m| m.id == selected_id) else {
            return view! { <p class="placeholder los-status">"レーダーが選択されていません"</p> }
                .into_any();
        };

        let origin = Origin { lat_deg: marker.lat_deg, lon_deg: marker.lon_deg };
        let params = LosParams { observer_height_m: marker.height_m, max_range_m: marker.max_range_m };
        let points = compute_los(&data, &origin, &params);
        if points.is_empty() {
            return view! { <p class="placeholder los-status">"計算できません"</p> }.into_any();
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
                <div class="los-marker-list">{list_view}</div>
            </div>
        </div>
    }
}
