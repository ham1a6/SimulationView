//! 画面全体で共有するUI状態(メニュー操作で開閉するフローティングパネルなど)。

use leptos::prelude::*;

/// 原点設定フローティングパネルの開閉状態。
/// `components/menu_bar.rs`(トリガー)と`components/origin_dialog.rs`(表示)で共有する。
#[derive(Clone, Copy)]
pub struct OriginDialogState(pub RwSignal<bool>);
