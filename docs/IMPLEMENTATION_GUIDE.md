# sim3dview 再実装ガイド(ドキュメントだけでライブラリを1から作る)

本書は、**ソースコードを見ずに、ドキュメントだけで`sim3dview`ライブラリと前処理ツールを再実装する**ための入口である。
AIエージェント・新規参加者のどちらも、まず本書を読み、フェーズごとに詳細仕様([impl/](impl/))を参照しながら進める。
本書は設計書体系([README.md](README.md))の一部で、基本設計・詳細設計の下に位置する「作り方」の入口である。

- 「何を・なぜ作るか」= [BASIC_DESIGN.md](BASIC_DESIGN.md)(確定した設計方針)・[DETAILED_DESIGN.md](DETAILED_DESIGN.md)(データ・座標系・設計方針・UML)
- 「どう作るか(定数・手順・バイト配置・シェーダー・テスト)」= **本書と[impl/](impl/)**(実装コードから起こした、実装者向けの正確な仕様)
- 両者が食い違う箇所は、**`impl/`が正**(コードから起こしているため。役割分担は[DETAILED_DESIGN.md](DETAILED_DESIGN.md) 0節)。経緯は[DEVELOPMENT_HISTORY.md](DEVELOPMENT_HISTORY.md)に残る。

---

## 1. 作るもの(成果物)

| 成果物 | 内容 | 言語 | 仕様 |
|---|---|---|---|
| `tools/geotiff_preprocess` | ALOS DSM GeoTIFF → 1度タイルごとの多段解像度グリッド+`metadata.json`(独立CMakeプロジェクト、GDAL依存) | C++20 | [第1部](impl/1_data_and_geodesy.md) 7章 |
| `sim3dview/`(crate) | 3D地形描画(wgpu)・覆域/見通し計算・観測点・作図・航跡・汎用UI部品(Leptos 0.8, WASM/CSR) | Rust | 第1〜5部 |
| `sim3dview/style/sim3dview.css` | 上記UIのスタイル | CSS | [第5部](impl/5_ui_and_integration.md) 6章 |
| (検証用)最小アプリ | `sim3dview`を組み込んで動作確認する小さなLeptosアプリ | Rust | [sim3dview/README.md](../sim3dview/README.md)「最小構成の使用例」 |

**含まないもの**: VAB・状況パネル・メニューバー・WebSocket/MessagePackプロトコル・C++シミュレーションサーバー本体(すべてアプリ固有。`sample/`が参照実装)。
ライブラリは通信プロトコルもサーバーのURLも知らない。ライブラリが要求するのは、HTTPで配信される**地形データの契約**だけ([第1部](impl/1_data_and_geodesy.md) 1章)。

## 2. ドキュメントの読み順

| 順 | 文書 | 目的 |
|---|---|---|
| 1 | [BASIC_DESIGN.md](BASIC_DESIGN.md) 4節(確定17項目) | 前提となる設計判断 |
| 2 | 本書 | 全体像・フェーズ・落とし穴 |
| 3 | [sim3dview/README.md](../sim3dview/README.md) | **公開API・使い方**(アプリから見た振る舞い。再実装後の互換性の基準) |
| 4 | [`impl/1〜5`](impl/) | 各フェーズの詳細仕様 |
| 参考 | [DETAILED_DESIGN.md](DETAILED_DESIGN.md) 1〜3・6節 | 背景・理由・図(UML)。4・5・7節(プロトコル・C++・サンプルUI)はライブラリの範囲外 |
| 参考 | [README.md](../README.md) | ツール・環境のセットアップ、トラブルシューティング |

| 詳細仕様 | 範囲 |
|---|---|
| [第1部 データ・測地・メッシュ・前処理](impl/1_data_and_geodesy.md) | ファイル契約(バイト配置)、`fetch/loader/geodesy/heightmap/mesh`、`geotiff_preprocess` |
| [第2部 カメラ・LOD・ピッキング・見通し](impl/2_lod_camera_los.md) | `camera/lod/pick/profile/los`(純粋関数) |
| [第3部 レンダラー・シェーダー](impl/3_renderer_and_shaders.md) | `renderer/*`、`vertex`、`terrain.wgsl`・`draw.wgsl`(**逐語**) |
| [第4部 重ね描き](impl/4_overlays.md) | `render_bias/markers/drawing/drawing_geometry/draw_tool/tracks` |
| [第5部 UI・統合](impl/5_ui_and_integration.md) | `ui/*`、context、`TerrainView`のイベント・Effect・LOD適用ループ、CSS契約、crate構成 |

## 3. 前提条件・ツール

- Rust stable(edition 2021)+`wasm32-unknown-unknown`ターゲット、`trunk`(WASMビルド・開発サーバー)、Python(補助スクリプト)。
- 主要依存(バージョン固定): `leptos 0.8 (csr)`、`wgpu 30`、`glam 0.33`、`bytemuck 1`、`earcutr 0.5`、`gloo-net 0.7`、`gloo-timers 0.4`、`serde`/`serde_json`、`web-sys`(feature一覧は[第5部](impl/5_ui_and_integration.md) 1.1)、dev: `naga 30 (wgsl-in)`。
  **wgpu・glamのAPIは版で名前が変わる**。上記の版を使うこと。
- 前処理ツール: CMake 3.20+、vcpkgの`gdal`。地形データ(約12GB)は`geotiff_preprocess`をリポジトリルートから実行して生成する(リポジトリには含まれない)。
- ブラウザ: WebGPU対応(Chrome/Edge)。UI確認は`http://localhost:8081`(`trunk serve`、`Trunk.toml`のポート)。
- 環境問題の対処(`trunk`の`NO_COLOR`、`cargo.exe`の「信頼されていないマウントポイント」等)は[README.md](../README.md)のトラブルシューティング表。

## 4. 全体で共通の約束(すべてのフェーズに効く)

### 4.1 座標系・単位・向き

- **ENU**: 東=X、北=Y、鉛直上=Z(右手系)。原点(`Origin{lat,lon}`)の**楕円体高0m**の点が(0,0,0)。頂点データは軸を入れ替えず、カメラの`up=Vec3::Z`で吸収する。
- 楕円体はWGS84(`a=6378137`, `inv_f=298.257222101`)。DSMの標高(EGM96海抜)をそのまま楕円体高として使う。地球の丸みは変換式に自然に含まれる(原点から1,000kmで約80km下がる)。
- **遠方の地表の高さ・クリック判定は標高ではなく`heightmap::ground_at_enu`(丸み込みのENU上座標)を使う**。`EnuTransform::inverse`は接平面近似で、原点近傍(見通し・断面)専用。
- 角度: 度(`_deg`)とラジアン(`_rad`)を名前で区別。方位・回転は**北(上)=0、時計回り**。緯度経度は`f64`、ENU座標のGPU頂点は`f32`だが**生成計算は`f64`**(原点から遠いほど`f32`の丸めでざらつく)。
- グリッド: 行=南→北、列=西→東。データなし=`i16::MIN`(-32768)。タイル・チャンクのキーは南西角の整数度/チャンク番号=`行*6+列`。
- 原点(`OriginState`。シミュレーション座標系の基準)とカメラの中心点(`OrbitCamera::target`)は**別物**。中心点を動かしても原点・メッシュ・観測点は変わらない。
  原点の変更はサーバー側でシミュレーション停止中のみ(ライブラリは`on_submit`を呼ぶだけ)。
- 描画は**反転Z**(近い=1, 遠い=0、有限far)。深度・射影を触るときは`screen_to_ray`(ピッキング)も整合させる。

### 4.2 コーディング規約(このリポジトリの慣習)

- コメント・ドキュメント・コミットメッセージ・チャット応答は**日本語**。`unsafe`は使わない(`unsafe impl Send`も禁止)。
- 純粋関数(計算)と状態(シグナルcontext)を分ける。GPU・ネットワーク・DOMに触れないコードはネイティブの`cargo test`で検証できるようにする。
- 失敗の扱い: 取得失敗は`Result<_, String>`+`log`。取得失敗したグリッドは再試行しない(`failed`集合)。localStorageは読み書きとも失敗しうるので必ずtry/無視で包む。
- 公開範囲は「アプリが使うものだけ`pub`、他は`pub(crate)`」([第5部](impl/5_ui_and_integration.md) 1.2)。

### 4.3 テストの方針

`TerrainData::synthetic`([第1部](impl/1_data_and_geodesy.md) 3.4)で合成地形を作り、数値で検証する。GPUは使わない。WGSLは`naga`で構文・型を検証し、
uniform・頂点の**バイトレイアウトがRustの構造体と一致する**ことを確かめる([第3部](impl/3_renderer_and_shaders.md) 6章)。
参照実装の単体テストは約100件(下表)。**同名・同趣旨のテストを書けば、仕様の主要な数値がすべて確認できる**。

## 5. 実装フェーズ(依存順)と受け入れ基準

各フェーズは「作る → 指定のテストが通る → 次へ」。`cargo check -p sim3dview --target wasm32-unknown-unknown`と`cargo test -p sim3dview`を各フェーズの終わりに通す。

| # | フェーズ | 作るもの | 仕様 | 受け入れ基準(テスト名は参照実装のもの) |
|---|---|---|---|---|
| 0 | 骨組み | ワークスペース(`members=["sim3dview", ...]`)、`sim3dview`のCargo.toml・`lib.rs`・空モジュール、合成地形`TerrainData::synthetic` | 第5部1章・第1部3.4 | `cargo check`(wasm32)と`cargo test`(ネイティブ)が空で通る |
| A | 前処理ツール(並行可) | `geotiff_preprocess`(C++)。実データの生成 | 第1部7章 | `base.bin`サイズ=`タイル数×61²×2`、`L4`ファイル=`36×601²×2`、縁ノードの一致、南北の向き(富士山) |
| 1 | 測地 | `Ellipsoid`・`Origin`・`EnuTransform` | 第1部4章 | `enu_round_trip_far_from_origin` `far_ground_curves_down` `axes_point_east_north_up` `shader_params_describe_the_ellipsoid_surface` `inverse_agrees_with_the_exact_inverse_near_the_origin` |
| 2 | データ保持・取得 | `loader::TerrainData`(+型・定数)、`fetch` | 第1部1〜3章 | loader: `tile_lookup_uses_bounds` `bilinear_reproduces_a_linear_slope_on_the_base_grid` `bilinear_treats_no_data_neighbours_as_sea_level` `detail_grid_is_used_only_after_the_chunk_level_is_set` `insert_chunk_grid_ignores_bad_arguments` `insert_tile_level_splits_records_per_chunk` `evict_unused_drops_oldest_first_and_respects_keep`。fetch: `tile_name_uses_hemisphere_letters` `decode_reads_little_endian_i16` |
| 3 | 標高・メッシュ | `heightmap`、`mesh`(`TerrainVertex`・色・スカート・法線・インデックス) | 第1部5・6章 | heightmap: `heightmap_is_sampled_inside_bounds_only` `ground_curves_down_like_d2_over_2r` `ground_follows_terrain_height_far_away` `same_ground_point_regardless_of_origin`。mesh: `color_ramp_clamps_and_handles_empty_range` `skirt_gets_shallower_at_finer_levels` `whole_tile_mesh_has_the_documented_vertex_count` `triangles_touching_no_data_are_dropped` `normals_are_lit_only_where_defined_and_lean_away_from_the_slope` |
| 4 | カメラ・ピッキング・断面 | `camera`、`pick`、`profile` | 第2部1・3・4章 | camera 8件(`depth_is_reversed_for_perspective/orthographic` `screen_to_ray_is_consistent_with_the_projection` `water_ray_basis_matches_screen_to_ray` `orbit_zoom_and_pan_clamp_and_move_as_documented` `pan_orbit_target_moves_along_the_screen_axes` `eye_is_kept_above_the_ground` `two_d_camera_looks_straight_down_with_north_up`)、pick 4件、profile 3件 |
| 5 | LOD計画 | `lod` | 第2部2章 | `cell_sizes_and_chunk_costs_match_the_documented_numbers` `ideal_level_picks_the_coarsest_level_fine_enough_for_the_pixel` `far_view_uses_the_floor_level_for_every_tile` `close_view_refines_near_chunks_only_and_lists_the_nearest_tile_first` `budget_decides_which_tiles_get_chunks` `upgrades_never_exceed_the_budget` `hysteresis_keeps_one_level_finer_than_ideal_but_not_more` `tiles_outside_the_view_keep_their_current_level` |
| 6 | 見通し計算 | `los`(`compute_los`・`is_visible`・`min_visible_altitude`・`compute_coverage_area`・`compute_los_dome`) | 第2部5章 | 8件(`azimuth_index_is_in_mils` `curvature_drop_grows_with_the_square_of_distance` `point_visibility_on_flat_ground_follows_the_radio_horizon` `a_ridge_hides_what_is_behind_it` `min_visible_altitude_is_consistent_with_visibility` `los_range_on_flat_ground_is_the_radio_horizon_in_every_direction` `coverage_area_reaches_the_max_range_for_a_high_target_and_stops_at_the_ridge` `dome_rings_are_full_spheres_over_flat_ground`)。加えて`compute_los_dome`の最適化版と素朴実装の一致テスト |
| 7 | レンダラー・シェーダー | `vertex`、`renderer/*`、`terrain.wgsl`、`draw.wgsl` | 第3部 | naga検証+レイアウト一致テスト10件(`wgsl_modules_parse_and_validate` `camera_uniform_matches_the_wgsl_struct` `draw_uniform_matches_the_wgsl_struct` `terrain_vertex_layout_matches_struct_and_wgsl` `draw_vertex_layout_matches_struct_and_wgsl` `supersample_size_doubles_and_clamps_each_side` `position_bounds_encloses_all_vertices` `screen_matrix_maps_pixels_to_clip_space_with_y_down` `frustum_test_culls_boxes_that_are_clearly_outside` `frustum_test_never_culls_a_visible_box`)、vertex 2件。**`python scripts/sync_impl_wgsl.py --check`が通る** |
| 8 | 地図コンポーネント | `terrain::{origin, origin_pick, recenter, hillshade, store}`、`ui::terrain_view`(初期化・カメラ操作・LOD適用ループ) | 第5部2〜4章 | 実機: 最小アプリで地形が出る→レベル0→1→…と近い順に細かくなる・3D回転/ズーム/Shift+ドラッグ・2D切替・原点変更で地形とカメラが追従・地面の下にもぐらない |
| 9 | 観測点と見通しUI | `markers`(状態・ピン・ドーム・2D覆域)、`ui::{los_view, cross_section_view, tabbed_panel, floating_panel, origin_dialog, coverage_altitude_dialog}` | 第4部1・2章、第5部5章 | markers 3件(`smoothing_keeps_constant_and_removes_single_spikes` `smoothing_wraps_around_and_softens_steps` `pin_is_built_from_billboard_triangles`)+実機: 右クリックで観測点追加→3Dドーム/2D覆域→極座標図・断面図が更新 |
| 10 | 作図 | `drawing`、`drawing_geometry`(+レンダラーの`update_drawings`・オーバーレイパス) | 第4部3・4章 | drawing_geometry 17件(第4部4.7)。実機: 全種類の図形を`World/View/Screen`で表示、カメラを回しても`View/Screen`が動かない |
| 11 | 図形の対話作成・右クリックメニュー | `draw_tool`、`ui::{context_menu, drawing_editor, util}` | 第4部5章、第5部5.3・5.8 | draw_tool 8件(`circle_radius_is_click_distance` `zero_size_shapes_are_rejected` `rect_is_centered_between_corners` `sector_sweeps_clockwise_from_start_to_end` `sphere_rests_on_ground` `polygon_and_polyline_need_enough_points` `preview_falls_back_to_polyline` `shapes_survive_json_round_trip`)+実機: 全ツールの作成・確定・取り消し・数値編集・リロード後の復元 |
| 12 | 航跡 | `tracks`(モデル・ジオメトリ・当たり判定)、`terrain_view/labels.rs`、選択 | 第4部6章、第5部3.10 | tracks 10件(第4部6.5)+実機: シンボルが進行方向を向く・ラベル追従・クリックで選択・右クリックメニュー |
| 13 | 仕上げ | `sim3dview.css`、`sim3dview/README.md`、`THIRD_PARTY_NOTICE.md`再生成 | 第5部6章 | 全テスト通過、README掲載のコード例がコンパイルできる |

### 依存関係(並行作業のヒント)

```
0 ─┬─ 1 ── 2 ── 3 ──┬─ 4 ── 5 ──┐
   │                 └─ 6        ├─ 8 ── 9 ── 10 ── 11
   ├─ 7(シェーダー・レンダラー。1〜3の型に依存、他とは独立に進められる)──┘        └─ 12
   └─ A(前処理ツール。Rustとは完全に独立。実データ生成に必要)
```
4・5・6・7は互いに独立(いずれも1〜3の後)。AはRust側と完全に並行できる。

## 6. 完了の定義(Definition of Done)

1. `cargo check -p sim3dview --target wasm32-unknown-unknown` と、最小アプリの`cargo check ... --target wasm32-unknown-unknown`が通る。
2. `cargo test -p sim3dview`(ネイティブ)が全件通る。
3. `python scripts/sync_impl_wgsl.py --check`が通る(ドキュメントのWGSLと実ファイルが一致)。
4. 実データで実機確認(上表の実機項目)。**判断を1枚のスクリーンショットだけで下さない**(同じ操作を複数回再現する)。
5. [sim3dview/README.md](../sim3dview/README.md)の公開APIと矛盾しない(この文書の仕様どおりに作れば一致する)。

## 7. 落とし穴チェックリスト(過去に踏んだもの。詳細は各仕様書と[DEVELOPMENT_HISTORY.md](DEVELOPMENT_HISTORY.md))

**データ・数値**
- [ ] GDALは北→南。前処理で行順を**南→北へ反転**しないと地形が南北反転する。
- [ ] `metadata.json`の数値は`setprecision(15)`で出す(既定6桁だと`138.859722`→`138.86`)。
- [ ] `base.bin`のサイズ検証を必ずする(`タイル数×(N₀+1)²×2`)。
- [ ] 標高の色の下限は0m固定(DSMには水面ノイズ由来の大きな負値がある)。上限だけ`elevation_max`。
- [ ] 海の判定はDSMのNODATAでは足りない。`*_MSK.tif`の画素値3を海とする。海は補間で埋めない。
- [ ] 遠方の地表は`ground_at_enu`(丸み込み)。`inverse`(接平面近似)を遠方に使わない。
- [ ] 頂点生成は`f64`で行い最後に`f32`へ(法線計算用の位置も`f64`のまま)。

**カメラ・描画**
- [ ] 深度は**反転Z+有限far**(無限遠射影は「原点から遠いほど地表が消える」)。クリア0.0、比較`Greater`。
- [ ] レイは逆VP行列で求めない(f32の丸めで遠方で向きがずれる)。`basis()`から直接。
- [ ] 水域は「地形メッシュが無い所」+**楕円体との交点の深度を1km奥へずらして書く**(丸みの向こうの地形が透けず、標高0m以下の地形は隠れない)。
- [ ] スカートは海抜0mまで届かせる(縁の下の隙間から地形の裏側が見えるため)。
- [ ] MSAAは4のみ(8は非対応環境がある)。+2倍スーパーサンプリング(縮小は自前の線形フィルタ。`TextureBlitter`は使わない)。
- [ ] `Timeout|Occluded`はエラーではなく「このフレームは描かない」。`Outdated`は再設定。
- [ ] 2D覆域は**深度テストなし**で描く(あると起伏に埋まって塗りが欠ける)。3Dドームの傘は約48枚に間引く(半透明の描画順依存のちらつき)。
- [ ] ビルボードのアンカー深度は距離の0.2%手前へ(粗いLODの地形に埋まらないように)。線は`LINE_DEPTH_BIAS`で面より手前に。

**UI・Leptos**
- [ ] `Effect`内の`RefCell`借用は、シグナル更新の前に`drop`する(同期再実行で`BorrowMutError`)。
- [ ] `Rc`を含むものは`RwSignal::new_local`/`StoredValue::new_local`。公開コールバックは`UnsyncCallback`。`unsafe impl Send`禁止。
- [ ] `ResizeObserver`は非表示タブでスロットリングされる。`visibilitychange`で取り直す(canvasが300×150のまま引き伸ばされる)。UI確認はBrowserペインを表示した状態で。
- [ ] `on_cleanup`で`ResizeObserver.disconnect`・`remove_event_listener`。`Closure::forget`しない(GPUデバイスが解放されない)。
- [ ] `TabbedPanel`は全タブを1度だけ生成して`display`で切り替える(作り直さない)。
- [ ] 作図エディタの一覧行は「追加・削除・改名」でだけ作り直す(編集のたびに作り直すと入力フォーカスが外れる)。
- [ ] カーソル移動での仮図形の更新は1フレームに1回にまとめる(更新のたびに作図全体の再構築+描画が走る)。
- [ ] `view!`の属性値に演算子を含む式を直接書かない(`let`で受けてから渡す)。
- [ ] ライブラリ側だけを編集したら`trunk serve`を再起動する(path依存先は監視されない)。

**LOD・性能**
- [ ] 全タイルを常駐させるので、視錐台カリング(メッシュのAABB)を必ず入れる。
- [ ] LOD更新は時間(12ms)+頂点数(60万)で区切って小分けに。取得完了後は8msで続きへ(デバウンス150msを待たない)。同時取得16。
- [ ] 目標より1つだけ細かいレベルは下げない(ヒステリシス)。視野外は現状維持。
- [ ] 取得済みグリッドは300MiBまで。超えたら画面に出していないものを古い順に捨てる。標高サンプリングは各チャンクの「いま画面に出しているレベル」で引く。
- [ ] 観測点1つの覆域計算=6400方位×1000サンプル。`compute_los_dome`は走査中の最大仰角の単調性を使って全リング×全サンプルを避ける。

## 8. ドキュメントの保守

- シェーダー(`terrain.wgsl`・`draw.wgsl`)を編集したら`python scripts/sync_impl_wgsl.py`を実行してドキュメントへ反映する(`--check`はCI/コミット前の確認用)。
- 定数・アルゴリズムを変えたら、対応する`impl/`の記述と、本書のテスト名表を更新する。設計上の理由(なぜ)が変わるときは[DETAILED_DESIGN.md](DETAILED_DESIGN.md)の該当節(6節)も直す(同じ数値を両方に書き写さない)。依存を変えたら`python scripts/gen_third_party_notice.py`。
- 再実装ドキュメントの品質確認方法: 新しいエージェントに`docs/`配下の設計書(BASIC_DESIGN・DETAILED_DESIGN・`impl/`・本書)だけを渡し(ソースは見せない)、フェーズ順に実装させ、受け入れ基準のテストを通させる。詰まった箇所がドキュメントの不足なので、その場で追記する。
