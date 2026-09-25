//! WebTransport 接続管理。信頼ストリームと到達保証なしデータグラムを使い分ける。

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use gloo_timers::callback::Timeout;
use js_sys::{Reflect, Uint8Array};
use leptos::prelude::*;
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};

use crate::protocol::{decode_frame, default_status_panel_config, frame_client_command, AppStatus, ClientCommand, CommandError, OriginState, ServerMessage, SimState, StatusPanelConfig, TrackList};

const INITIAL_BACKOFF_MS: u32 = 1_000;
const MAX_BACKOFF_MS: u32 = 30_000;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = WebTransport)]
    #[derive(Clone)]
    type BrowserWebTransport;
    #[wasm_bindgen(catch, constructor, js_class = WebTransport)] fn new(url: &str) -> Result<BrowserWebTransport, JsValue>;
    #[wasm_bindgen(catch, constructor, js_class = WebTransport)] fn new_with_options(url: &str, options: &JsValue) -> Result<BrowserWebTransport, JsValue>;
    #[wasm_bindgen(method, getter, js_class = WebTransport)] fn ready(this: &BrowserWebTransport) -> js_sys::Promise;
    #[wasm_bindgen(method, getter, js_class = WebTransport, js_name = closed)] fn closed(this: &BrowserWebTransport) -> js_sys::Promise;
    #[wasm_bindgen(method, getter, js_class = WebTransport, js_name = datagrams)] fn datagrams(this: &BrowserWebTransport) -> JsValue;
    #[wasm_bindgen(method, getter, js_class = WebTransport, js_name = incomingUnidirectionalStreams)] fn incoming_unidirectional_streams(this: &BrowserWebTransport) -> JsValue;
    #[wasm_bindgen(method, js_class = WebTransport, js_name = createUnidirectionalStream)] fn create_unidirectional_stream(this: &BrowserWebTransport) -> js_sys::Promise;
    #[wasm_bindgen(js_name = ReadableStream)] type ReadableStream;
    #[wasm_bindgen(method, js_class = ReadableStream, js_name = getReader)] fn get_reader(this: &ReadableStream) -> StreamReader;
    #[wasm_bindgen(js_name = ReadableStreamDefaultReader)] type StreamReader;
    #[wasm_bindgen(method, js_class = ReadableStreamDefaultReader, js_name = read)] fn read(this: &StreamReader) -> js_sys::Promise;
    #[wasm_bindgen(js_name = WritableStream)] type WritableStream;
    #[wasm_bindgen(method, js_class = WritableStream, js_name = getWriter)] fn get_writer(this: &WritableStream) -> StreamWriter;
    #[wasm_bindgen(js_name = WritableStreamDefaultWriter)] type StreamWriter;
    #[wasm_bindgen(method, js_class = WritableStreamDefaultWriter, js_name = write)] fn write(this: &StreamWriter, chunk: &JsValue) -> js_sys::Promise;
    #[wasm_bindgen(method, js_class = WritableStreamDefaultWriter, js_name = close)] fn close(this: &StreamWriter) -> js_sys::Promise;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionStatus { Connecting, Connected, Reconnecting { attempt: u32 }, PausedHidden }
impl fmt::Display for ConnectionStatus { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { match self { Self::Connecting => write!(f, "接続中..."), Self::Connected => write!(f, "接続済み"), Self::Reconnecting { attempt } => write!(f, "再接続試行中 (#{attempt})"), Self::PausedHidden => write!(f, "再接続一時停止中(タブ非表示)") } } }

#[derive(Clone, Copy)]
pub struct WsSignals { pub status: RwSignal<ConnectionStatus>, pub origin: RwSignal<Option<OriginState>>, pub status_panel_config: RwSignal<Option<StatusPanelConfig>>, pub last_sim_state: RwSignal<Option<SimState>>, pub last_command_error: RwSignal<Option<CommandError>>, pub app_status: RwSignal<Option<AppStatus>>, pub track_list: RwSignal<Option<TrackList>> }
impl WsSignals { pub fn new() -> Self { Self { status: RwSignal::new(ConnectionStatus::Connecting), origin: RwSignal::new(None), status_panel_config: RwSignal::new(Some(default_status_panel_config())), last_sim_state: RwSignal::new(None), last_command_error: RwSignal::new(None), app_status: RwSignal::new(None), track_list: RwSignal::new(None) } } }
impl Default for WsSignals { fn default() -> Self { Self::new() } }

struct Inner { url: String, transport: Option<BrowserWebTransport>, reconnect_attempt: u32, tab_visible: bool, reconnect_timeout: Option<Timeout> }
#[derive(Clone)] pub struct WsConnection { signals: WsSignals, inner: Rc<RefCell<Inner>> }
#[derive(Clone, Copy)] pub struct WsHandle(StoredValue<WsConnection, LocalStorage>);
impl WsHandle { pub fn new(conn: WsConnection) -> Self { Self(StoredValue::new_local(conn)) } pub fn send_command(&self, cmd: &ClientCommand) { self.0.with_value(|conn| conn.send_command(cmd)); } }

impl WsConnection {
    pub fn connect_new(url: String, signals: WsSignals) -> Self {
        let connection = Self { signals, inner: Rc::new(RefCell::new(Inner { url, transport: None, reconnect_attempt: 0, tab_visible: true, reconnect_timeout: None })) };
        connection.setup_visibility_listener(); connection.open_transport(); connection
    }
    pub fn send_command(&self, command: &ClientCommand) {
        let Some(transport) = self.inner.borrow().transport.clone() else { log::warn!("[webtransport] 切断中のコマンドを破棄しました"); return; };
        let frame = frame_client_command(command);
        spawn_local(async move {
            let Ok(stream) = JsFuture::from(transport.create_unidirectional_stream()).await else { return; };
            let Ok(stream) = stream.dyn_into::<WritableStream>() else { return; };
            let writer = stream.get_writer();
            let bytes = Uint8Array::from(frame.as_slice());
            let _ = JsFuture::from(writer.write(&bytes.into())).await;
            let _ = JsFuture::from(writer.close()).await;
        });
    }
    fn open_transport(&self) {
        self.inner.borrow_mut().reconnect_timeout = None;
        self.signals.status.set(ConnectionStatus::Connecting);
        let url = self.inner.borrow().url.clone();
        let transport = match webtransport_new(&url) { Ok(value) => value, Err(error) => { log::error!("[webtransport] 作成に失敗: {error:?}"); self.schedule_reconnect(); return; } };
        self.inner.borrow_mut().transport = Some(transport.clone());
        let this = self.clone(); let ready_transport = transport.clone();
        spawn_local(async move {
            if JsFuture::from(ready_transport.ready()).await.is_err() { this.schedule_reconnect(); return; }
            this.inner.borrow_mut().reconnect_attempt = 0;
            this.signals.status.set(ConnectionStatus::Connected);
            this.start_readers(ready_transport);
        });
        let this = self.clone();
        spawn_local(async move { let _ = JsFuture::from(transport.closed()).await; this.schedule_reconnect(); });
    }
    fn start_readers(&self, transport: BrowserWebTransport) {
        let this = self.clone();
        let datagrams = transport.datagrams().dyn_into::<js_sys::Object>().ok().and_then(|value| Reflect::get(&value, &JsValue::from_str("readable")).ok()).and_then(|value| value.dyn_into::<ReadableStream>().ok());
        if let Some(stream) = datagrams { spawn_local(async move { read_frames(stream, move |frame| this.handle_frame(&frame)).await; }); }
        let this = self.clone();
        let streams = transport.incoming_unidirectional_streams().dyn_into::<ReadableStream>();
        if let Ok(streams) = streams { spawn_local(async move { let reader = streams.get_reader(); while let Some(value) = read_value(&reader).await { if let Ok(stream) = value.dyn_into::<ReadableStream>() { let mut bytes = Vec::new(); let stream_reader = stream.get_reader(); while let Some(chunk) = read_value(&stream_reader).await { bytes.extend(Uint8Array::new(&chunk).to_vec()); } this.handle_frame(&bytes); } } }); }
    }
    fn handle_frame(&self, bytes: &[u8]) { match decode_frame(bytes) { Ok(ServerMessage::SimState(value)) => self.signals.last_sim_state.set(Some(value)), Ok(ServerMessage::OriginState(value)) => self.signals.origin.set(Some(value)), Ok(ServerMessage::CommandError(value)) => self.signals.last_command_error.set(Some(value)), Ok(ServerMessage::AppStatus(value)) => self.signals.app_status.set(Some(value)), Ok(ServerMessage::TrackList(value)) => self.signals.track_list.set(Some(value)), Err(error) => log::warn!("[webtransport] 不正フレームを破棄: {error}"), } }
    fn schedule_reconnect(&self) {
        let mut inner = self.inner.borrow_mut(); inner.transport = None; inner.reconnect_timeout = None;
        if !inner.tab_visible { self.signals.status.set(ConnectionStatus::PausedHidden); return; }
        let attempt = inner.reconnect_attempt; inner.reconnect_attempt = attempt.saturating_add(1); self.signals.status.set(ConnectionStatus::Reconnecting { attempt: attempt + 1 });
        let delay = INITIAL_BACKOFF_MS.saturating_mul(1u32 << attempt.min(5)).min(MAX_BACKOFF_MS);
        let this = self.clone(); inner.reconnect_timeout = Some(Timeout::new(delay, move || this.open_transport()));
    }
    fn setup_visibility_listener(&self) { let this = self.clone(); let closure = Closure::<dyn FnMut()>::new(move || { let visible = web_sys::window().and_then(|window| window.document()).is_none_or(|document| !document.hidden()); let reconnect = { let mut inner = this.inner.borrow_mut(); let was_hidden = !inner.tab_visible; inner.tab_visible = visible; if !visible { inner.reconnect_timeout = None; } visible && was_hidden && inner.transport.is_none() }; if reconnect { this.open_transport(); } }); if let Some(document) = web_sys::window().and_then(|window| window.document()) { let _ = document.add_event_listener_with_callback("visibilitychange", closure.as_ref().unchecked_ref()); } closure.forget(); }
}

async fn read_value(reader: &StreamReader) -> Option<JsValue> { let result = JsFuture::from(reader.read()).await.ok()?; if Reflect::get(&result, &JsValue::from_str("done")).ok()?.as_bool().unwrap_or(false) { None } else { Reflect::get(&result, &JsValue::from_str("value")).ok() } }
async fn read_frames(stream: ReadableStream, mut handler: impl FnMut(Vec<u8>) + 'static) { let reader = stream.get_reader(); while let Some(value) = read_value(&reader).await { handler(Uint8Array::new(&value).to_vec()); } }

fn page_host_and_tls() -> (String, bool) { let location = web_sys::window().map(|window| window.location()); let hostname = location.as_ref().and_then(|location| location.hostname().ok()).unwrap_or_else(|| "localhost".to_string()); let is_tls = location.and_then(|location| location.protocol().ok()).is_some_and(|protocol| protocol == "https:"); (hostname, is_tls) }
fn webtransport_new(url: &str) -> Result<BrowserWebTransport, JsValue> {
    let query = web_sys::window().and_then(|window| window.location().search().ok()).unwrap_or_default();
    let Some(hash) = query.trim_start_matches('?').split('&').find_map(|part| part.strip_prefix("wt_cert_hash=")) else { return BrowserWebTransport::new(url); };
    let mut base64 = hash.replace('-', "+").replace('_', "/");
    while base64.len() % 4 != 0 { base64.push('='); }
    let decoded = web_sys::window().ok_or_else(|| JsValue::from_str("windowがありません"))?.atob(&base64)?;
    let bytes: Vec<u8> = decoded.bytes().collect();
    let options = js_sys::Object::new(); let hashes = js_sys::Array::new(); let certificate = js_sys::Object::new();
    Reflect::set(&certificate, &JsValue::from_str("algorithm"), &JsValue::from_str("sha-256"))?;
    Reflect::set(&certificate, &JsValue::from_str("value"), &Uint8Array::from(bytes.as_slice()).into())?;
    hashes.push(&certificate); Reflect::set(&options, &JsValue::from_str("serverCertificateHashes"), &hashes.into())?;
    BrowserWebTransport::new_with_options(url, &options.into())
}
fn port_from_query(query: &str) -> u16 { query.trim_start_matches('?').split('&').find_map(|part| part.strip_prefix("sim_port=")).filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())).and_then(|value| value.parse::<u16>().ok()).filter(|port| *port != 0).unwrap_or(9001) }
fn server_port() -> u16 { let query = web_sys::window().and_then(|window| window.location().search().ok()).unwrap_or_default(); port_from_query(&query) }
/// 互換名を維持するが、返すのは `https://…/sim` のWebTransport URL。
pub fn default_ws_url() -> String { let (hostname, _) = page_host_and_tls(); format!("https://{hostname}:{}/sim", server_port()) }
pub fn default_terrain_base_url() -> String { format!("{}/terrain", default_http_base_url()) }
pub fn default_upload_url() -> String { format!("{}/uploads", default_http_base_url()) }
fn default_http_base_url() -> String { let (hostname, _) = page_host_and_tls(); format!("https://{hostname}:{}", server_port()) }

#[cfg(test)] mod tests { use super::*; #[test] fn desktop_port_validation() { assert_eq!(port_from_query("?sim_port=49152"), 49152); assert_eq!(port_from_query("?sim_port=0"), 9001); } }
