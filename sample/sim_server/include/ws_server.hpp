#pragma once

// WebSocketサーバー。DETAILED_DESIGN.md 5.1節: 実シミュレーション(Simulation)との結合。
//
// スレッドモデル(DETAILED_DESIGN.md 5.1節):
// - uWSイベントループスレッドとsimスレッドを分離する。
// - .messageハンドラで受信したコマンドは直接状態を書き換えず、
//   Simulation::enqueue_command()でスレッドセーフなキューに積む。
// - simスレッドがSimulation::step()を進め、その結果(SimState/OriginState変化/CommandError)を
//   uWS::Loop::defer() 経由でuWSイベントループスレッドに送信させる。

#include <cstdint>
#include <string>

#include "simulation.hpp"

namespace sim3dview {

// TLS(HTTPS/WSS)設定。証明書・秘密鍵(PEM)のパスが両方そろったときだけTLSを有効にする。
// 空ならこれまでどおり平文のHTTP/WSで待ち受ける。
struct TlsConfig {
    std::string cert_file;
    std::string key_file;

    bool enabled() const { return !cert_file.empty() && !key_file.empty(); }
};

// uWebSocketsベースのWebSocketサーバー。
class WsServer {
public:
    explicit WsServer(uint16_t port, TlsConfig tls = {});
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
