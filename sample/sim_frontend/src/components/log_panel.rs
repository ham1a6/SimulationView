//! 画面最下部のログパネル(DETAILED_DESIGN.md 7.10節)。画面幅いっぱいで、3行分の高さに
//! 時刻・レベル・本文を1件ずつ並べる。
//!
//! 追記は`LogState`(contextで共有)へ行う。アプリの出来事(接続状態・コマンド拒否など)は
//! `log_bridge.rs`が、`log`クレートの警告・エラー(ライブラリのものを含む)は`init_logger`で
//! 差し込んだロガーがここへ流す。
//!
//! スクロール: 自動スクロール中は追記のたびに最下部へ移動する。ユーザーが上へスクロールすると
//! 自動スクロールを止めて位置を保ち、最下部まで戻すと再開する。右端のボタンでも切り替えられる。

use std::collections::VecDeque;
use std::sync::{Arc, OnceLock};

use leptos::prelude::*;

/// 保持する最大件数。超えたら古いものから捨てる(長時間動かしても増え続けないように)。
pub const MAX_LOG_ENTRIES: usize = 1000;

/// 最下部とみなす残りスクロール量(px)。高DPIでscrollTopが整数へ丸められる分の余裕。
const BOTTOM_TOLERANCE_PX: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    fn label(self) -> &'static str {
        match self {
            Self::Info => "情報",
            Self::Warn => "警告",
            Self::Error => "エラー",
        }
    }

    fn class(self) -> &'static str {
        match self {
            Self::Info => "log-entry log-info",
            Self::Warn => "log-entry log-warn",
            Self::Error => "log-entry log-error",
        }
    }
}

#[derive(Debug)]
pub struct LogEntry {
    /// `<For>`のキー。追記順の連番で、クリアしても振り直さない。
    pub id: u64,
    /// ローカル時刻の`HH:MM:SS`。
    pub time: String,
    pub level: LogLevel,
    pub text: String,
}

/// 件数上限つきのログ本体。描画のたびに全件を複製するため、要素は`Arc`で持つ。
#[derive(Debug, Default)]
struct LogBuffer {
    entries: VecDeque<Arc<LogEntry>>,
    next_id: u64,
}

impl LogBuffer {
    fn push(&mut self, level: LogLevel, time: String, text: String) {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push_back(Arc::new(LogEntry { id, time, level, text }));
        while self.entries.len() > MAX_LOG_ENTRIES {
            self.entries.pop_front();
        }
    }
}

/// ログの共有状態。`App`で1つ作ってcontextへ提供する。
#[derive(Clone, Copy)]
pub struct LogState {
    buffer: RwSignal<LogBuffer>,
}

impl LogState {
    pub fn new() -> Self {
        Self { buffer: RwSignal::new(LogBuffer::default()) }
    }

    pub fn push(&self, level: LogLevel, text: impl Into<String>) {
        let text = text.into();
        let time = now_hms();
        // ページ終了時など、シグナル破棄後に呼ばれても落とさない。
        let _ = self.buffer.try_update(|buffer| buffer.push(level, time, text));
    }

    pub fn info(&self, text: impl Into<String>) {
        self.push(LogLevel::Info, text);
    }

    pub fn warn(&self, text: impl Into<String>) {
        self.push(LogLevel::Warn, text);
    }

    pub fn error(&self, text: impl Into<String>) {
        self.push(LogLevel::Error, text);
    }

    pub fn clear(&self) {
        self.buffer.update(|buffer| buffer.entries.clear());
    }

    /// `log`クレートの警告・エラーをこの状態へ流すようにする(`init_logger`と組で使う)。
    pub fn install_as_log_sink(self) {
        let _ = LOG_SINK.set(self);
    }
}

impl Default for LogState {
    fn default() -> Self {
        Self::new()
    }
}

fn format_hms(hours: u32, minutes: u32, seconds: u32) -> String {
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn now_hms() -> String {
    let date = js_sys::Date::new_0();
    format_hms(date.get_hours(), date.get_minutes(), date.get_seconds())
}

static LOG_SINK: OnceLock<LogState> = OnceLock::new();

/// ブラウザのコンソールへ出しつつ、警告以上をログパネルへも流すロガー。
struct PanelLogger {
    level: log::Level,
}

impl log::Log for PanelLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        console_log::log(record);
        let level = match record.level() {
            log::Level::Error => LogLevel::Error,
            log::Level::Warn => LogLevel::Warn,
            _ => return,
        };
        let Some(state) = LOG_SINK.get().copied() else { return };
        let text = record.args().to_string();
        // ログはシグナルの読み書きの途中からも呼ばれうる。同じ呼び出しの中で書き込まず、
        // 現在の処理が終わってから追記する。
        wasm_bindgen_futures::spawn_local(async move { state.push(level, text) });
    }

    fn flush(&self) {}
}

/// `console_log::init_with_level`の代わりに使う。`level`以上をコンソールへ出す。
pub fn init_logger(level: log::Level) -> Result<(), log::SetLoggerError> {
    static LOGGER: OnceLock<PanelLogger> = OnceLock::new();
    let logger = LOGGER.get_or_init(|| PanelLogger { level });
    log::set_logger(logger)?;
    log::set_max_level(level.to_level_filter());
    Ok(())
}

/// スクロール後の自動スクロールの状態を決める。最下部なら再開し、上へ動いたときだけ止める。
/// 追記後の最下部への移動は下向きなので止めない。内容が減ってscrollTopが切り詰められた場合は
/// 最下部に張り付くので、先に最下部を判定して止めないようにする。
fn follow_after_scroll(
    following: bool,
    previous_top: i32,
    top: i32,
    scroll_height: i32,
    client_height: i32,
) -> bool {
    if scroll_height - top - client_height <= BOTTOM_TOLERANCE_PX {
        true
    } else if top < previous_top {
        false
    } else {
        following
    }
}

#[component]
pub fn LogPanel() -> impl IntoView {
    let log = use_context::<LogState>().expect("LogState context not found");
    let list = NodeRef::<leptos::html::Div>::new();
    let following = RwSignal::new(true);
    let last_top = StoredValue::new(0_i32);

    // 追記・自動スクロールの再開のたびに、描画が済んでから最下部へ移動する。
    Effect::new(move |_| {
        log.buffer.track();
        if following.get() {
            request_animation_frame(move || {
                if let Some(element) = list.get_untracked() {
                    element.set_scroll_top(element.scroll_height());
                }
            });
        }
    });

    let on_scroll = move |_| {
        let Some(element) = list.get_untracked() else { return };
        let top = element.scroll_top();
        let next = follow_after_scroll(
            following.get_untracked(),
            last_top.get_value(),
            top,
            element.scroll_height(),
            element.client_height(),
        );
        last_top.set_value(top);
        if next != following.get_untracked() {
            following.set(next);
        }
    };

    let entries = move || log.buffer.with(|buffer| buffer.entries.iter().cloned().collect::<Vec<_>>());
    let follow_title = move || {
        if following.get() {
            "自動スクロール中(クリックで停止)"
        } else {
            "自動スクロール停止中(クリックで再開)"
        }
    };

    view! {
        <div class="log-panel">
            <div class="log-list" node_ref=list on:scroll=on_scroll role="log" aria-label="ログ">
                <For
                    each=entries
                    key=|entry| entry.id
                    children=|entry| {
                        let class = entry.level.class();
                        let label = entry.level.label();
                        view! {
                            <div class=class>
                                <span class="log-time">{entry.time.clone()}</span>
                                <span class="log-level">{label}</span>
                                <span class="log-text">{entry.text.clone()}</span>
                            </div>
                        }
                    }
                />
            </div>
            <div class="log-tools">
                <button
                    class="log-tool-button"
                    class:active=move || following.get()
                    aria-pressed=move || following.get().to_string()
                    title=follow_title
                    on:click=move |_| following.update(|value| *value = !*value)
                >
                    "自動スクロール"
                </button>
                <button
                    class="log-tool-button"
                    title="ログを全て消去"
                    on:click=move |_| {
                        log.clear();
                        last_top.set_value(0);
                        following.set(true);
                    }
                >
                    "クリア"
                </button>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_drops_oldest_entries_over_the_limit() {
        let mut buffer = LogBuffer::default();
        for index in 0..MAX_LOG_ENTRIES + 5 {
            buffer.push(LogLevel::Info, String::new(), index.to_string());
        }
        assert_eq!(buffer.entries.len(), MAX_LOG_ENTRIES);
        assert_eq!(buffer.entries.front().unwrap().text, "5");
        assert_eq!(buffer.entries.back().unwrap().id, (MAX_LOG_ENTRIES + 4) as u64);
    }

    #[test]
    fn scrolling_up_stops_and_reaching_bottom_resumes_following() {
        // 高さ100の表示領域に300の内容。最下部はscrollTop=200。
        assert!(!follow_after_scroll(true, 200, 150, 300, 100));
        assert!(follow_after_scroll(false, 150, 199, 300, 100));
        // 最下部への移動中に追記されて届かなかった場合(下向き)は止めない。
        assert!(follow_after_scroll(true, 150, 200, 320, 100));
        // 手動で止めた状態は、下へ動いても最下部に届くまでは止めたまま。
        assert!(!follow_after_scroll(false, 100, 150, 300, 100));
        // クリアで内容が減りscrollTopが0へ切り詰められても止めない。
        assert!(follow_after_scroll(true, 200, 0, 100, 100));
    }

    #[test]
    fn time_is_zero_padded() {
        assert_eq!(format_hms(9, 5, 3), "09:05:03");
    }
}
