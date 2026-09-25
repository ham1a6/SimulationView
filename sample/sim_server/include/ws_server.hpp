#pragma once

// HTTP静的配信とWebSocketサーバー。DETAILED_DESIGN.md 5.1節: 実シミュレーション(Simulation)との結合。
//
// スレッドモデル(DETAILED_DESIGN.md 5.1節):
// - HTTP/WebSocketイベントループとsimスレッドを分離する。
// - WebSocket受信コールバックで受けたコマンドは直接状態を書き換えず、
//   Simulation::enqueue_command()でスレッドセーフなキューに積む。
// - simスレッドがSimulation::step()を進め、その結果(SimState/OriginState変化/CommandError)を
//   uWSイベントループ経由でクライアントに送信する。

#include <cstdint>
#include <string>

#include "simulation.hpp"

namespace sim3dview {

// HTTP静的配信とWebSocketを束ねるサーバー。ローカル配布では証明書を使わない。
class WsServer {
public:
    explicit WsServer(uint16_t port, std::string host = "0.0.0.0",
                      std::string terrain_dir = "assets/terrain",
                      std::string upload_dir = "uploads");
    ~WsServer();

    // impl_は所有権を持つ生ポインタでコピー・ムーブ双方が二重解放を招くため禁止する。
    WsServer(const WsServer&) = delete;
    WsServer& operator=(const WsServer&) = delete;
    WsServer(WsServer&&) = delete;
    WsServer& operator=(WsServer&&) = delete;

    // イベントループを起動する(呼び出しスレッドをブロックする)。
    void run();

private:
    struct Impl;
    Impl* impl_;
};

} // namespace sim3dview
