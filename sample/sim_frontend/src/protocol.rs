//! WebSocket用の固定長バイナリプロトコル。C++の`webtransport_protocol.hpp`と同じlittle endian配置を読む。

use std::fmt;

const HEADER_SIZE: usize = 8;
const SIM_STATE_SIZE: usize = 48;
const ORIGIN_STATE_SIZE: usize = 16;
const COMMAND_ERROR_SIZE: usize = 8;
const APP_STATUS_SIZE: usize = 8;
const TRACK_SIZE: usize = 88;
const TRACK_LIST_SIZE: usize = 1424;
const MAX_TRACKS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum MsgType { ClientCommand = 1, SimState = 2, OriginState = 3, CommandError = 4, AppStatus = 5, TrackList = 6 }
impl MsgType { fn from_u16(value: u16) -> Option<Self> { match value { 1 => Some(Self::ClientCommand), 2 => Some(Self::SimState), 3 => Some(Self::OriginState), 4 => Some(Self::CommandError), 5 => Some(Self::AppStatus), 6 => Some(Self::TrackList), _ => None } } }

#[derive(Debug, Clone)]
pub enum ServerMessage { SimState(SimState), OriginState(OriginState), CommandError(CommandError), AppStatus(AppStatus), TrackList(TrackList) }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeFrameError { TooShort, UnknownType(u16), InvalidLength { msg_type: MsgType, actual: usize, expected: usize }, ReservedHeader }
impl fmt::Display for DecodeFrameError { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { match self { Self::TooShort => write!(f, "フレームが短すぎます"), Self::UnknownType(id) => write!(f, "未知のmessage_id: 0x{id:04x}"), Self::InvalidLength { msg_type, actual, expected } => write!(f, "{msg_type:?}のサイズが不正: {actual} (期待値 {expected})"), Self::ReservedHeader => write!(f, "予約ヘッダが不正です") } } }
impl std::error::Error for DecodeFrameError {}

fn u16_at(bytes: &[u8], at: usize) -> u16 { u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()) }
fn u32_at(bytes: &[u8], at: usize) -> u32 { u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) }
fn f32_at(bytes: &[u8], at: usize) -> f32 { f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) }
fn f64_at(bytes: &[u8], at: usize) -> f64 { f64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) }
fn expect_size(kind: MsgType, bytes: &[u8], expected: usize) -> Result<(), DecodeFrameError> { if bytes.len() == expected { Ok(()) } else { Err(DecodeFrameError::InvalidLength { msg_type: kind, actual: bytes.len(), expected }) } }

/// `[u16 message_id][u16 reserved=0][u32 payload_size][固定長payload]`を復号する。
pub fn decode_frame(bytes: &[u8]) -> Result<ServerMessage, DecodeFrameError> {
    if bytes.len() < HEADER_SIZE { return Err(DecodeFrameError::TooShort); }
    let message_id = u16_at(bytes, 0);
    let msg_type = MsgType::from_u16(message_id).ok_or(DecodeFrameError::UnknownType(message_id))?;
    if u16_at(bytes, 2) != 0 || u32_at(bytes, 4) as usize != bytes.len() - HEADER_SIZE { return Err(DecodeFrameError::ReservedHeader); }
    let body = &bytes[HEADER_SIZE..];
    match msg_type {
        MsgType::SimState => { expect_size(msg_type, body, SIM_STATE_SIZE)?; Ok(ServerMessage::SimState(SimState { t: f64_at(body, 0), positions: vec![f32_at(body, 8), f32_at(body, 12), f32_at(body, 16)], frame_id: u32_at(body, 20), status_values: vec![f64_at(body, 24), f64_at(body, 32), f64_at(body, 40)] })) }
        MsgType::OriginState => { expect_size(msg_type, body, ORIGIN_STATE_SIZE)?; Ok(ServerMessage::OriginState(OriginState { lat_deg: f64_at(body, 0), lon_deg: f64_at(body, 8) })) }
        MsgType::CommandError => { expect_size(msg_type, body, COMMAND_ERROR_SIZE)?; Ok(ServerMessage::CommandError(CommandError::from_wire(body[0], body[1]))) }
        MsgType::AppStatus => { expect_size(msg_type, body, APP_STATUS_SIZE)?; Ok(ServerMessage::AppStatus(AppStatus { text: if body[0] == 0 { "一時停止中" } else { "シミュレーション実行中" }.to_string() })) }
        MsgType::TrackList => decode_track_list(body).map(ServerMessage::TrackList),
        MsgType::ClientCommand => Err(DecodeFrameError::UnknownType(message_id)),
    }
}

fn decode_track_list(body: &[u8]) -> Result<TrackList, DecodeFrameError> {
    expect_size(MsgType::TrackList, body, TRACK_LIST_SIZE)?;
    let count = (u32_at(body, 8) as usize).min(MAX_TRACKS);
    let mut tracks = Vec::with_capacity(count);
    for index in 0..count {
        let track = &body[16 + index * TRACK_SIZE..16 + (index + 1) * TRACK_SIZE];
        let end = track[64..88].iter().position(|byte| *byte == 0).unwrap_or(24);
        tracks.push(Track { lat_deg: f64_at(track, 0), lon_deg: f64_at(track, 8), alt_m: f64_at(track, 16), heading_deg: f64_at(track, 24), speed_mps: f64_at(track, 32), pitch_deg: f64_at(track, 40), roll_deg: f64_at(track, 48), id: u32_at(track, 56), kind: track[60], affiliation: track[61], alt_ref: track[62], label: String::from_utf8_lossy(&track[64..64 + end]).into_owned() });
    }
    Ok(TrackList { t: f64_at(body, 0), tracks })
}

#[derive(Debug, Clone)] pub struct SimState { pub t: f64, pub positions: Vec<f32>, pub frame_id: u32, pub status_values: Vec<f64> }
#[derive(Debug, Clone, Copy)] pub struct OriginState { pub lat_deg: f64, pub lon_deg: f64 }
#[derive(Debug, Clone)] pub struct StatusItem { pub id: String, pub label: String, pub unit: String }
#[derive(Debug, Clone)] pub struct StatusPanelConfig { pub items: Vec<StatusItem> }
#[derive(Debug, Clone)] pub struct AppStatus { pub text: String }
#[derive(Debug, Clone)] pub struct Track { pub id: u32, pub kind: u8, pub affiliation: u8, pub label: String, pub lat_deg: f64, pub lon_deg: f64, pub alt_m: f64, pub alt_ref: u8, pub heading_deg: f64, pub speed_mps: f64, pub pitch_deg: f64, pub roll_deg: f64 }
#[derive(Debug, Clone)] pub struct TrackList { pub t: f64, pub tracks: Vec<Track> }
#[derive(Debug, Clone)] pub struct CommandError { pub command_type: String, pub message: String }
impl CommandError { fn from_wire(command: u8, code: u8) -> Self { let command_type = match command { 1 => "pause", 2 => "resume", 3 => "set_param", 4 => "set_origin", _ => "unknown" }.to_string(); let message = match code { 2 => "シミュレーション実行中は原点を変更できません", 3 => "指定された緯度経度が地形データの範囲外です", _ => "未対応のコマンドです" }.to_string(); Self { command_type, message } } }

#[derive(Debug, Clone, Copy)] enum Command { Pause = 1, Resume = 2, SetOrigin = 4 }
#[derive(Debug, Clone)] pub struct ClientCommand { command: Command, pub value: f64, pub lat_deg: f64, pub lon_deg: f64 }
impl ClientCommand {
    pub fn set_origin(lat_deg: f64, lon_deg: f64) -> Self { Self { command: Command::SetOrigin, value: 0.0, lat_deg, lon_deg } }
    pub fn resume() -> Self { Self { command: Command::Resume, value: 0.0, lat_deg: 0.0, lon_deg: 0.0 } }
    pub fn pause() -> Self { Self { command: Command::Pause, value: 0.0, lat_deg: 0.0, lon_deg: 0.0 } }
    pub fn encode(&self) -> [u8; 32] { let mut bytes = [0_u8; 32]; bytes[0] = self.command as u8; bytes[8..16].copy_from_slice(&self.value.to_le_bytes()); bytes[16..24].copy_from_slice(&self.lat_deg.to_le_bytes()); bytes[24..32].copy_from_slice(&self.lon_deg.to_le_bytes()); bytes }
}
pub fn frame_client_command(command: &ClientCommand) -> Vec<u8> { let payload = command.encode(); let mut frame = Vec::with_capacity(HEADER_SIZE + payload.len()); frame.extend_from_slice(&(MsgType::ClientCommand as u16).to_le_bytes()); frame.extend_from_slice(&0_u16.to_le_bytes()); frame.extend_from_slice(&(payload.len() as u32).to_le_bytes()); frame.extend_from_slice(&payload); frame }
pub fn default_status_panel_config() -> StatusPanelConfig { StatusPanelConfig { items: vec![StatusItem { id: "elapsed_time".into(), label: "経過時間".into(), unit: "s".into() }, StatusItem { id: "altitude".into(), label: "高度".into(), unit: "m".into() }, StatusItem { id: "speed".into(), label: "速度".into(), unit: "m/s".into() }] } }

#[cfg(test)]
mod tests { use super::*; #[test] fn command_is_a_fixed_cxx_layout() { let frame = frame_client_command(&ClientCommand::set_origin(35.0, 139.0)); assert_eq!(frame.len(), 40); assert_eq!(u16_at(&frame, 0), MsgType::ClientCommand as u16); assert_eq!(f64_at(&frame[HEADER_SIZE..], 16), 35.0); assert_eq!(f64_at(&frame[HEADER_SIZE..], 24), 139.0); } }
