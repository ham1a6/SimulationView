//! WebSocket接続管理。固定長バイナリフレームをそのまま送受信する。
//!
//! 高頻度の`SimState`はサーバー側で常に最新値だけを送信待ちに残す。WebSocket自体は
//! 到達保証ありだが、古いリアルタイム状態をアプリケーション層で廃棄して遅延を蓄積しない。

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use gloo_timers::callback::Timeout;
use js_sys::Uint8Array;
use leptos::prelude::*;
use wasm_bindgen::{closure::Closure, JsCast};
use web_sys::{BinaryType, CloseEvent, MessageEvent, WebSocket};

use crate::protocol::{decode_frame, default_status_panel_config, frame_client_command, AppStatus,
    ClientCommand, CommandError, OriginState, ServerMessage, SimState, StatusPanelConfig, TrackList};

const INITIAL_BACKOFF_MS: u32 = 1_000;
const MAX_BACKOFF_MS: u32 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionStatus { Connecting, Connected, Reconnecting { attempt: u32 }, PausedHidden }
impl fmt::Display for ConnectionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connecting => write!(f, "接続中..."), Self::Connected => write!(f, "接続済み"),
            Self::Reconnecting { attempt } => write!(f, "再接続試行中 (#{attempt})"),
            Self::PausedHidden => write!(f, "再接続一時停止中(タブ非表示)"),
        }
    }
}

#[derive(Clone, Copy)]
pub struct WsSignals {
    pub status: RwSignal<ConnectionStatus>, pub origin: RwSignal<Option<OriginState>>,
    pub status_panel_config: RwSignal<Option<StatusPanelConfig>>,
    pub last_sim_state: RwSignal<Option<SimState>>, pub last_command_error: RwSignal<Option<CommandError>>,
    pub app_status: RwSignal<Option<AppStatus>>, pub track_list: RwSignal<Option<TrackList>>,
}
impl WsSignals {
    pub fn new() -> Self { Self { status: RwSignal::new(ConnectionStatus::Connecting), origin: RwSignal::new(None), status_panel_config: RwSignal::new(Some(default_status_panel_config())), last_sim_state: RwSignal::new(None), last_command_error: RwSignal::new(None), app_status: RwSignal::new(None), track_list: RwSignal::new(None) } }
}
impl Default for WsSignals { fn default() -> Self { Self::new() } }

struct Inner {
    url: String, socket: Option<WebSocket>, reconnect_attempt: u32, tab_visible: bool,
    reconnect_timeout: Option<Timeout>, onopen: Option<Closure<dyn FnMut()>>,
    onmessage: Option<Closure<dyn FnMut(MessageEvent)>>, onerror: Option<Closure<dyn FnMut()>>,
    onclose: Option<Closure<dyn FnMut(CloseEvent)>>,
}
#[derive(Clone)] pub struct WsConnection { signals: WsSignals, inner: Rc<RefCell<Inner>> }
#[derive(Clone, Copy)] pub struct WsHandle(StoredValue<WsConnection, LocalStorage>);
impl WsHandle { pub fn new(conn: WsConnection) -> Self { Self(StoredValue::new_local(conn)) } pub fn send_command(&self, cmd: &ClientCommand) { self.0.with_value(|conn| conn.send_command(cmd)); } }

impl WsConnection {
    pub fn connect_new(url: String, signals: WsSignals) -> Self {
        let connection = Self { signals, inner: Rc::new(RefCell::new(Inner { url, socket: None, reconnect_attempt: 0, tab_visible: true, reconnect_timeout: None, onopen: None, onmessage: None, onerror: None, onclose: None })) };
        connection.setup_visibility_listener(); connection.open_socket(); connection
    }
    pub fn send_command(&self, command: &ClientCommand) {
        let Some(socket) = self.inner.borrow().socket.clone() else { log::warn!("[ws] 切断中のコマンドを破棄しました"); return; };
        let frame = frame_client_command(command);
        if let Err(error) = socket.send_with_u8_array(&frame) { log::warn!("[ws] コマンド送信に失敗: {error:?}"); }
    }
    fn open_socket(&self) {
        self.inner.borrow_mut().reconnect_timeout = None;
        self.signals.status.set(ConnectionStatus::Connecting);
        let socket = match WebSocket::new(&self.inner.borrow().url) { Ok(socket) => socket, Err(error) => { log::error!("[ws] 接続作成に失敗: {error:?}"); self.schedule_reconnect(); return; } };
        socket.set_binary_type(BinaryType::Arraybuffer);
        let this = self.clone();
        let onopen = Closure::<dyn FnMut()>::new(move || { this.inner.borrow_mut().reconnect_attempt = 0; this.signals.status.set(ConnectionStatus::Connected); });
        socket.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        let this = self.clone();
        let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |event| this.handle_message(event));
        socket.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        let onerror = Closure::<dyn FnMut()>::new(|| log::warn!("[ws] 通信エラー"));
        socket.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        let this = self.clone();
        let onclose = Closure::<dyn FnMut(CloseEvent)>::new(move |_| this.schedule_reconnect());
        socket.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        let mut inner = self.inner.borrow_mut();
        inner.socket = Some(socket); inner.onopen = Some(onopen); inner.onmessage = Some(onmessage); inner.onerror = Some(onerror); inner.onclose = Some(onclose);
    }
    fn handle_message(&self, event: MessageEvent) {
        let Ok(buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() else { log::warn!("[ws] バイナリ以外のフレームを破棄しました"); return; };
        match decode_frame(&Uint8Array::new(&buffer).to_vec()) {
            Ok(ServerMessage::SimState(value)) => self.signals.last_sim_state.set(Some(value)),
            Ok(ServerMessage::OriginState(value)) => self.signals.origin.set(Some(value)),
            Ok(ServerMessage::CommandError(value)) => self.signals.last_command_error.set(Some(value)),
            Ok(ServerMessage::AppStatus(value)) => self.signals.app_status.set(Some(value)),
            Ok(ServerMessage::TrackList(value)) => self.signals.track_list.set(Some(value)),
            Err(error) => log::warn!("[ws] 不正フレームを破棄: {error}"),
        }
    }
    fn schedule_reconnect(&self) {
        let mut inner = self.inner.borrow_mut(); inner.socket = None; inner.onopen = None; inner.onmessage = None; inner.onerror = None; inner.onclose = None; inner.reconnect_timeout = None;
        if !inner.tab_visible { self.signals.status.set(ConnectionStatus::PausedHidden); return; }
        let attempt = inner.reconnect_attempt; inner.reconnect_attempt = attempt.saturating_add(1); self.signals.status.set(ConnectionStatus::Reconnecting { attempt: attempt + 1 });
        let delay = INITIAL_BACKOFF_MS.saturating_mul(1 << attempt.min(5)).min(MAX_BACKOFF_MS);
        let this = self.clone(); inner.reconnect_timeout = Some(Timeout::new(delay, move || this.open_socket()));
    }
    fn setup_visibility_listener(&self) {
        let this = self.clone();
        let closure = Closure::<dyn FnMut()>::new(move || {
            let visible = web_sys::window().and_then(|window| window.document()).is_none_or(|document| !document.hidden());
            let reconnect = { let mut inner = this.inner.borrow_mut(); let was_hidden = !inner.tab_visible; inner.tab_visible = visible; if !visible { inner.reconnect_timeout = None; } visible && was_hidden && inner.socket.is_none() };
            if reconnect { this.open_socket(); }
        });
        if let Some(document) = web_sys::window().and_then(|window| window.document()) { let _ = document.add_event_listener_with_callback("visibilitychange", closure.as_ref().unchecked_ref()); }
        closure.forget();
    }
}

fn port_from_query(query: &str) -> u16 { query.trim_start_matches('?').split('&').find_map(|part| part.strip_prefix("sim_port=")).filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())).and_then(|value| value.parse::<u16>().ok()).filter(|port| *port != 0).unwrap_or(9001) }
fn server_port() -> u16 { let query = web_sys::window().and_then(|window| window.location().search().ok()).unwrap_or_default(); port_from_query(&query) }
fn page_host() -> String { web_sys::window().and_then(|window| window.location().hostname().ok()).unwrap_or_else(|| "localhost".to_string()) }
pub fn default_ws_url() -> String { format!("ws://{}:{}/sim", page_host(), server_port()) }
pub fn default_terrain_base_url() -> String { format!("http://{}:{}/terrain", page_host(), server_port()) }
pub fn default_upload_url() -> String { format!("http://{}:{}/uploads", page_host(), server_port()) }

#[cfg(test)] mod tests { use super::*; #[test] fn desktop_port_validation() { assert_eq!(port_from_query("?sim_port=49152"), 49152); assert_eq!(port_from_query("?sim_port=0"), 9001); } }
