//! 3Dモデル(glTF/GLB)表示: 航跡(`terrain::tracks`)のトラックを、シンボルの代わりに3Dモデルで描く。
//! DETAILED_DESIGN.md 6.13節。**おまけ機能**で、既存の航跡・作図・地形の描画には手を入れず、この
//! モジュールと対応する描画部(`renderer::model_batch`)・読み込みと更新(`ui::terrain_view::models`)に閉じている。
//!
//! アプリは`ModelsState`を`provide_context`し、トラックの種別(`SymbolKind`)ごとに使うモデルのURLを`set_source`で
//! 登録する(URLの組み立てはアプリの役目。ライブラリはサーバーの場所を知らない)。`TerrainView`は、モデルを
//! 登録した種別のトラックが現れたときにそのファイルを取得し、表示方式(`ModelDisplayMode`)に従って
//! 「モデルで描くトラック」を毎フレーム決める。モデルで描くトラックは、シンボルを描かない(航跡・高度線・ラベル・
//! 選択の輪はそのまま)。`ModelsState`が無い・モデルが未登録・取得に失敗した・読み込み中のときは、従来どおりシンボルで描く。
//!
//! - **表示方式**: `SwitchToSymbol`(既定。カメラからの距離が`switch_distance_m`以内ならモデル、それより遠ければシンボル)/
//!   `MinScreenSize`(常にモデル。画面での大きさが`min_screen_px`に満たないときは実寸より大きくして、その大きさを保証する)/
//!   `Off`(シンボルのみ)。
//! - **向き**: トラックのヘディング・ピッチ・ロール(`Track::heading_deg`等)。モデルは機体座標(x=右、y=前、z=上)で持つ。
//! - **モデルの作り方**: glTF 2.0のGLB。単位はメートル、前が+Z・上が+Y(glTFの規約)。原点が基準点(航空機は重心、艦船・車両は
//!   水線・接地面が便利)。対応する内容と制限は`gltf_import`。単位・向きが違うモデルは`ModelSource`の`scale`・`yaw_offset_deg`で直す。

mod gltf_import;
pub(crate) mod placement;
pub(crate) mod types;

use std::collections::HashMap;

use leptos::prelude::*;

use super::tracks::SymbolKind;
pub(crate) use gltf_import::import_glb;

/// 切替距離(`ModelsState::switch_distance_m`)の既定値(メートル)。実寸の航空機(十数m)がシンボル(約30px)に近い大きさで
/// 見える近さ(フルHDの縦幅で約500m)よりやや遠く。モデルの大きさに合わせて調整する(艦船は数km、車両は数百m)。
pub const DEFAULT_SWITCH_DISTANCE_M: f64 = 1_500.0;
/// 最小画面サイズ(`ModelsState::min_screen_px`)の既定値(px)。シンボル(約30px)と同じくらい。
pub const DEFAULT_MIN_SCREEN_PX: f64 = 32.0;

/// モデルの表示方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelDisplayMode {
    /// モデルを使わず、シンボルだけで描く。
    Off,
    /// カメラからの距離が`switch_distance_m`以内のトラックはモデル、それより遠いトラックはシンボルで描く。
    SwitchToSymbol,
    /// 常にモデルで描く。画面での大きさが`min_screen_px`に満たないモデルは、その大きさになるよう実寸より大きくする。
    MinScreenSize,
}

impl ModelDisplayMode {
    /// 設定画面などに出す名前(日本語)。
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "シンボルのみ",
            Self::SwitchToSymbol => "近くはモデル・遠くはシンボル",
            Self::MinScreenSize => "常にモデル(最小サイズを保証)",
        }
    }
}

/// 種別に使うモデルの取得元と補正。
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSource {
    /// GLBファイルのURL(ページからの相対でも絶対でもよい)。
    pub url: String,
    /// モデルの単位をメートルにする倍率(cmで作られたモデルなら0.01)。既定は1。
    pub scale: f32,
    /// モデルの前が、glTFの前(+Z)から上から見て時計回りにこの角度(度)だけずれて作られているとき、その角度(打ち消して前に合わせる)。既定は0。
    pub yaw_offset_deg: f32,
}

impl ModelSource {
    /// URLだけ指定する(単位はメートル、前は+Z)。
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into(), scale: 1.0, yaw_offset_deg: 0.0 }
    }
}

/// 3Dモデル表示の設定。アプリは`provide_context`で1つだけ生成して渡す(`ModelsState::new()`)。
/// `TerrainView`が購読して描く(未提供ならモデルなしで動作する)。
#[derive(Clone, Copy)]
pub struct ModelsState {
    /// 表示方式。
    pub mode: RwSignal<ModelDisplayMode>,
    /// `SwitchToSymbol`でモデルからシンボルへ切り替える、カメラからの距離(メートル)。2Dは、同じ縦幅が映る透視投影の距離に換算する。
    pub switch_distance_m: RwSignal<f64>,
    /// `MinScreenSize`でモデルを最低この大きさ(画面のpx。モデルの外接球の直径)で見せる。
    pub min_screen_px: RwSignal<f64>,
    /// 種別ごとに使うモデル(`set_source`で登録する)。
    pub sources: RwSignal<HashMap<SymbolKind, ModelSource>>,
}

impl ModelsState {
    pub fn new() -> Self {
        Self {
            mode: RwSignal::new(ModelDisplayMode::SwitchToSymbol),
            switch_distance_m: RwSignal::new(DEFAULT_SWITCH_DISTANCE_M),
            min_screen_px: RwSignal::new(DEFAULT_MIN_SCREEN_PX),
            sources: RwSignal::new(HashMap::new()),
        }
    }

    /// 種別`kind`のトラックを描くモデルを登録する(同じ種別を登録し直すと置き換わる)。
    pub fn set_source(&self, kind: SymbolKind, source: ModelSource) {
        self.sources.update(|sources| {
            sources.insert(kind, source);
        });
    }

    /// 種別`kind`のモデルの登録を外す(その種別はシンボルで描く)。
    pub fn clear_source(&self, kind: SymbolKind) {
        self.sources.update(|sources| {
            sources.remove(&kind);
        });
    }
}

impl Default for ModelsState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_registered_per_kind_and_can_be_replaced_or_cleared() {
        let owner = leptos::reactive::owner::Owner::new();
        owner.with(|| {
            let state = ModelsState::new();
            assert_eq!(state.mode.get_untracked(), ModelDisplayMode::SwitchToSymbol, "既定は距離で切り替え");
            state.set_source(SymbolKind::Aircraft, ModelSource::new("a.glb"));
            state.set_source(SymbolKind::Ship, ModelSource::new("s.glb"));
            state.set_source(SymbolKind::Aircraft, ModelSource { scale: 0.01, ..ModelSource::new("b.glb") });
            let sources = state.sources.get_untracked();
            assert_eq!(sources.len(), 2);
            assert_eq!((sources[&SymbolKind::Aircraft].url.as_str(), sources[&SymbolKind::Aircraft].scale), ("b.glb", 0.01));
            state.clear_source(SymbolKind::Ship);
            assert!(!state.sources.get_untracked().contains_key(&SymbolKind::Ship));
        });
    }

    #[test]
    fn mode_labels_are_japanese() {
        assert_eq!(ModelDisplayMode::Off.label(), "シンボルのみ");
        assert!(ModelDisplayMode::MinScreenSize.label().contains("最小サイズ"));
    }
}
