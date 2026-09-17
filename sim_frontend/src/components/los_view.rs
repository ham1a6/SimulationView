//! ボトムステータスパネルの「見通し範囲」タブ。任意の地点(既定はシミュレーション原点)を
//! アンテナ位置として、全方位角の見通し限界距離(地形遮蔽 + 等価地球半径を考慮)を
//! 2D極座標図(レーダー覆域図)として表示する。アンテナ位置(緯度経度)・アンテナ高・
//! 最大観測範囲はいずれもパラメータとして入力欄から指定できる。
//! アンテナ位置はシミュレーション本体の原点(`OriginState`)とは独立した、この画面だけの
//! ローカルな状態(`set_origin`コマンドは送らない。地形は再計算しないため、原点変更と違い
//! シミュレーション停止中でなくても自由に動かせる)。

use leptos::prelude::*;

use crate::terrain::loader::GeodeticBounds;
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

    // アンテナ位置(緯度経度、文字列入力)。既定ではシミュレーション原点に合わせておくが、
    // ユーザーが手で編集したらそれ以降は原点変更による自動上書きを止める(origin_dialog.rsの
    // dirtyフラグと同じ考え方)。
    let antenna_lat_input = RwSignal::new(String::new());
    let antenna_lon_input = RwSignal::new(String::new());
    let antenna_dirty = RwSignal::new(false);

    Effect::new(move |_| {
        if antenna_dirty.get_untracked() {
            return;
        }
        if let Some(o) = signals.origin.get() {
            antenna_lat_input.set(format!("{:.6}", o.lat_deg));
            antenna_lon_input.set(format!("{:.6}", o.lon_deg));
        } else if let Some(data) = terrain_store.get() {
            antenna_lat_input.set(format!("{:.6}", data.metadata.default_origin.lat_deg));
            antenna_lon_input.set(format!("{:.6}", data.metadata.default_origin.lon_deg));
        }
    });

    let use_origin = move |_| {
        antenna_dirty.set(false);
        if let Some(o) = signals.origin.get_untracked() {
            antenna_lat_input.set(format!("{:.6}", o.lat_deg));
            antenna_lon_input.set(format!("{:.6}", o.lon_deg));
        }
    };

    // アンテナ位置の入力値を解析し、地形データ範囲内かを検証する。
    let parsed_antenna = move |bounds: &GeodeticBounds| -> Result<Origin, String> {
        let lat: f64 = antenna_lat_input
            .get()
            .trim()
            .parse()
            .map_err(|_| "アンテナ緯度は数値で入力してください".to_string())?;
        let lon: f64 = antenna_lon_input
            .get()
            .trim()
            .parse()
            .map_err(|_| "アンテナ経度は数値で入力してください".to_string())?;
        if lat < bounds.min_lat || lat > bounds.max_lat {
            return Err(format!(
                "アンテナ緯度は{:.1}〜{:.1}の範囲で入力してください",
                bounds.min_lat, bounds.max_lat
            ));
        }
        if lon < bounds.min_lon || lon > bounds.max_lon {
            return Err(format!(
                "アンテナ経度は{:.1}〜{:.1}の範囲で入力してください",
                bounds.min_lon, bounds.max_lon
            ));
        }
        Ok(Origin { lat_deg: lat, lon_deg: lon })
    };

    let chart = move || {
        let Some(data) = terrain_store.get() else {
            return view! { <p class="placeholder los-status">"地形データを読み込み中..."</p> }
                .into_any();
        };
        let antenna = match parsed_antenna(&data.metadata.geodetic_bounds) {
            Ok(o) => o,
            Err(msg) => return view! { <p class="placeholder los-status">{msg}</p> }.into_any(),
        };

        let max_range_m = max_range_km.get().max(0.001) * 1000.0;
        let params = LosParams { observer_height_m: observer_height_m.get(), max_range_m };
        let points = compute_los(&data, &antenna, &params);
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
                <div class="los-antenna-row">
                    <label class="los-param-label">
                        "アンテナ緯度"
                        <input
                            type="text"
                            inputmode="decimal"
                            prop:value=move || antenna_lat_input.get()
                            on:input=move |ev| {
                                antenna_dirty.set(true);
                                antenna_lat_input.set(event_target_value(&ev));
                            }
                        />
                    </label>
                    <label class="los-param-label">
                        "アンテナ経度"
                        <input
                            type="text"
                            inputmode="decimal"
                            prop:value=move || antenna_lon_input.get()
                            on:input=move |ev| {
                                antenna_dirty.set(true);
                                antenna_lon_input.set(event_target_value(&ev));
                            }
                        />
                    </label>
                    <button class="los-use-origin" title="原点の緯度経度に戻す" on:click=use_origin>
                        "原点を使う"
                    </button>
                </div>
                <div class="los-antenna-row">
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
        </div>
    }
}
