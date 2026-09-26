//! レンダラーの初期化と、1フレームの描画。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use leptos::prelude::*;

use super::{
    labels::*, loading::InitStatus, lod_driver::*, models::update_models, overlay::*, state::*,
};
use crate::terrain::geodesy::EnuTransform;
use crate::terrain::heightmap;
use crate::terrain::loader::{TerrainData, WHOLE_TILE};
use crate::terrain::lod::TileLayout;
use crate::terrain::mesh;
use crate::terrain::origin::{Origin, OriginState};
use crate::terrain::renderer::TerrainRenderer;

/// 現在の状態でレンダラーを構築できるなら構築する。
/// canvasのサイズ確定(ResizeObserver)と地形データ取得(TerrainStore)は非同期かつ独立して
/// 完了するため、両方のイベントからこの関数を呼び、揃った時点で実際に初期化されるようにする。
pub(super) fn try_init(
    state: Rc<RefCell<ViewState>>,
    canvas: web_sys::HtmlCanvasElement,
    data: Option<Rc<TerrainData>>,
    origin_state: OriginState,
    status: RwSignal<InitStatus>,
) {
    // canvasの大きさ・地形データのどちらかがまだなら、揃ったときにもう一方から呼ばれるので待つ。
    let width = canvas.width();
    let height = canvas.height();
    if width == 0 || height == 0 {
        return;
    }
    let Some(data) = data else {
        return;
    };
    // 初期化済み・初期化中なら何もしない(両方のイベントがほぼ同時に来ても1回だけ初期化する)。
    {
        let s = state.borrow();
        if s.renderer.is_some() || s.initializing {
            return;
        }
    }
    state.borrow_mut().initializing = true;

    let origin = origin_state
        .0
        .get_untracked()
        .unwrap_or(data.metadata.default_origin);
    // 注視点は原点の実際の地表標高に置く(Vec3::ZEROのままだと、原点が高山の
    // 斜面にある場合にズームインした際カメラが地面に埋まって真っ黒になる)。
    let target_up = heightmap::sample_heightmap(&data, origin.lat_deg, origin.lon_deg);

    wasm_bindgen_futures::spawn_local(async move {
        match TerrainRenderer::new(canvas).await {
            Ok(mut renderer) => {
                // 全タイルを最粗のレベル0(タイル全体で1枚)で載せる(細かいレベルはカメラに近い
                // チャンクだけ、あとから`update_lod`が差し替える)。
                let transform = EnuTransform::new(&origin, &data.metadata.ellipsoid);
                let mut resident = HashMap::new();
                for tile in data.tiles() {
                    let tile_mesh = mesh::build_whole_tile_mesh(&data, tile, &transform);
                    renderer.set_mesh((tile.key.0, tile.key.1, WHOLE_TILE), &tile_mesh);
                    resident.insert(tile.key, TileLayout::Whole);
                }
                {
                    let mut s = state.borrow_mut();
                    s.target_up = target_up;
                    s.camera.target.z = target_up;
                    s.lod.resident = resident;
                }
                renderer.set_hillshade(state.borrow().hillshade.enabled.get_untracked());
                renderer.set_ellipsoid_origin(&transform);
                let camera = state.borrow().camera.to_camera(renderer.aspect_ratio());
                if let Err(e) = renderer.render(&camera) {
                    log::error!("[terrain] initial render failed: {e}");
                }
                let mut s = state.borrow_mut();
                s.renderer = Some(renderer);
                s.terrain = Some(data);
                s.mesh_origin = Some(origin);
                s.initializing = false;
                // signalの更新で購読しているEffectが同期的に走っても`state`を借用し直せるよう、
                // 借用を手放してから更新する。
                drop(s);
                status.set(InitStatus::Ready);
                rebuild_markers(&state);
                rebuild_drawings(&state);
                rebuild_tracks(&state);
                render_now(&state);
            }
            Err(e) => {
                log::error!("[terrain] {e}");
                status.set(InitStatus::Failed(e));
                state.borrow_mut().initializing = false;
            }
        }
    });
}

/// 3Dモードのカメラ(視点)が地面の下にもぐらないようにする(`OrbitCamera::keep_above_ground`)。
/// 視点の真下の地面の高さは、いま画面に出している地形(`heightmap::ground_at_enu`)から引く。
/// カメラの操作(回転・ズーム・移動)・原点変更・LODの切り替えのあとの描画の前に必ず通る。
fn keep_camera_above_ground(state: &Rc<RefCell<ViewState>>) {
    let mut s = state.borrow_mut();
    let Some((terrain, transform)) = s.mesh_frame() else {
        return;
    };
    s.camera.keep_above_ground(|east, north| {
        heightmap::ground_at_enu(&terrain, &transform, east as f64, north as f64).2
    });
}

/// 原点を`new_origin`へ変える(DETAILED_DESIGN.md 3.3節): 常駐している全メッシュ・水域・マーカー・作図・航跡を
/// 新しい原点のENU座標で作り直して描く。注視点は、原点の真上を見ていたなら新しい原点に追従し、
/// パンして別の場所を見ていたなら同じ緯度経度を見続ける。未初期化・同じ原点なら何もしない。
pub(super) fn change_origin(state: &Rc<RefCell<ViewState>>, new_origin: Origin) {
    let mut s = state.borrow_mut();
    let Some(terrain) = s.terrain.clone() else {
        return;
    };
    if s.renderer.is_none() || s.mesh_origin == Some(new_origin) {
        return;
    }
    let new_transform = EnuTransform::new(&new_origin, &terrain.metadata.ellipsoid);
    // パンして別の場所を見ていたなら、ENU座標のオフセットが新しい原点基準のまま残って
    // 表示が飛んでしまわないよう、同じ緯度経度を見続けるよう新しいENU座標へ変換し直す。
    let follows_origin = s.camera.target.x == 0.0 && s.camera.target.y == 0.0;
    s.target_up = heightmap::sample_heightmap(&terrain, new_origin.lat_deg, new_origin.lon_deg);
    if follows_origin {
        // 高さも新しい原点の地表標高へ更新する(古い標高のままだと、原点移動後に
        // ズームインした際カメラが地面に埋まって真っ黒になりうる)。
        s.camera.target.z = s.target_up;
    } else if let Some((_, old_transform)) = s.mesh_frame() {
        let (lat, lon, _) = heightmap::ground_at_enu(
            &terrain,
            &old_transform,
            s.camera.target.x as f64,
            s.camera.target.y as f64,
        );
        let (x, y, up) = heightmap::ground_at_geodetic(&terrain, &new_transform, lat, lon);
        s.camera.target.x = x;
        s.camera.target.y = y;
        s.camera.target.z = up;
    }
    // 常駐している全メッシュ(タイル全体・各チャンク、各自の解像度レベル)の頂点位置を、新しい原点のENU座標で
    // 作り直してアップロードする(頂点数・並びは原点に依存しない)。
    // `renderer`(可変)と`resident`(読み取り)を同時に借りるので、`RefMut`を素の`&mut`にして
    // フィールドごとの借用に分ける。
    let st = &mut *s;
    let Some(renderer) = st.renderer.as_mut() else {
        return;
    };
    // 水域レイヤー(楕円体の海抜0mの面)も新しい原点基準にする。
    renderer.set_ellipsoid_origin(&new_transform);
    for (&key, resident) in st.lod.resident.iter() {
        let Some(tile) = terrain.tile(key) else {
            continue;
        };
        match resident {
            TileLayout::Whole => {
                let vertices = mesh::build_whole_tile_vertices(&terrain, tile, &new_transform);
                renderer.update_mesh_vertices((key.0, key.1, WHOLE_TILE), &vertices);
            }
            TileLayout::Chunks(levels) => {
                for (c, &level) in levels.iter().enumerate() {
                    if let Some(vertices) = mesh::build_chunk_vertices(
                        &terrain,
                        tile,
                        c,
                        level as usize,
                        &new_transform,
                    ) {
                        renderer.update_mesh_vertices((key.0, key.1, c as u8), &vertices);
                    }
                }
            }
        }
    }
    let camera = st.camera.to_camera(renderer.aspect_ratio());
    if let Err(e) = renderer.render(&camera) {
        log::error!("[terrain] re-render after origin change failed: {e}");
    }
    st.mesh_origin = Some(new_origin);
    drop(s);
    // マーカー・覆域リング・作図も新しい原点基準のENU座標へ再変換する。
    rebuild_markers(state);
    rebuild_drawings(state);
    rebuild_tracks(state);
    render_now(state);
}

/// 注視点(中心点)を、緯度経度`target`の地表(`None`なら原点の地表)へ動かして描く。
/// シミュレーション原点(`OriginState`)には触れない。レンダラー・地形が未作成の間は動かさない。
pub(super) fn recenter(state: &Rc<RefCell<ViewState>>, target: Option<(f64, f64)>) {
    let mut s = state.borrow_mut();
    if s.renderer.is_none() {
        return;
    }
    match target {
        // 指定した地点へ。高さはその地点の実際の地表(ENU上座標)にする
        // (Shift+ドラッグでの移動と同じ。古い高さのままだとズームインしたときカメラが地面に埋まる)。
        Some((lat, lon)) => {
            let Some((terrain, transform)) = s.mesh_frame() else {
                return;
            };
            let (east, north, up) = heightmap::ground_at_geodetic(&terrain, &transform, lat, lon);
            s.camera.target.x = east;
            s.camera.target.y = north;
            s.camera.target.z = up;
        }
        None => {
            s.camera.target.x = 0.0;
            s.camera.target.y = 0.0;
            s.camera.target.z = s.target_up;
        }
    }
    drop(s);
    render_now(state);
}

/// 次の画面更新で最新の状態を描く。同じフレームへの要求は1つに集約する。
pub(super) fn render_frame(state: &Rc<RefCell<ViewState>>) {
    {
        let mut s = state.borrow_mut();
        if !s.frame_request.request() {
            return;
        }
    }
    // コールバックは弱参照で状態を持つ(コンポーネントが破棄された後に走っても何もしない)。
    let weak_state = Rc::downgrade(state);
    if let Err(error) = request_animation_frame_with_handle(move || {
        if let Some(state) = weak_state.upgrade() {
            state.borrow_mut().frame_request.clear();
            draw_frame(&state);
        }
    }) {
        state.borrow_mut().frame_request.clear();
        log::warn!("[terrain] 描画の予約に失敗しました: {error:?}");
    }
}

/// 予約されたフレームを描画する。状態更新・フェードのどちらもこの経路を通る。
pub(super) fn draw_frame(state: &Rc<RefCell<ViewState>>) {
    keep_camera_above_ground(state);
    // どのトラックを3Dモデルで描くか(カメラからの距離・大きさで決まる)。描画の前に決める。
    update_models(state);
    let mut guard = state.borrow_mut();
    let s = &mut *guard;
    let mut fading = false;
    if let Some(renderer) = s.renderer.as_mut() {
        let camera = s.camera.to_camera(renderer.aspect_ratio());
        if let Err(e) = renderer.render(&camera) {
            log::error!("[terrain] render failed: {e}");
        }
        fading = renderer.is_fading();
    }
    update_labels(s);
    // シグナル更新や次フレームの予約より先にRefCellの借用を解放する。
    drop(guard);
    // クロスフェードの途中なら、時間を進めるために次のフレームも描く。
    if fading {
        render_frame(state);
    }
}

/// 描画とLOD更新を予約する。地面との衝突補正は次の入力より先に反映する。
pub(super) fn render_now(state: &Rc<RefCell<ViewState>>) {
    keep_camera_above_ground(state);
    render_frame(state);
    schedule_lod(state);
}
