//! 右パネル下部: 断面図(地形断面図)。中央地図の3D視点ではなく、**中心**を通る、スライダーで指定した
//! 方位角の直線に沿った地表断面を2D(距離 vs 標高の折れ線)で表示する。中心は、この画面内の
//! **コンボボックスで選んだ航跡**の位置(地図上のシンボルクリックで変わる`TracksState::selected`とは
//! 独立。以前はそちらを見ていたが、要望により「選択したものではなく、コンボボックスで選べる」形にした)。
//! 何も選んでいなければ基準位置(マップ原点)。中心が距離0で、方位角の向きが正、
//! 反対が負。片側の長さは選べる(`RANGE_OPTIONS_KM`)。選んだ航跡は、断面の中心に、高度の位置で印を付ける。
//! 地形データ本体は`TerrainStore`を通じて中央地図と共有する(フェッチは1回だけ)。
//! メインパネル上で追加したレーダー観測点(`RadarMarkersState`)がいずれか1つでも見通せる
//! 断面上の区間は、地表トラックを緑の線で(`compute_coverage`)、さらに**上空を含めた
//! 探知可能領域**を緑の塗りつぶし(`compute_airspace_boundary`)で示す。塗りつぶしの下端は
//! 各点の「これ以上の高度なら見える」という下限高度(`terrain::los::min_visible_altitude`)で、
//! 上端はグラフ表示用の便宜的な上限(`SKY_MARGIN_M`)。

use leptos::prelude::*;

use crate::terrain::drawing::Altitude;
use crate::terrain::loader::TerrainData;
use crate::terrain::los::{is_visible, min_visible_altitude};
use crate::terrain::markers::{RadarMarker, RadarMarkersState};
use crate::terrain::origin::Origin;
use crate::terrain::origin::OriginState;
use crate::terrain::profile::{build_profile_span, ProfilePoint};
use crate::terrain::store::TerrainStore;
use crate::terrain::tracks::{Track, TrackId, TracksState};

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
/// 選択中のシンボルの高度が入るように上端を広げるとき、シンボルの上に取る余裕(m)。
const SYMBOL_HEADROOM_M: f32 = 1_500.0;

/// 断面上の点群から実際の標高範囲(最小・最大)を求める。
fn elevation_bounds(points: &[ProfilePoint]) -> (f32, f32) {
    let min_elev = points
        .iter()
        .map(|p| p.elevation_m)
        .fold(f32::INFINITY, f32::min);
    let max_elev = points
        .iter()
        .map(|p| p.elevation_m)
        .fold(f32::NEG_INFINITY, f32::max);
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
    let start = points[0].distance_m;

    let to_xy = |p: &ProfilePoint| -> (f64, f64) {
        let x = PAD_L + ((p.distance_m - start) / max_distance) * plot_w;
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
fn compute_coverage(
    data: &TerrainData,
    points: &[ProfilePoint],
    markers: &[RadarMarker],
) -> Vec<bool> {
    points
        .iter()
        .map(|p| {
            markers
                .iter()
                .any(|m| is_visible(data, &m.origin(), &m.los_params(), p.lat_deg, p.lon_deg))
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
    let start = points.first().map_or(0.0, |p| p.distance_m);
    let to_xy = |p: &ProfilePoint| -> (f64, f64) {
        let x = PAD_L + ((p.distance_m - start) / max_distance) * plot_w;
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
                    min_visible_altitude(data, &m.origin(), &m.los_params(), p.lat_deg, p.lon_deg)
                })
                .fold(None, |acc: Option<f64>, h| {
                    Some(acc.map_or(h, |a| a.min(h)))
                })
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
    let start = points.first().map_or(0.0, |p| p.distance_m);
    let x_of = |distance: f64| PAD_L + ((distance - start) / max_distance) * plot_w;
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

/// 断面の長さ(中心から片側)の選択肢(km)。
const RANGE_OPTIONS_KM: [f64; 6] = [10.0, 25.0, 50.0, 100.0, 200.0, 500.0];
/// 断面の長さ(片側)の既定(km)。
const DEFAULT_RANGE_KM: f64 = 100.0;
/// 中心(選択中のシンボル)がこれだけ(度。約500m)動くまでは、断面を作り直さない。シンボルは毎秒何度も動き、
/// 断面(覆域の判定を含む)の計算は重いので、動くたびに作り直さない。シンボルの印(位置・高度)は、断面とは別に、動きに追従して描く。
const CENTER_STEP_DEG: f64 = 0.005;

/// 計算済みの断面(SVGのパスと、目盛りに使う値)。
#[derive(Clone)]
struct Section {
    line_d: String,
    area_d: String,
    coverage_d: Option<String>,
    airspace_d: Option<String>,
    min_elev: f32,
    max_elev: f32,
    /// グラフの上端(空側)の標高。選択中のシンボルの高度が入るように広げることがある。
    sky_ceiling: f32,
    /// 中心から、方位角の反対側・方位角の向きの長さ(m)。
    back_m: f64,
    forward_m: f64,
    /// 中心の地表の標高(`AboveGround`のシンボルの高度を海抜にするのに使う)。
    center_ground_m: f32,
}

#[derive(Clone)]
enum SectionState {
    Loading,
    Failed,
    Ready(Section),
}

/// シンボルの海抜高度(m)。`AboveGround`は、中心(=シンボルの位置)の地表の標高に足す。
fn symbol_altitude_msl(track: &Track, center_ground_m: f32) -> f32 {
    match track.altitude {
        Altitude::Msl(h) => h as f32,
        Altitude::AboveGround(offset) => center_ground_m + offset as f32,
    }
}

#[component]
pub fn CrossSectionView() -> impl IntoView {
    let origin_state = use_context::<OriginState>().expect("OriginState context not found");
    let terrain_store = use_context::<TerrainStore>().expect("TerrainStore context not found");
    let radar_markers =
        use_context::<RadarMarkersState>().expect("RadarMarkersState context not found");
    // 未提供なら、いつも原点(基準位置)が中心(上と同じく後付けのオプション機能)。
    let tracks = use_context::<TracksState>();
    terrain_store.ensure_loaded();

    // 方位角(度)。0=北、90=東、時計回り。
    let azimuth = RwSignal::new(0.0_f64);
    // 中心から片側の長さ(km)。
    let range_km = RwSignal::new(DEFAULT_RANGE_KM);

    // 断面の中心にする航跡のID(コンボボックスで選ぶ。Noneなら基準位置)。地図上のシンボル
    // クリックによる`TracksState::selected`とは別の、この画面だけのローカルな選択状態。
    let center_track_id = RwSignal::new(None::<TrackId>);
    // 選んだ航跡が一覧から消えたら(サーバー側で消滅)、選択を解除する(`TracksState::set`が
    // 自分の`selected`にしているのと同じ扱い)。`entries`を読む(トラッキングする)ので、
    // 一覧が届くたびに走るが、実際に変えるのは消えたときだけ。
    if let Some(tracks) = tracks {
        Effect::new(move |_| {
            let still_exists = center_track_id.get_untracked().is_none_or(|id| {
                tracks
                    .entries
                    .with(|es| es.iter().any(|e| e.track.id == id))
            });
            if !still_exists {
                center_track_id.set(None);
            }
        });
    }
    // コンボボックスの選択肢(id・ラベル)。位置は~20Hzで更新されるが、選択肢自体(トラックの
    // 増減・ラベル)が実際に変わったときだけ下流(`<select>`の再構築)へ通知したいので、
    // `Memo`にする(Leptosの`Memo`は計算結果が前回と同じなら通知しない)。
    let track_options = Memo::new(move |_| {
        tracks
            .map(|t| {
                t.entries.with(|es| {
                    es.iter()
                        .map(|e| (e.track.id, e.track.label.clone()))
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default()
    });
    // 中心にする航跡の現在値(リアクティブに追跡する版・しない版)。断面の再計算・シンボルの
    // 印の両方から使う(`TracksState::selected_track`/`selected_track_untracked`と同じ使い分け)。
    let center_track = move || -> Option<Track> {
        let id = center_track_id.get()?;
        tracks?.entries.with(|es| {
            es.iter()
                .find(|e| e.track.id == id)
                .map(|e| e.track.clone())
        })
    };
    let center_track_untracked = move || -> Option<Track> {
        let id = center_track_id.get_untracked()?;
        tracks?.entries.with_untracked(|es| {
            es.iter()
                .find(|e| e.track.id == id)
                .map(|e| e.track.clone())
        })
    };

    // 選んだ航跡の、断面を作り直す単位に丸めた位置。選択の変更・一定以上の移動のときだけ変わる。
    let center_key = Memo::new(move |_| {
        let track = center_track()?;
        Some((
            track.id,
            (track.lat_deg / CENTER_STEP_DEG).round() as i64,
            (track.lon_deg / CENTER_STEP_DEG).round() as i64,
        ))
    });

    // 断面(地表の折れ線・覆域)。重いので、中心・方位角・長さ・観測点・地形が変わったときだけ作り直す。
    let section = RwSignal::new(SectionState::Loading);
    Effect::new(move |_| {
        let Some(data) = terrain_store.get() else {
            section.set(SectionState::Loading);
            return;
        };
        let _ = center_key.get();
        let azimuth_deg = azimuth.get();
        let range_m = range_km.get() * 1000.0;
        let markers = radar_markers.markers.get();
        let base = origin_state.0.get().unwrap_or(Origin {
            lat_deg: data.metadata.default_origin.lat_deg,
            lon_deg: data.metadata.default_origin.lon_deg,
        });
        // 中心: コンボボックスで選んだ航跡の位置、無ければ基準位置(原点)。位置の更新では作り直さない
        // (`center_key`が刻む)ので、追跡しないで読む。
        let track = center_track_untracked();
        let center = track.as_ref().map_or(base, |t| Origin {
            lat_deg: t.lat_deg,
            lon_deg: t.lon_deg,
        });

        let points = build_profile_span(&data, &center, azimuth_deg, range_m, range_m);
        if points.len() < 2 {
            section.set(SectionState::Failed);
            return;
        }
        let span = (points.last().unwrap().distance_m - points[0].distance_m).max(1.0);
        let (min_elev, max_elev) = elevation_bounds(&points);
        let center_ground_m = points
            .iter()
            .min_by(|a, b| a.distance_m.abs().total_cmp(&b.distance_m.abs()))
            .map_or(min_elev, |p| p.elevation_m);
        // シンボルが上空にいるときは、高度が入るまで上端を広げる。
        let mut sky_ceiling = max_elev + SKY_MARGIN_M;
        if let Some(t) = &track {
            sky_ceiling =
                sky_ceiling.max(symbol_altitude_msl(t, center_ground_m) + SYMBOL_HEADROOM_M);
        }

        let Some((line_d, area_d)) = build_svg_paths(&points, min_elev, sky_ceiling, span) else {
            section.set(SectionState::Failed);
            return;
        };
        let (coverage_d, airspace_d) = if markers.is_empty() {
            (None, None)
        } else {
            let covered = compute_coverage(&data, &points, &markers);
            let ground_d = build_coverage_path(&points, &covered, min_elev, sky_ceiling, span);
            let boundary = compute_airspace_boundary(&data, &points, &markers);
            let sky_d = build_airspace_path(&points, &boundary, min_elev, sky_ceiling, span);
            (ground_d, sky_d)
        };
        section.set(SectionState::Ready(Section {
            line_d,
            area_d,
            coverage_d,
            airspace_d,
            min_elev,
            max_elev,
            sky_ceiling,
            back_m: -points[0].distance_m,
            forward_m: points.last().unwrap().distance_m,
            center_ground_m,
        }));
    });

    let chart = move || {
        let sec = match section.get() {
            SectionState::Loading => {
                return view! { <p class="placeholder cross-section-status">"地形データを読み込み中..."</p> }
                    .into_any();
            }
            SectionState::Failed => {
                return view! { <p class="placeholder cross-section-status">"断面を計算できません"</p> }.into_any();
            }
            SectionState::Ready(sec) => sec,
        };
        let plot_w = VIEW_W - PAD_L - PAD_R;
        let plot_h = VIEW_H - PAD_T - PAD_B;
        let elev_range = (sec.sky_ceiling - sec.min_elev).max(1.0);
        let y_of = |elev: f32| {
            PAD_T + plot_h - ((elev - sec.min_elev) as f64 / elev_range as f64) * plot_h
        };
        let max_elev_y = y_of(sec.max_elev);
        let span = (sec.back_m + sec.forward_m).max(1.0);
        // 中心(距離0)の位置。
        let center_x = PAD_L + (sec.back_m / span) * plot_w;

        // 選んだ航跡のシンボル: 中心の真上に、高度の位置で印を付ける(位置・高度の更新には、断面を作り直さずに追従する)。
        let symbol = center_track().map(|track| {
            let altitude = symbol_altitude_msl(&track, sec.center_ground_m);
            let y = y_of(altitude.clamp(sec.min_elev, sec.sky_ceiling));
            let ground_y = y_of(sec.center_ground_m);
            let label = format!("{} {:.0}m", track.label, altitude);
            // ラベルは、右端にはみ出さないよう、中心が右寄りなら左側に出す。
            let (label_x, anchor) = if center_x > VIEW_W * 0.6 { (center_x - 6.0, "end") } else { (center_x + 6.0, "start") };
            view! {
                <line x1=center_x y1=ground_y x2=center_x y2=y class="cs-symbol-line"></line>
                <circle cx=center_x cy=y r=4.0 class="cs-symbol"></circle>
                <text x=label_x y=(y - 6.0).max(PAD_T + 8.0) text-anchor=anchor class="cs-symbol-label">
                    {label}
                </text>
            }
        });

        view! {
            <svg
                class="cross-section-svg"
                viewBox=format!("0 0 {VIEW_W} {VIEW_H}")
                preserveAspectRatio="none"
                role="img"
                aria-label="地形断面図(覆域は上空を含む)"
            >
                <line x1=PAD_L y1=PAD_T x2=PAD_L y2=PAD_T + plot_h class="cs-axis"></line>
                <line x1=PAD_L y1=PAD_T + plot_h x2=VIEW_W - PAD_R y2=PAD_T + plot_h class="cs-axis"></line>
                {sec.airspace_d.map(|d| view! { <path d=d class="cs-airspace"></path> })}
                <path d=sec.area_d class="cs-area"></path>
                <path d=sec.line_d class="cs-line" fill="none"></path>
                {sec.coverage_d.map(|d| view! { <path d=d class="cs-coverage" fill="none"></path> })}
                <line x1=center_x y1=PAD_T x2=center_x y2=PAD_T + plot_h class="cs-center"></line>
                {symbol}
                <line x1=PAD_L - 3.0 y1=max_elev_y x2=PAD_L y2=max_elev_y class="cs-axis"></line>
                <text x=PAD_L - 4.0 y=max_elev_y + 3.0 text-anchor="end" class="cs-label">
                    {format!("{:.0}m", sec.max_elev)}
                </text>
                <text x=PAD_L - 4.0 y=PAD_T + 8.0 text-anchor="end" class="cs-label">
                    {format!("+{:.0}m", sec.sky_ceiling - sec.max_elev)}
                </text>
                <text x=PAD_L - 4.0 y=PAD_T + plot_h text-anchor="end" class="cs-label">
                    {format!("{:.0}m", sec.min_elev)}
                </text>
                <text x=PAD_L y=VIEW_H - 4.0 text-anchor="start" class="cs-label">
                    {format!("-{:.1}km", sec.back_m / 1000.0)}
                </text>
                <text x=center_x y=VIEW_H - 4.0 text-anchor="middle" class="cs-label">"0"</text>
                <text x=VIEW_W - PAD_R y=VIEW_H - 4.0 text-anchor="end" class="cs-label">
                    {format!("+{:.1}km", sec.forward_m / 1000.0)}
                </text>
            </svg>
        }
        .into_any()
    };

    let has_track = move || center_track().is_some();

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
            <div class="cross-section-controls cross-section-center-row">
                <label class="cross-section-range-label cross-section-center-label" title="断面の中心にする航跡。選ばなければ基準位置(原点)">
                    "中心"
                    <select
                        id="cross-section-center"
                        on:change=move |ev| {
                            let v = event_target_value(&ev);
                            center_track_id.set(if v.is_empty() { None } else { v.parse::<TrackId>().ok() });
                        }
                    >
                        <option value="" selected=move || center_track_id.get().is_none()>
                            "基準位置"
                        </option>
                        {move || {
                            track_options
                                .get()
                                .into_iter()
                                .map(|(id, label)| {
                                    view! {
                                        <option value=id.to_string() selected=move || center_track_id.get() == Some(id)>
                                            {label}
                                        </option>
                                    }
                                })
                                .collect::<Vec<_>>()
                        }}
                    </select>
                </label>
                <label class="cross-section-range-label">
                    "片側"
                    <select
                        id="cross-section-range"
                        on:change=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<f64>() {
                                range_km.set(v);
                            }
                        }
                    >
                        {RANGE_OPTIONS_KM
                            .iter()
                            .map(|&km| {
                                view! {
                                    <option value=km.to_string() selected=move || range_km.get() == km>
                                        {format!("{km:.0}km")}
                                    </option>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </select>
                </label>
                <button
                    class="cross-section-heading-btn"
                    title="方位角を、選んだ航跡の進行方向に合わせる"
                    disabled=move || !has_track()
                    on:click=move |_| {
                        if let Some(track) = center_track_untracked() {
                            azimuth.set(track.heading_deg.rem_euclid(360.0).round() % 360.0);
                        }
                    }
                >
                    "進行方向"
                </button>
            </div>
        </div>
    }
}
