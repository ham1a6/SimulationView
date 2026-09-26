//! 現在の基準位置(原点)。`ui::terrain_view::TerrainView`はこのcontextを読んで
//! メッシュ・カメラ注視点を再計算する(原点が変わるたびに)。`ui::origin_dialog::OriginDialog`は
//! これを表示・編集する。
//!
//! どのプロトコル・どの通信手段で原点を配信するかはライブラリの関心事ではないため、
//! ここでは`RwSignal<Option<Origin>>`を保持するだけの薄いラッパーにしてある。呼び出し側
//! (アプリ)が自前のプロトコルから受け取った値をこのシグナルへ反映する。

use leptos::prelude::*;
use serde::Deserialize;

/// 基準位置(原点)。DETAILED_DESIGN.md 3.1節。地形の`metadata.json`の`default_origin`もこの形で読む。
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Origin {
    /// 緯度(度。北が正)。
    pub lat_deg: f64,
    /// 経度(度。東が正)。
    pub lon_deg: f64,
}

/// 現在の原点。`None`はまだ受信/設定されていない状態
/// (`TerrainView`は地形データの`default_origin`にフォールバックする)。
#[derive(Clone, Copy)]
pub struct OriginState(pub RwSignal<Option<Origin>>);
