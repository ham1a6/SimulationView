//! 地図の右クリックメニューの項目(sim3dviewの`ui::context_menu::MapMenuState`に渡す、項目を作る関数)。
//! 右クリックした場所(地表の緯度経度・航跡のシンボル)から、そこに対する操作を並べる:
//! - 航跡のシンボル: 見出し(名前・種別・所属)、中心点をその航跡へ移す、選択の解除
//! - 地表: 緯度経度の表示、レーダー観測点の追加、原点の指定(シミュレーション停止中のみサーバーが受理)、
//!   中心点の移動、図形の作成(種類を選ぶサブメニュー)、緯度経度のコピー
//!
//! ライブラリは何を並べるかを知らない(項目はここで決める)。項目の中身は、ライブラリの各`State`(context)を呼ぶだけ。

use leptos::prelude::*;

use sim3dview::terrain::draw_tool::{DrawToolState, ToolKind};
use sim3dview::terrain::markers::RadarMarkersState;
use sim3dview::terrain::origin_pick::OriginPickState;
use sim3dview::terrain::recenter::RecenterRequestState;
use sim3dview::terrain::tracks::TracksState;
use sim3dview::ui::context_menu::MenuItem;
use sim3dview::ui::context_menu::{MapMenuState, MapMenuTarget};
use sim3dview::ui::util::copy_to_clipboard;

/// 項目が操作する状態。すべて`Copy`(シグナルの束)なので、項目のコールバックへそのまま持ち込める。
#[derive(Clone, Copy)]
struct Deps {
    radar_markers: RadarMarkersState,
    origin_pick: OriginPickState,
    recenter: RecenterRequestState,
    draw_tool: DrawToolState,
    tracks: TracksState,
}

/// 右クリックした場所に対するメニューの項目。
fn build_items(target: MapMenuTarget, d: Deps) -> Vec<MenuItem> {
    let mut items = Vec::new();

    if let Some(id) = target.track {
        let track = d.tracks.entries.with_untracked(|entries| {
            entries
                .iter()
                .find(|e| e.track.id == id)
                .map(|e| e.track.clone())
        });
        if let Some(track) = track {
            items.push(MenuItem::label(format!(
                "{}({}・{})",
                track.label,
                track.kind.label(),
                track.affiliation.label()
            )));
            let (lat, lon) = (track.lat_deg, track.lon_deg);
            items.push(MenuItem::action("中心点をこの航跡へ", move || {
                d.recenter.request_at(lat, lon)
            }));
            items.push(MenuItem::action("選択を解除", move || {
                d.tracks.select(None)
            }));
        }
    }

    if let Some((lat, lon)) = target.position {
        if !items.is_empty() {
            items.push(MenuItem::separator());
        }
        items.push(MenuItem::label(format!("緯度 {lat:.5}°  経度 {lon:.5}°")));
        items.push(MenuItem::action(
            "ここにレーダー観測点を追加",
            move || {
                d.radar_markers.add(lat, lon);
            },
        ));
        // 原点の変更はシミュレーション停止中のみサーバーが受理する(受理されなければCommandErrorが返る)。
        items.push(MenuItem::action("ここを原点に設定", move || {
            d.origin_pick.on_pick.run((lat, lon))
        }));
        items.push(MenuItem::action("ここを中心点にする", move || {
            d.recenter.request_at(lat, lon)
        }));
        items.push(MenuItem::separator());
        items.push(MenuItem::submenu(
            "ここに図形を作成",
            ToolKind::ALL
                .into_iter()
                .map(|kind| {
                    MenuItem::action(kind.label(), move || d.draw_tool.start_at(kind, lat, lon))
                })
                .collect(),
        ));
        items.push(MenuItem::action("緯度経度をコピー", move || {
            copy_to_clipboard(&format!("{lat:.6}, {lon:.6}"));
        }));
    }
    items
}

/// 地図の右クリックメニューを有効にする(`TerrainView`が`MapMenuState`を見て、右クリックでメニューを出す)。
/// 各`State`が`provide_context`されたあとに、1度だけ呼ぶこと。
pub fn provide_map_menu() {
    let deps = Deps {
        radar_markers: use_context::<RadarMarkersState>()
            .expect("RadarMarkersState context not found"),
        origin_pick: use_context::<OriginPickState>().expect("OriginPickState context not found"),
        recenter: use_context::<RecenterRequestState>()
            .expect("RecenterRequestState context not found"),
        draw_tool: use_context::<DrawToolState>().expect("DrawToolState context not found"),
        tracks: use_context::<TracksState>().expect("TracksState context not found"),
    };
    provide_context(MapMenuState::new(move |target| build_items(target, deps)));
}
