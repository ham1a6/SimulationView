//! 航跡(トラック)の橋渡し: `sample/sim_server`のプロトコル(`protocol::TrackList`)を、通信を知らない
//! sim3dviewライブラリの`terrain::tracks::TracksState`へ反映する。ライブラリを自分のシミュレータへつなぐときの
//! 実装例(受信した値を`Track`へ変換して`TracksState::set`へ渡すだけ)。
//!
//! 位置・方位の単位は最初から同じ(緯度経度は度、方位は北から時計回りの度、速度はm/s、ピッチ・ロールは度)なので、
//! 変換は種別・所属・高度基準の数値をライブラリの列挙型へ読み替えるだけ。

use leptos::prelude::*;

use sim3dview::terrain::drawing::Altitude;
use sim3dview::terrain::tracks::{Affiliation, SymbolKind, Track, TracksState};

use crate::protocol;
use crate::ws::WsSignals;

/// 受信した1トラックを、ライブラリの表示用`Track`へ変換する(値の意味は`protocol.hpp`のenumと一致)。
fn to_library_track(t: &protocol::Track) -> Track {
    Track {
        id: u64::from(t.id),
        kind: match t.kind {
            1 => SymbolKind::Aircraft,
            2 => SymbolKind::Helicopter,
            3 => SymbolKind::Ship,
            4 => SymbolKind::Vehicle,
            5 => SymbolKind::Missile,
            _ => SymbolKind::Unknown,
        },
        affiliation: match t.affiliation {
            1 => Affiliation::Friendly,
            2 => Affiliation::Hostile,
            3 => Affiliation::Neutral,
            _ => Affiliation::Unknown,
        },
        label: t.label.clone(),
        lat_deg: t.lat_deg,
        lon_deg: t.lon_deg,
        altitude: if t.alt_ref == 1 { Altitude::AboveGround(t.alt_m) } else { Altitude::Msl(t.alt_m) },
        heading_deg: t.heading_deg,
        speed_mps: t.speed_mps,
        pitch_deg: t.pitch_deg,
        roll_deg: t.roll_deg,
    }
}

/// サーバーから届く`TrackList`を`TracksState`へ反映するEffectを登録する(`App`から1回だけ呼ぶ)。
/// 一覧は毎回全トラックの最新状態なので、そのまま`set`で置き換える(消えたトラックは航跡ごと消える)。
pub fn bridge_tracks(signals: WsSignals, tracks: TracksState) {
    Effect::new(move |_| {
        if let Some(list) = signals.track_list.get() {
            tracks.set(list.tracks.iter().map(to_library_track).collect());
        }
    });
}
