mod app;
mod components;
mod log_bridge;
mod protocol;
mod track_bridge;
mod ws;

use app::App;

fn main() {
    console_error_panic_hook::set_once();
    // コンソールへの出力に加え、警告・エラーを画面下部のログパネルへも流す(components/log_panel.rs)。
    let _ = components::log_panel::init_logger(log::Level::Debug);

    leptos::mount::mount_to_body(App);
}
