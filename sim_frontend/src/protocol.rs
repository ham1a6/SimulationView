//! Sim3dView 通信プロトコル定義。
//! フレーミング: `[1 byte: msg_type][MessagePack body]`
//! 詳細: DETAILED_DESIGN.md 4節。C++側 (`sim_server/include/protocol.hpp`) と内容を一致させること。
//!
//! 注意: rmp-serdeはデフォルトでstructをmsgpackの「配列」としてエンコード/デコードする
//! (C++側の`MSGPACK_DEFINE`マクロと同じ、フィールド名ではなく宣言順の位置で対応する形式)。
//! そのため各structのフィールド宣言順は、C++側の`MSGPACK_DEFINE(...)`の引数順と
//! **完全に一致させる必要がある**(片方だけ並び替えるとデータが壊れる)。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    SimState = 0x01,
    VabConfig = 0x02,
    OriginState = 0x03,
    StatusPanelConfig = 0x04,
    CommandError = 0x05,
    AppStatus = 0x06,
}

impl MsgType {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(MsgType::SimState),
            0x02 => Some(MsgType::VabConfig),
            0x03 => Some(MsgType::OriginState),
            0x04 => Some(MsgType::StatusPanelConfig),
            0x05 => Some(MsgType::CommandError),
            0x06 => Some(MsgType::AppStatus),
            _ => None,
        }
    }
}

// --- Server -> Client ------------------------------------------------

/// シミュレーション状態(高頻度、約60Hz)。DETAILED_DESIGN.md 4.3節。
#[derive(Debug, Clone, Deserialize)]
pub struct SimState {
    pub t: f64,
    #[allow(dead_code)] // v1はまだ実体を描画しないが、msgpackは配列位置エンコードのため保持が必要。
    pub positions: Vec<f32>,
    pub frame_id: u32,
    /// StatusPanelConfig.items と同じ順序・同じ数(v1では数値項目のみ)。
    pub status_values: Vec<f64>,
}

/// VAB(操作ボタン)1個分。ラベルが空文字のボタンは「未使用の穴」(DETAILED_DESIGN.md 7.4節)。
#[derive(Debug, Clone, Deserialize)]
pub struct VabButton {
    pub id: String,
    pub label: String,
    pub enabled: bool,
}

/// VABボタン配置設定(状態変化時のみ送信)。
#[derive(Debug, Clone, Deserialize)]
pub struct VabConfig {
    #[allow(dead_code)] // UIは先頭行(カテゴリ選択、cols個ぶん)しか使わないが、msgpackが
    // 配列位置エンコードのためフィールド自体は削除できない(vab.rs参照)。
    pub rows: u32,
    pub cols: u32,
    pub buttons: Vec<VabButton>,
}

/// 基準位置(原点)。DETAILED_DESIGN.md 3節・4.3節。サーバーが保持する状態が正。
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct OriginState {
    pub lat_deg: f64,
    pub lon_deg: f64,
}

/// 状況パネル項目1個分。DETAILED_DESIGN.md 4.3節・7.5節。
#[derive(Debug, Clone, Deserialize)]
pub struct StatusItem {
    #[allow(dead_code)] // UIはlabel/unitのみ表示に使うが、msgpackは配列位置エンコードのため保持が必要。
    pub id: String,
    pub label: String,
    /// 単位。なければ空文字列
    pub unit: String,
}

/// 状況パネル項目定義(状態変化時のみ送信)。実際の値は SimState.status_values で配信する。
#[derive(Debug, Clone, Deserialize)]
pub struct StatusPanelConfig {
    pub items: Vec<StatusItem>,
}

/// コマンド拒否応答。要求元クライアントのみに送信される。DETAILED_DESIGN.md 4.3節。
#[derive(Debug, Clone, Deserialize)]
pub struct CommandError {
    /// 拒否された ClientCommand.type
    pub command_type: String,
    /// エラー内容(人間可読)
    pub message: String,
}

/// シミュレータアプリケーション自体の状態を表す表示用文字列(状態変化時+接続直後)。
/// DETAILED_DESIGN.md 7.7節: WebSocket接続が確立している間、シミュレーションステータス
/// パネルはこの文字列をそのまま表示する。
#[derive(Debug, Clone, Deserialize)]
pub struct AppStatus {
    pub text: String,
}

// --- Client -> Server --------------------------------------------------

/// クライアントからの操作コマンド。DETAILED_DESIGN.md 4.3節。
/// (プレフィックスバイトなし、msgpack本体のみで送信する)
#[derive(Debug, Clone, Serialize, Default)]
pub struct ClientCommand {
    /// "vab_press" / "pause" / "resume" / "set_param" / "set_origin"
    #[serde(rename = "type")]
    pub type_: String,
    /// vab_press時のみ使用
    pub button_id: String,
    /// set_param時のみ使用
    pub value: f64,
    /// set_origin時のみ使用
    pub lat_deg: f64,
    /// set_origin時のみ使用
    pub lon_deg: f64,
}

impl ClientCommand {
    pub fn set_origin(lat_deg: f64, lon_deg: f64) -> Self {
        Self {
            type_: "set_origin".to_string(),
            lat_deg,
            lon_deg,
            ..Default::default()
        }
    }

    pub fn vab_press(button_id: impl Into<String>) -> Self {
        Self {
            type_: "vab_press".to_string(),
            button_id: button_id.into(),
            ..Default::default()
        }
    }
}
