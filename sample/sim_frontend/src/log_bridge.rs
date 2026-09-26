//! アプリの出来事をログパネル(`components/log_panel.rs`、DETAILED_DESIGN.md 7.10節)へ書き出す。
//! 受信データのシグナル(`WsSignals`)を購読し、接続状態の変化・シミュレーション状態の変化・
//! 原点の変化・コマンドの拒否を1行ずつ残す。

use leptos::prelude::*;

use crate::components::log_panel::LogState;
use crate::ws::{ConnectionStatus, WsSignals};

pub fn bridge_logs(signals: WsSignals, log: LogState) {
    // 再接続の各試行は「再接続試行中→接続中」と続くので、接続中は最初の1回だけ残す。
    Effect::new(move |previous: Option<ConnectionStatus>| {
        let status = signals.status.get();
        if previous != Some(status) {
            match status {
                ConnectionStatus::Connecting if previous.is_some() => {}
                ConnectionStatus::Reconnecting { .. } => log.warn(format!("サーバー: {status}")),
                _ => log.info(format!("サーバー: {status}")),
            }
        }
        status
    });

    // AppStatus・OriginStateは再接続のたびに同じ値が再送されるので、変わったときだけ残す。
    Effect::new(move |previous: Option<Option<String>>| {
        let text = signals.app_status.with(|status| status.as_ref().map(|s| s.text.clone()));
        if let Some(current) = &text {
            if previous.flatten().as_ref() != Some(current) {
                log.info(current.clone());
            }
        }
        text
    });

    Effect::new(move |previous: Option<Option<(f64, f64)>>| {
        let origin = signals.origin.get().map(|o| (o.lat_deg, o.lon_deg));
        if let Some((lat, lon)) = origin {
            if previous.flatten() != origin {
                log.info(format!("原点: {lat:.6}, {lon:.6}"));
            }
        }
        origin
    });

    Effect::new(move |_| {
        if let Some(error) = signals.last_command_error.get() {
            log.error(format!("[{}] {}", error.command_type, error.message));
        }
    });
}
