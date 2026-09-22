//! トップステータスパネル(`right_panel.rs`)の「航跡情報」タブの中身。地図上でクリックして選択した航跡
//! (`sim3dview::terrain::tracks::TracksState::selected`)の詳細を表示する。
//!
//! ライブラリは「どの航跡が選択されたか」(`selected`)と、その最新の状態(`selected_track`)を渡すだけで、
//! 何をどう表示するかはアプリの役目。ここでは、サーバーから届く値(名前・種別・所属・位置・高度・針路・速度)に加えて、
//! 原点(シミュレーションの基準点)からの距離・方位を計算して出す。位置が更新されるたびに、表示も追従する。

use leptos::prelude::*;

use sim3dview::terrain::drawing::Altitude;
use sim3dview::terrain::origin::OriginState;
use sim3dview::terrain::tracks::{Track, TracksState};

const COMPASS: [&str; 16] = [
    "北",
    "北北東",
    "北東",
    "東北東",
    "東",
    "東南東",
    "南東",
    "南南東",
    "南",
    "南南西",
    "南西",
    "西南西",
    "西",
    "西北西",
    "北西",
    "北北西",
];

/// 方位(度、北から時計回り)の16方位の名前。
fn compass_name(deg: f64) -> &'static str {
    COMPASS[((deg.rem_euclid(360.0) / 22.5) + 0.5).floor() as usize % 16]
}

/// 点(lat0, lon0)から点(lat1, lon1)への、球面(平均半径6371km)の大円距離(メートル)と初期方位(度、北から時計回り)。
fn distance_and_bearing(lat0: f64, lon0: f64, lat1: f64, lon1: f64) -> (f64, f64) {
    const EARTH_RADIUS_M: f64 = 6_371_000.0;
    let (p0, p1) = (lat0.to_radians(), lat1.to_radians());
    let dlon = (lon1 - lon0).to_radians();
    let a = ((p1 - p0) * 0.5).sin().powi(2) + p0.cos() * p1.cos() * (dlon * 0.5).sin().powi(2);
    let distance = 2.0 * EARTH_RADIUS_M * a.sqrt().min(1.0).asin();
    let bearing =
        (dlon.sin() * p1.cos()).atan2(p0.cos() * p1.sin() - p0.sin() * p1.cos() * dlon.cos());
    (distance, bearing.to_degrees().rem_euclid(360.0))
}

fn format_latitude(lat: f64) -> String {
    format!("{:.6}°{}", lat.abs(), if lat >= 0.0 { "N" } else { "S" })
}

fn format_longitude(lon: f64) -> String {
    format!("{:.6}°{}", lon.abs(), if lon >= 0.0 { "E" } else { "W" })
}

fn format_altitude(altitude: Altitude) -> String {
    match altitude {
        Altitude::Msl(h) => format!("{h:.0} m(海抜)"),
        Altitude::AboveGround(o) => format!("{o:.0} m(地表から)"),
    }
}

/// 針路・速度・原点からの位置関係など、値から作る表示文字列(名前・種別などは`view!`で直接書く)。
fn derived_rows(track: &Track, origin: Option<(f64, f64)>) -> Vec<(&'static str, String)> {
    let mut rows = vec![
        (
            "針路",
            format!(
                "{:03.0}°({})",
                track.heading_deg.rem_euclid(360.0),
                compass_name(track.heading_deg)
            ),
        ),
        (
            "速度",
            format!(
                "{:.0} m/s = {:.0} km/h = {:.0} kt",
                track.speed_mps,
                track.speed_mps * 3.6,
                track.speed_mps * 1.943_844
            ),
        ),
    ];
    if let Some((lat0, lon0)) = origin {
        let (distance, bearing) = distance_and_bearing(lat0, lon0, track.lat_deg, track.lon_deg);
        rows.push((
            "原点から",
            format!(
                "{:.1} km / 方位{:03.0}°({})",
                distance / 1000.0,
                bearing,
                compass_name(bearing)
            ),
        ));
    }
    rows
}

#[component]
pub fn TrackDetail() -> impl IntoView {
    let tracks = use_context::<TracksState>().expect("TracksState context not found");
    let origin = use_context::<OriginState>().expect("OriginState context not found");

    view! {
        <div class="track-detail">
            {move || match tracks.selected_track() {
                None => view! {
                    <p class="placeholder">"地図上の航跡のシンボルをクリックすると、詳細が表示されます。"</p>
                }
                .into_any(),
                Some(track) => {
                    let color = track.affiliation.color();
                    let chip = format!(
                        "background: rgb({},{},{})",
                        (color.r * 255.0) as u8,
                        (color.g * 255.0) as u8,
                        (color.b * 255.0) as u8
                    );
                    let origin_position = origin.0.get().map(|o| (o.lat_deg, o.lon_deg));
                    let rows = derived_rows(&track, origin_position);
                    view! {
                        <dl class="kv-list">
                            <dt>"名前"</dt>
                            <dd class="track-detail-name">{track.label.clone()}</dd>
                            <dt>"識別番号"</dt>
                            <dd>{track.id}</dd>
                            <dt>"種別"</dt>
                            <dd>{track.kind.label()}</dd>
                            <dt>"所属"</dt>
                            <dd>
                                <span class="track-chip" style=chip></span>
                                {track.affiliation.label()}
                            </dd>
                            <dt>"位置"</dt>
                            <dd>
                                {format_latitude(track.lat_deg)}
                                <br/>
                                {format_longitude(track.lon_deg)}
                            </dd>
                            <dt>"高度"</dt>
                            <dd>{format_altitude(track.altitude)}</dd>
                            {rows
                                .into_iter()
                                .map(|(label, value)| view! { <dt>{label}</dt><dd>{value}</dd> })
                                .collect::<Vec<_>>()}
                        </dl>
                        <button class="sim-button track-deselect" on:click=move |_| tracks.select(None)>
                            "選択を解除"
                        </button>
                    }
                    .into_any()
                }
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compass_names_cover_all_directions() {
        assert_eq!(compass_name(0.0), "北");
        assert_eq!(compass_name(90.0), "東");
        assert_eq!(compass_name(202.4), "南南西");
        assert_eq!(compass_name(359.0), "北");
        assert_eq!(compass_name(-90.0), "西");
    }

    #[test]
    fn distance_and_bearing_are_consistent() {
        // 赤道上で東へ1度: 約111.2km・真東。
        let (d, b) = distance_and_bearing(0.0, 0.0, 0.0, 1.0);
        assert!((d - 111_195.0).abs() < 100.0, "d={d}");
        assert!((b - 90.0).abs() < 1e-6, "b={b}");
        // 真北へ0.5度: 約55.6km・方位0。
        let (d, b) = distance_and_bearing(35.0, 139.0, 35.5, 139.0);
        assert!((d - 55_597.0).abs() < 100.0, "d={d}");
        assert!(b.abs() < 1e-6 || (b - 360.0).abs() < 1e-6, "b={b}");
        // 同じ点: 距離0。
        assert!(distance_and_bearing(35.0, 139.0, 35.0, 139.0).0 < 1e-6);
    }

    #[test]
    fn formats_are_readable() {
        assert_eq!(format_latitude(-12.5), "12.500000°S");
        assert_eq!(format_longitude(139.5), "139.500000°E");
        assert_eq!(format_altitude(Altitude::Msl(4000.4)), "4000 m(海抜)");
        assert_eq!(
            format_altitude(Altitude::AboveGround(120.0)),
            "120 m(地表から)"
        );
    }
}
