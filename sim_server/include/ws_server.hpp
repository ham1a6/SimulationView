#pragma once

// WebSocketサーバー。DESIGN.md 9節フェーズ5: 実シミュレーション(Simulation)との結合。
//
// スレッドモデル(指示書 C++側実装要件):
// - uWSイベントループスレッドとsimスレッドを分離する。
// - .messageハンドラで受信したコマンドは直接状態を書き換えず、
//   Simulation::enqueue_command()でスレッドセーフなキューに積む。
// - simスレッドがSimulation::step()を進め、その結果(SimState/OriginState変化/CommandError)を
//   uWS::Loop::defer() 経由でuWSイベントループスレッドに送信させる。

#include <cstdint>

#include "simulation.hpp"

namespace sim3dview {

// uWebSocketsベースのWebSocketサーバー。
class WsServer {
public:
    explicit WsServer(uint16_t port);
    ~WsServer();

    // イベントループを起動する(呼び出しスレッドをブロックする)。
    void run();

private:
    struct Impl;
    Impl* impl_;
};

} // namespace sim3dview
