//! C++ の `WebTransportMessaging` と HTTP/3/WebTransport 実装をつなぐ最小の C ABI。
//! C++ のコールバックは Tokio ワーカースレッドから呼ばれるため、受信後の状態変更は
//! C++ 側でスレッドセーフなキューへ渡すこと。

use std::collections::HashMap;
use std::ffi::{c_char, CStr};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::runtime::{Handle, Runtime};
use wtransport::{Connection, Endpoint, Identity, ServerConfig};

const RELIABLE_STREAM: u8 = 0;
const UNRELIABLE_DATAGRAM: u8 = 1;
const MAX_FRAME_BYTES: usize = 64 * 1024;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Sim3dviewWtCallbacks {
    connected: Option<extern "C" fn(*mut std::ffi::c_void, u64)>,
    disconnected: Option<extern "C" fn(*mut std::ffi::c_void, u64)>,
    received: Option<extern "C" fn(*mut std::ffi::c_void, u64, u8, *const u8, usize)>,
}

struct Bridge {
    sessions: Mutex<HashMap<u64, Connection>>,
    callbacks: Sim3dviewWtCallbacks,
    context: usize,
    next_client_id: AtomicU64,
    runtime: Handle,
}

static BRIDGE: OnceLock<Arc<Bridge>> = OnceLock::new();

fn write_error(destination: *mut c_char, capacity: usize, message: &str) {
    if destination.is_null() || capacity == 0 {
        return;
    }
    let bytes = message.as_bytes();
    let copy_len = bytes.len().min(capacity.saturating_sub(1));
    // 呼び出し側が capacity バイトを確保しているという C ABI 契約。
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), destination.cast::<u8>(), copy_len);
        *destination.add(copy_len) = 0;
    }
}

fn path_from_c(value: *const c_char, name: &str) -> Result<String, String> {
    if value.is_null() {
        return Err(format!("{name} が指定されていません"));
    }
    unsafe { CStr::from_ptr(value) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| format!("{name} がUTF-8ではありません"))
}

/// HTTP/3 の UDP endpoint を開始する。TCP の HTTP 静的配信と同じポート番号を使える。
#[no_mangle]
pub extern "C" fn sim3dview_wt_start(
    port: u16,
    cert_file: *const c_char,
    key_file: *const c_char,
    context: *mut std::ffi::c_void,
    callbacks: Sim3dviewWtCallbacks,
    error: *mut c_char,
    error_size: usize,
) -> i32 {
    if BRIDGE.get().is_some() {
        write_error(error, error_size, "WebTransportは既に開始されています");
        return -1;
    }
    let cert_file = match path_from_c(cert_file, "cert_file") {
        Ok(value) => value,
        Err(message) => {
            write_error(error, error_size, &message);
            return -1;
        }
    };
    let key_file = match path_from_c(key_file, "key_file") {
        Ok(value) => value,
        Err(message) => {
            write_error(error, error_size, &message);
            return -1;
        }
    };

    let runtime = match Runtime::new() {
        Ok(value) => value,
        Err(source) => {
            write_error(error, error_size, &format!("Tokio runtimeの作成に失敗: {source}"));
            return -1;
        }
    };
    let endpoint = match runtime.block_on(async {
        let identity = Identity::load_pemfiles(cert_file, key_file)
            .await
            .map_err(|source| source.to_string())?;
        let config = ServerConfig::builder()
            .with_bind_default(port)
            .with_identity(identity)
            .build();
        Endpoint::server(config).map_err(|source| source.to_string())
    }) {
        Ok(value) => value,
        Err(source) => {
            write_error(error, error_size, &format!("WebTransport待受の開始に失敗: {source}"));
            return -1;
        }
    };

    let bridge = Arc::new(Bridge {
        sessions: Mutex::new(HashMap::new()),
        callbacks,
        context: context as usize,
        next_client_id: AtomicU64::new(1),
        runtime: runtime.handle().clone(),
    });
    if BRIDGE.set(bridge.clone()).is_err() {
        write_error(error, error_size, "WebTransportの初期化競合が発生しました");
        return -1;
    }
    std::thread::Builder::new()
        .name("sim3dview-webtransport".into())
        .spawn(move || runtime.block_on(accept_loop(endpoint, bridge)))
        .expect("WebTransportスレッドの作成に失敗しました");
    0
}

async fn accept_loop(
    endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    bridge: Arc<Bridge>,
) {
    loop {
        let incoming = endpoint.accept().await;
        let bridge = bridge.clone();
        tokio::spawn(async move {
            let request = match incoming.await {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("[webtransport] 接続確立に失敗: {error}");
                    return;
                }
            };
            if request.path() != "/sim" {
                request.not_found().await;
                return;
            }
            let connection = match request.accept().await {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("[webtransport] セッション受理に失敗: {error}");
                    return;
                }
            };
            let client_id = bridge.next_client_id.fetch_add(1, Ordering::Relaxed);
            bridge
                .sessions
                .lock()
                .expect("sessions mutex poisoned")
                .insert(client_id, connection.clone());
            if let Some(callback) = bridge.callbacks.connected {
                callback(bridge.context as *mut std::ffi::c_void, client_id);
            }
            receive_loop(connection, client_id, bridge.clone()).await;
            bridge
                .sessions
                .lock()
                .expect("sessions mutex poisoned")
                .remove(&client_id);
            if let Some(callback) = bridge.callbacks.disconnected {
                callback(bridge.context as *mut std::ffi::c_void, client_id);
            }
        });
    }
}

async fn receive_loop(connection: Connection, client_id: u64, bridge: Arc<Bridge>) {
    loop {
        tokio::select! {
            datagram = connection.receive_datagram() => match datagram {
                Ok(data) => deliver(&bridge, client_id, UNRELIABLE_DATAGRAM, &data),
                Err(_) => return,
            },
            stream = connection.accept_uni() => match stream {
                Ok(mut stream) => {
                    let bridge = bridge.clone();
                    tokio::spawn(async move {
                        let mut frame = Vec::new();
                        let mut buffer = [0_u8; 4096];
                        loop {
                            match stream.read(&mut buffer).await {
                                Ok(Some(read)) => {
                                    if frame.len() + read > MAX_FRAME_BYTES { return; }
                                    frame.extend_from_slice(&buffer[..read]);
                                }
                                Ok(None) => break,
                                Err(_) => return,
                            }
                        }
                        deliver(&bridge, client_id, RELIABLE_STREAM, &frame);
                    });
                }
                Err(_) => return,
            },
            _ = connection.closed() => return,
        }
    }
}

fn deliver(bridge: &Bridge, client_id: u64, delivery: u8, data: &[u8]) {
    if data.len() > MAX_FRAME_BYTES {
        return;
    }
    if let Some(callback) = bridge.callbacks.received {
        callback(
            bridge.context as *mut std::ffi::c_void,
            client_id,
            delivery,
            data.as_ptr(),
            data.len(),
        );
    }
}

#[no_mangle]
pub extern "C" fn sim3dview_wt_send(
    client_id: u64,
    delivery: u8,
    data: *const u8,
    size: usize,
) -> i32 {
    let Some(bridge) = BRIDGE.get().cloned() else { return -1; };
    if data.is_null() || size == 0 || size > MAX_FRAME_BYTES || delivery > UNRELIABLE_DATAGRAM {
        return -1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(data, size) }.to_vec();
    let connection = match bridge.sessions.lock().expect("sessions mutex poisoned").get(&client_id).cloned() {
        Some(value) => value,
        None => return -1,
    };
    bridge.runtime.spawn(send_one(connection, delivery, bytes));
    0
}

#[no_mangle]
pub extern "C" fn sim3dview_wt_broadcast(delivery: u8, data: *const u8, size: usize) -> i32 {
    let Some(bridge) = BRIDGE.get().cloned() else { return -1; };
    if data.is_null() || size == 0 || size > MAX_FRAME_BYTES || delivery > UNRELIABLE_DATAGRAM {
        return -1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(data, size) }.to_vec();
    let connections: Vec<_> = bridge
        .sessions
        .lock()
        .expect("sessions mutex poisoned")
        .values()
        .cloned()
        .collect();
    for connection in connections {
        bridge.runtime.spawn(send_one(connection, delivery, bytes.clone()));
    }
    0
}

async fn send_one(connection: Connection, delivery: u8, data: Vec<u8>) {
    if delivery == UNRELIABLE_DATAGRAM {
        let _ = connection.send_datagram(&data);
        return;
    }
    let Ok(opening) = connection.open_uni().await else { return; };
    let Ok(mut stream) = opening.await else { return; };
    if stream.write_all(&data).await.is_ok() {
        let _ = stream.finish().await;
    }
}
