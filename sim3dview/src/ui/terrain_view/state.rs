//! `TerrainView`の状態(`ViewState`)と、その部品。

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use leptos::prelude::*;

use super::coverage::CoverageState;
use super::models::ModelsView;
use crate::terrain::camera::OrbitCamera;
use crate::terrain::drawing::DrawingState;
use crate::terrain::hillshade::HillshadeState;
use crate::terrain::loader::{TerrainData, TileKey};
use crate::terrain::lod::TileLayout;
use crate::terrain::markers::RadarMarkersState;
use crate::terrain::origin::Origin;
use crate::terrain::renderer::TerrainRenderer;
use crate::terrain::tracks::{TrackId, TrackLabel, TracksState};
use crate::ui::pointer_drag::DragTracker;

#[derive(Default)]
pub(super) struct InteractionState {
    pub(super) drag: DragTracker,
}

pub(super) struct ViewState {
    pub(super) renderer: Option<TerrainRenderer>,
    pub(super) terrain: Option<Rc<TerrainData>>,
    /// 現在GPUにアップロードされているメッシュが基づいている原点。
    pub(super) mesh_origin: Option<Origin>,
    pub(super) camera: OrbitCamera,
    /// 現在の注視点の地表標高(ENU上座標、メートル)。原点の緯度経度における
    /// heightmapの値。カメラの`target`はここではなく`Vec3::new(0,0,target_up)`に
    /// 置くことで、ズームインしても地表に埋まらないようにする(camera.rs参照)。
    pub(super) target_up: f32,
    pub(super) initializing: bool,
    pub(super) interaction: InteractionState,
    pub(super) radar_markers: RadarMarkersState,
    /// 作図(図形・線)の一覧(`terrain::drawing`)。
    pub(super) drawings: DrawingState,
    /// 航跡(トラック)の一覧・表示設定(`terrain::tracks`)。
    pub(super) tracks: TracksState,
    /// 3Dモデル(`terrain::models`)の設定・取得状況・配置。
    pub(super) models: ModelsView,
    /// 航跡のラベルを置くHTML要素(canvasに重ねる層)と、いま置いているラベル。
    pub(super) labels_ref: NodeRef<leptos::html::Div>,
    pub(super) labels: Vec<LabelView>,
    /// クリックでの選択(当たり判定)に使う、各トラックのシンボルの位置(ENU座標)。ラベルの表示設定に関係なく持つ。
    pub(super) pick_anchors: Vec<(TrackId, [f32; 3])>,
    /// 陰影(ヒルシェード)のON/OFF。レンダラー作成時の初期値に使う(以後の変更はEffect 7が反映する)。
    pub(super) hillshade: HillshadeState,
    /// 各タイルの、いまGPUに載っている状態(全体1枚か、チャンクごとのレベルか。`terrain::lod`参照)。
    pub(super) resident: HashMap<TileKey, TileLayout>,
    /// 取得中のグリッド。
    pub(super) loading: HashSet<FetchKey>,
    /// 取得に失敗したグリッド。同じ取得を延々と繰り返さないよう覚えておく。
    pub(super) failed: HashSet<FetchKey>,
    /// LOD更新のタイマー待ち中か(連続する操作をまとめるため)。
    pub(super) lod_pending: bool,
    /// 取得の完了・メッシュ反映の続きによる、短い待ちのLOD更新の予約中か(`schedule_lod_soon`)。
    pub(super) lod_soon_pending: bool,
    /// 地形メッシュのクロスフェード(`TerrainRenderer::is_fading`)の描き直しを予約中か。
    pub(super) fade_frame_pending: bool,
    /// 覆域(3Dドーム・2D領域)の計算結果のキャッシュと、進行中の計算(`coverage`)。
    pub(super) coverage: CoverageState,
}

/// 画面に重ねている航跡ラベル1つ分(HTML要素と、その元のデータ)。
pub(super) struct LabelView {
    pub(super) anchor: TrackLabel,
    pub(super) root: web_sys::HtmlElement,
    pub(super) name: web_sys::HtmlElement,
    pub(super) detail: web_sys::HtmlElement,
}

/// 取得するグリッドの識別子: (タイル, レベル, チャンク)。チャンクがNoneなら、タイル1枚分・
/// 1レベルのファイル全体(小さいレベル)を指す(`loader::WHOLE_FILE_MAX_LEVEL`)。
pub(super) type FetchKey = (TileKey, usize, Option<usize>);
