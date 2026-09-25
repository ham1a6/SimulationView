#pragma once

// HTTP静的配信とWebTransportサーバー。DETAILED_DESIGN.md 5.1節: 実シミュレーション(Simulation)との結合。
//
// スレッドモデル(DETAILED_DESIGN.md 5.1節):
// - HTTPイベントループ、WebTransportランタイム、simスレッドを分離する。
// - WebTransport受信コールバックで受けたコマンドは直接状態を書き換えず、
//   Simulation::enqueue_command()でスレッドセーフなキューに積む。
// - simスレッドがSimulation::step()を進め、その結果(SimState/OriginState変化/CommandError)を
//   WebTransportMessaging経由でクライアントに送信する。

#include <cstdint>
#include <string>

#include "simulation.hpp"

namespace sim3dview {

// TLS(HTTPS/HTTP3)設定。WebTransportはTLSを必須とする。
struct TlsConfig {
    std::string cert_file;
    std::string key_file;

    bool enabled() const { return !cert_file.empty() && !key_file.empty(); }
};

// HTTPS静的配信とWebTransportを束ねるサーバー。
class WebTransportServer {
public:
    explicit WebTransportServer(uint16_t port, TlsConfig tls = {},
                      std::string host = "0.0.0.0",
                      std::string terrain_dir = "assets/terrain",
                      std::string upload_dir = "uploads");
    ~WebTransportServer();

    // impl_は所有権を持つ生ポインタでコピー・ムーブ双方が二重解放を招くため禁止する。
    WebTransportServer(const WebTransportServer&) = delete;
    WebTransportServer& operator=(const WebTransportServer&) = delete;
    WebTransportServer(WebTransportServer&&) = delete;
    WebTransportServer& operator=(WebTransportServer&&) = delete;

    // イベントループを起動する(呼び出しスレッドをブロックする)。
    void run();

private:
    struct Impl;
    Impl* impl_;
};

} // namespace sim3dview
