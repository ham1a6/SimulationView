//! WebSocket接続管理。
//!
//! DETAILED_DESIGN.md 4.5節: 自動再接続(指数バックオフ1s→30s+ジッター)、
//! ブラウザタブが非表示の間は再接続を一時停止(Page Visibility API)。
//! バイナリフレームをTextとして誤受信しないよう、`set_binary_type(BinaryType::Arraybuffer)` を必ず設定する。

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use gloo_timers::callback::Timeout;
use js_sys::Uint8Array;
use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::{BinaryType, CloseEvent, MessageEvent, WebSocket};

use crate::protocol::{
    decode_frame, AppStatus, ClientCommand, CommandError, OriginState, ServerMessage, SimState,
    StatusPanelConfig, TrackList, VabConfig,
};

const INITIAL_BACKOFF_MS: u32 = 1_000;
const MAX_BACKOFF_MS: u32 = 30_000;
const JITTER_MS: f64 = 300.0; // ±300ms

fn reconnect_delay_ms(attempt: u32, jitter_ms: i64) -> u32 {
    let base_ms = INITIAL_BACKOFF_MS
        .saturating_mul(1u32 << attempt.min(5))
        .min(MAX_BACKOFF_MS);
    (i64::from(base_ms) + jitter_ms).clamp(200, i64::from(MAX_BACKOFF_MS)) as u32
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Reconnecting {
        attempt: u32,
    },
    /// タブが非表示のため再接続を一時停止中。
    PausedHidden,
}

impl fmt::Display for ConnectionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectionStatus::Connecting => write!(f, "接続中..."),
            ConnectionStatus::Connected => write!(f, "接続済み"),
            ConnectionStatus::Reconnecting { attempt } => {
                write!(f, "再接続試行中 (#{attempt})")
            }
            ConnectionStatus::PausedHidden => write!(f, "再接続一時停止中(タブ非表示)"),
        }
    }
}

/// サーバーから配信される状態を保持するシグナル群。
/// RwSignalはCopyなので、この構造体ごとコンポーネント間で自由に受け渡せる。
#[derive(Clone, Copy)]
pub struct WsSignals {
    pub status: RwSignal<ConnectionStatus>,
    pub origin: RwSignal<Option<OriginState>>,
    pub vab_config: RwSignal<Option<VabConfig>>,
    pub status_panel_config: RwSignal<Option<StatusPanelConfig>>,
    pub last_sim_state: RwSignal<Option<SimState>>,
    pub last_command_error: RwSignal<Option<CommandError>>,
    /// シミュレータアプリケーション自体の状態文字列(DETAILED_DESIGN.md 7.7節)。
    pub app_status: RwSignal<Option<AppStatus>>,
    /// 航跡(航空機・艦船・車両等)の最新の一覧。`track_bridge.rs`がsim3dviewライブラリの`TracksState`へ橋渡しする。
    pub track_list: RwSignal<Option<TrackList>>,
}

impl WsSignals {
    pub fn new() -> Self {
        Self {
            status: RwSignal::new(ConnectionStatus::Connecting),
            origin: RwSignal::new(None),
            vab_config: RwSignal::new(None),
            status_panel_config: RwSignal::new(None),
            last_sim_state: RwSignal::new(None),
            last_command_error: RwSignal::new(None),
            app_status: RwSignal::new(None),
            track_list: RwSignal::new(None),
        }
    }
}

impl Default for WsSignals {
    fn default() -> Self {
        Self::new()
    }
}

/// WebSocket本体・再接続タイマー・イベントクロージャなど、
/// 可変な接続管理状態。Rc<RefCell<>>で共有する。
struct Inner {
    url: String,
    socket: Option<WebSocket>,
    reconnect_attempt: u32,
    tab_visible: bool,
    reconnect_timeout: Option<Timeout>,
    // JS側からコールバックされる間、クロージャの生存を保証するために保持する。
    // 新しい接続を張るたびに古いクロージャは(Optionへの再代入で)破棄される。
    onopen: Option<Closure<dyn FnMut()>>,
    onmessage: Option<Closure<dyn FnMut(MessageEvent)>>,
    onerror: Option<Closure<dyn FnMut()>>,
    onclose: Option<Closure<dyn FnMut(CloseEvent)>>,
}

/// WebSocket接続を管理するハンドル。clone()すると同じ接続を共有する(Rc)。
#[derive(Clone)]
pub struct WsConnection {
    signals: WsSignals,
    inner: Rc<RefCell<Inner>>,
}

/// `WsConnection`をコンポーネントへ渡すためのハンドル。`Copy`で`Send`+`Sync`(Leptosの`view!`の子や
/// `provide_context`が要求する境界)を満たし、`unsafe`なしで`Rc<RefCell<..>>`を持つ`WsConnection`を
/// 共有できる(本体はこのスレッド専用の`StoredValue`に入り、ハンドルは指すだけ)。
#[derive(Clone, Copy)]
pub struct WsHandle(StoredValue<WsConnection, LocalStorage>);

impl WsHandle {
    pub fn new(conn: WsConnection) -> Self {
        Self(StoredValue::new_local(conn))
    }

    pub fn send_command(&self, cmd: &ClientCommand) {
        self.0.with_value(|conn| conn.send_command(cmd));
    }
}

impl WsConnection {
    /// 接続を開始する。返り値を保持し続ける必要はない
    /// (内部のイベントクロージャがselfを保持するため、接続が生きている間は破棄されない)。
    pub fn connect_new(url: String, signals: WsSignals) -> Self {
        let conn = WsConnection {
            signals,
            inner: Rc::new(RefCell::new(Inner {
                url,
                socket: None,
                reconnect_attempt: 0,
                tab_visible: true,
                reconnect_timeout: None,
                onopen: None,
                onmessage: None,
                onerror: None,
                onclose: None,
            })),
        };
        conn.setup_visibility_listener();
        conn.open_socket();
        conn
    }

    pub fn send_command(&self, cmd: &ClientCommand) {
        let inner = self.inner.borrow();
        if let Some(ws) = &inner.socket {
            match rmp_serde::to_vec(cmd) {
                Ok(bytes) => {
                    if let Err(e) = ws.send_with_u8_array(&bytes) {
                        log::error!("[ws] send failed: {e:?}");
                    }
                }
                Err(e) => log::error!("[ws] failed to encode ClientCommand: {e}"),
            }
        } else {
            log::warn!("[ws] send_command called while disconnected, dropping: {cmd:?}");
        }
    }

    fn open_socket(&self) {
        {
            let mut inner = self.inner.borrow_mut();
            inner.reconnect_timeout = None;
        }
        self.signals.status.set(ConnectionStatus::Connecting);

        let url = self.inner.borrow().url.clone();
        let ws = match WebSocket::new(&url) {
            Ok(ws) => ws,
            Err(e) => {
                log::error!("[ws] WebSocket::new failed: {e:?}");
                self.schedule_reconnect();
                return;
            }
        };
        ws.set_binary_type(BinaryType::Arraybuffer);

        let this = self.clone();
        let onopen = Closure::<dyn FnMut()>::new(move || {
            log::info!("[ws] connected");
            this.inner.borrow_mut().reconnect_attempt = 0;
            this.signals.status.set(ConnectionStatus::Connected);
        });
        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));

        let this = self.clone();
        let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            this.handle_message(event);
        });
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));

        // WebSocketの"error"イベントは(ErrorEventではなく)常にプレーンなEventとして発火するため、
        // イベント引数の型を要求しないクロージャにする(ErrorEventとして解釈しようとするとpanicする)。
        let onerror = Closure::<dyn FnMut()>::new(move || {
            log::error!("[ws] error event fired");
        });
        ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));

        let this = self.clone();
        let onclose = Closure::<dyn FnMut(CloseEvent)>::new(move |_event: CloseEvent| {
            log::warn!("[ws] closed, scheduling reconnect");
            this.schedule_reconnect();
        });
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));

        let mut inner = self.inner.borrow_mut();
        inner.socket = Some(ws);
        inner.onopen = Some(onopen);
        inner.onmessage = Some(onmessage);
        inner.onerror = Some(onerror);
        inner.onclose = Some(onclose);
    }

    fn handle_message(&self, event: MessageEvent) {
        let array_buffer = match event.data().dyn_into::<js_sys::ArrayBuffer>() {
            Ok(buf) => buf,
            Err(_) => {
                log::warn!("[ws] received non-binary message, ignoring");
                return;
            }
        };
        let bytes = Uint8Array::new(&array_buffer).to_vec();
        let message = match decode_frame(&bytes) {
            Ok(message) => message,
            Err(error) => {
                log::warn!("[ws] frame ignored: {error}");
                return;
            }
        };
        match message {
            ServerMessage::SimState(state) => self.signals.last_sim_state.set(Some(state)),
            ServerMessage::VabConfig(cfg) => self.signals.vab_config.set(Some(cfg)),
            ServerMessage::OriginState(origin) => self.signals.origin.set(Some(origin)),
            ServerMessage::StatusPanelConfig(cfg) => {
                self.signals.status_panel_config.set(Some(cfg));
            }
            ServerMessage::CommandError(err) => {
                log::warn!("[ws] CommandError: {} ({})", err.message, err.command_type);
                self.signals.last_command_error.set(Some(err));
            }
            ServerMessage::AppStatus(status) => self.signals.app_status.set(Some(status)),
            ServerMessage::TrackList(list) => self.signals.track_list.set(Some(list)),
        }
    }

    fn schedule_reconnect(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.socket = None;
        inner.onopen = None;
        inner.onmessage = None;
        inner.onerror = None;
        inner.onclose = None;
        inner.reconnect_timeout = None;

        if !inner.tab_visible {
            // DETAILED_DESIGN.md 4.5節: タブが非表示の間は再接続を試みない。
            self.signals.status.set(ConnectionStatus::PausedHidden);
            return;
        }

        let attempt = inner.reconnect_attempt;
        inner.reconnect_attempt = attempt.saturating_add(1);
        self.signals.status.set(ConnectionStatus::Reconnecting {
            attempt: attempt + 1,
        });

        let jitter_ms = (js_sys::Math::random() * (2.0 * JITTER_MS) - JITTER_MS).round() as i64;
        let delay_ms = reconnect_delay_ms(attempt, jitter_ms);

        let this = self.clone();
        let timeout = Timeout::new(delay_ms, move || {
            this.open_socket();
        });
        inner.reconnect_timeout = Some(timeout);
    }

    fn setup_visibility_listener(&self) {
        let this = self.clone();
        let closure = Closure::<dyn FnMut()>::new(move || {
            let hidden = web_sys::window()
                .and_then(|w| w.document())
                .map(|d| d.hidden())
                .unwrap_or(false);
            this.set_tab_visible(!hidden);
        });

        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            let _ = document.add_event_listener_with_callback(
                "visibilitychange",
                closure.as_ref().unchecked_ref(),
            );
        }
        // アプリケーション生存期間中ずっと必要なリスナーなのでforgetしてよい。
        closure.forget();
    }

    fn set_tab_visible(&self, visible: bool) {
        let should_reconnect_now = {
            let mut inner = self.inner.borrow_mut();
            let was_hidden = !inner.tab_visible;
            inner.tab_visible = visible;
            if !visible {
                // 非表示になった: 保留中の再接続タイマーを止める。
                inner.reconnect_timeout = None;
            }
            // 再表示され、かつ現在未接続なら即座に再接続を試みる。
            visible && was_hidden && inner.socket.is_none()
        };
        if should_reconnect_now {
            self.open_socket();
        }
    }
}

/// 現在のページの(ホスト名, HTTPSか)。取れなければ("localhost", false)。
fn page_host_and_tls() -> (String, bool) {
    let location = web_sys::window().map(|w| w.location());
    let hostname = location
        .as_ref()
        .and_then(|loc| loc.hostname().ok())
        .unwrap_or_else(|| "localhost".to_string());
    // HTTPSのページから平文のws://・http://へは、ブラウザのMixed Content規則で接続できない。
    // sim_serverも同じ証明書でHTTPS/WSSを待ち受ける前提(README.mdの「HTTPS」節)で、
    // ページのスキームに合わせて選ぶ。
    let is_tls = location
        .and_then(|loc| loc.protocol().ok())
        .is_some_and(|p| p == "https:");
    (hostname, is_tls)
}

/// デスクトップ版のポート指定を検証し、省略・不正値ではブラウザ版の既定へ戻す。
fn port_from_query(query: &str) -> u16 {
    query
        .trim_start_matches('?')
        .split('&')
        .find_map(|part| part.strip_prefix("sim_port="))
        .filter(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port != 0)
        .unwrap_or(9001)
}

// Electronは自身のサーバーの割当ポートをURLに渡す。ブラウザ版は従来どおり9001。
fn server_port() -> u16 {
    let query = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    port_from_query(&query)
}

/// 接続先WebSocket URLを、現在のページと省略可能なポート指定から組み立てる。
pub fn default_ws_url() -> String {
    let (hostname, is_tls) = page_host_and_tls();
    let scheme = if is_tls { "wss" } else { "ws" };
    let port = server_port();
    format!("{scheme}://{hostname}:{port}/sim")
}

/// `sim3dview::terrain::store::TerrainStore::new()`へ渡す
/// 地形データ配信のベースURL。sim3dviewライブラリはサーバーのホスト名・ポートを知らないため
/// (`sim3dview/README.md`のデータ契約参照)、このサンプルアプリ側でsample/sim_serverの規約
/// (`/terrain/*`、ポート9001)に基づいて組み立てる。
pub fn default_terrain_base_url() -> String {
    let (hostname, is_tls) = page_host_and_tls();
    let scheme = if is_tls { "https" } else { "http" };
    let port = server_port();
    format!("{scheme}://{hostname}:{port}/terrain")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_port_preserves_browser_default_and_rejects_invalid_values() {
        for query in [
            "",
            "?other=123",
            "?sim_port=0",
            "?sim_port=65536",
            "?sim_port=-1",
            "?sim_port=+80",
            "?sim_port=abc",
        ] {
            assert_eq!(port_from_query(query), 9001, "{query}");
        }
        assert_eq!(port_from_query("?sim_port=49152"), 49152);
        assert_eq!(port_from_query("?other=1&sim_port=65535&more=2"), 65535);
    }

    #[test]
    fn reconnect_backoff_saturates_and_clamps_jitter() {
        assert_eq!(reconnect_delay_ms(0, 0), 1_000);
        assert_eq!(reconnect_delay_ms(1, -300), 1_700);
        assert_eq!(reconnect_delay_ms(5, 0), MAX_BACKOFF_MS);
        assert_eq!(reconnect_delay_ms(99, 300), MAX_BACKOFF_MS);
        assert_eq!(reconnect_delay_ms(0, -9_999), 200);
    }
}
