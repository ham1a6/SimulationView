//! 地図と組み合わせて使うUIコンポーネント(Leptos)。`terrain`の状態(context)を読み書きし、
//! 地図(`terrain_view`)・各種ダイアログ・パネルの部品を提供する。

/// 汎用の右クリックメニューと、地図の右クリックへのつなぎ込み。
pub mod context_menu;
/// 2Dの覆域高度の設定ダイアログ。
pub mod coverage_altitude_dialog;
/// 選択中の観測点からの断面図(標高プロファイル)。
pub mod cross_section_view;
/// 作図の一覧と、図形の属性の編集。
pub mod drawing_editor;
/// ドラッグで動かせる、画面に浮かぶパネル。
pub mod floating_panel;
/// 選択中の観測点の見通し範囲の平面図。
pub mod los_view;
/// 3Dモデルの表示設定ダイアログ。
pub mod model_settings_dialog;
/// シミュレーション原点の設定ダイアログ。
pub mod origin_dialog;
/// ポインタのドラッグの追跡(クリックとの区別)。
pub mod pointer_drag;
/// 仕切りをドラッグして大きさを変えられる2分割レイアウト。
pub mod split_pane;
/// タブで中身を切り替えるパネル。
pub mod tabbed_panel;
/// 地形と重ねるものを描く地図のcanvas(`TerrainView`)。
pub mod terrain_view;
/// UIの小さな共通処理。
pub mod util;
