//! 画面最下部のログパネル(DETAILED_DESIGN.md 7.10節)。画面幅いっぱいで、3行分の高さに
//! 時刻・レベル・本文を1件ずつ並べる。
//!
//! 追記は`LogState`(contextで共有)へ行う。アプリの出来事(接続状態・コマンド拒否など)は
//! `log_bridge.rs`が、`log`クレートの警告・エラー(ライブラリのものを含む)は`init_logger`で
//! 差し込んだロガーがここへ流す。
//!
//! スクロール: 最下部を表示している間だけ、追記のたびに最下部へ移動する(自動スクロール)。
//! ユーザーが上へスクロールすると位置を保ち、最下部まで戻すと再開する。1件が改行・折り返しで
//! 何行になっても追従できるよう、行数ではなく内容の実寸(ResizeObserver)の変化で最下部へ合わせる。

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
        let text = normalize_text(text.into());
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

/// 改行をLFへ揃え、末尾の改行・空白を落とす(末尾の改行で空行が出ないように)。
/// 途中の改行はそのまま残し、`.log-text`の`white-space: pre-wrap`で複数行として表示する。
fn normalize_text(text: String) -> String {
    let text = if text.contains('\r') { text.replace("\r\n", "\n").replace('\r', "\n") } else { text };
    match text.trim_end() {
        trimmed if trimmed.len() == text.len() => text,
        trimmed => trimmed.to_string(),
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

/// スクロール後に、最下部へ張り付いている(自動スクロールする)かを決める。
///
/// 現在の内容の最下部、または前回レイアウトを見たときの内容の高さ(`laid_out_height`)での最下部なら
/// 張り付く。スクロールイベントは次のフレームでまとめて届くため、その前に行が追記されて内容が
/// 伸びていると、ユーザーが最下部まで動かしていても現在の高さでは最下部から外れて見えるので、
/// 追記前の高さでも判定する。
/// 最下部でなければ、上へ動いたときだけ外す(下へ動いたが最下部に届かないときは今の状態のまま)。
/// 内容が減ってscrollTopが切り詰められた場合は最下部になるので、先に最下部を判定して外さない。
fn pinned_after_scroll(
    pinned: bool,
    previous_top: i32,
    laid_out_height: i32,
    top: i32,
    scroll_height: i32,
    client_height: i32,
) -> bool {
    let at_bottom_of = |height: i32| height - top - client_height <= BOTTOM_TOLERANCE_PX;
    if at_bottom_of(scroll_height) || at_bottom_of(laid_out_height) {
        true
    } else if top < previous_top {
        false
    } else {
        pinned
    }
}

#[component]
pub fn LogPanel() -> impl IntoView {
    use wasm_bindgen::{closure::Closure, JsCast};

    let log = use_context::<LogState>().expect("LogState context not found");
    let list = NodeRef::<leptos::html::Div>::new();
    let content = NodeRef::<leptos::html::Div>::new();
    // 最下部を表示しているか(=自動スクロールするか)。スクロールイベントで更新する。
    let pinned = RwSignal::new(true);
    // 直前に見たscrollTop(ユーザーのスクロールか、こちらが最下部へ移動した位置)。
    let last_top = StoredValue::new(0_i32);
    // 前回レイアウトを見たときの内容の高さ(`pinned_after_scroll`参照)。
    let laid_out_height = StoredValue::new(0_i32);

    // 内容の高さを記録し、張り付いていれば最下部へ移動する。移動先は`last_top`にも記録する。
    // スクロールイベントは1フレームに1回へまとめられるので、記録しないと、この移動と同じフレームで
    // ユーザーが少し上へ戻した操作が、前回の位置より下にあるせいで「上へ動いた」と判定されず、
    // 最下部へ引き戻されてしまう。
    let follow_content = move || {
        let Some(element) = list.get_untracked() else { return };
        if pinned.get_untracked() {
            element.set_scroll_top(element.scroll_height());
            last_top.set_value(element.scroll_top());
        }
        laid_out_height.set_value(element.scroll_height());
    };

    // 内容・表示領域の実寸が変わるたび(追記・複数行の本文・幅の変化による折り返し・クリア)に、
    // 描画前に最下部へ合わせる。行数を数えないので、1件が何行になっても最下部に届く。
    Effect::new(move |_| {
        let (Some(list_element), Some(content_element)) = (list.get(), content.get()) else {
            return;
        };
        let callback = Closure::<dyn FnMut(js_sys::Array)>::new(move |_| follow_content());
        let observer = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref())
            .expect("ログパネルのサイズ監視を開始できません");
        observer.observe(&list_element);
        observer.observe(&content_element);
        let resources = StoredValue::new_local((observer, callback));
        on_cleanup(move || resources.with_value(|(observer, _)| observer.disconnect()));
    });

    // 件数が上限に達すると、先頭の1行を捨てて1行足すため内容の高さが変わらないことがある。
    // そのときはResizeObserverが通知しないので、追記のたびにも次の描画の前に合わせる。
    Effect::new(move |_| {
        log.buffer.track();
        request_animation_frame(follow_content);
    });

    let on_scroll = move |_| {
        let Some(element) = list.get_untracked() else { return };
        let top = element.scroll_top();
        let next = pinned_after_scroll(
            pinned.get_untracked(),
            last_top.get_value(),
            laid_out_height.get_value(),
            top,
            element.scroll_height(),
            element.client_height(),
        );
        last_top.set_value(top);
        if next != pinned.get_untracked() {
            pinned.set(next);
        }
    };

    let entries = move || log.buffer.with(|buffer| buffer.entries.iter().cloned().collect::<Vec<_>>());

    view! {
        <div class="log-panel">
            <div class="log-list" node_ref=list on:scroll=on_scroll role="log" aria-label="ログ">
                <div node_ref=content>
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
            </div>
            <div class="log-tools">
                <button
                    class="log-tool-button"
                    title="最新のログまでスクロールし、自動スクロールを再開する"
                    disabled=move || pinned.get()
                    on:click=move |_| {
                        pinned.set(true);
                        follow_content();
                    }
                >
                    "最新へ"
                </button>
                <button
                    class="log-tool-button"
                    title="ログを全て消去"
                    on:click=move |_| {
                        log.clear();
                        last_top.set_value(0);
                        pinned.set(true);
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
    fn scrolling_up_unpins_and_reaching_bottom_pins() {
        // 高さ100の表示領域に300の内容。最下部はscrollTop=200。
        assert!(!pinned_after_scroll(true, 200, 300, 150, 300, 100));
        assert!(pinned_after_scroll(false, 150, 300, 199, 300, 100));
        // 上へ戻した状態は、下へ動いても最下部に届くまでは外れたまま。
        assert!(!pinned_after_scroll(false, 100, 300, 150, 300, 100));
        // 最下部まで動かした後、イベントが届く前に複数行の本文が追記されて内容が360へ伸びても張り付く。
        assert!(pinned_after_scroll(false, 150, 300, 200, 360, 100));
        // こちらが最下部へ移動した後、同じフレームで少しだけ上へ戻されたら外す。
        assert!(!pinned_after_scroll(true, 260, 360, 240, 360, 100));
        // クリアや折り返しの解消で内容が減りscrollTopが切り詰められても外さない。
        assert!(pinned_after_scroll(true, 200, 300, 0, 100, 100));
        assert!(pinned_after_scroll(true, 200, 300, 120, 220, 100));
    }

    #[test]
    fn text_keeps_inner_newlines_and_drops_trailing_ones() {
        assert_eq!(normalize_text("a\r\nb\rc\n\n".into()), "a\nb\nc");
        assert_eq!(normalize_text("一行".into()), "一行");
    }

    #[test]
    fn time_is_zero_padded() {
        assert_eq!(format_hms(9, 5, 3), "09:05:03");
    }
}
