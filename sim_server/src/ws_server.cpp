#include "ws_server.hpp"

#include <atomic>
#include <chrono>
#include <fstream>
#include <iostream>
#include <mutex>
#include <sstream>
#include <string>
#include <thread>
#include <unordered_map>

// uWebSockets(SSLなし)。LIBUS_NO_SSL は CMake 側で定義する。
#include <App.h>

namespace sim3dview {

using protocol::ClientCommand;
using protocol::MsgType;

namespace {

// WebSocket 1接続あたりの付随データ。
// simスレッドとのやり取りは生の`WebSocket*`ではなくClientId(不透明な整数)で行うため、
// 各接続に割り当てたIDをここに保持しておく。
struct PerSocketData {
    ClientId client_id = 0;
};

using ServerWebSocket = uWS::WebSocket<false, true, PerSocketData>;

// 地形データ(heightmap.bin/metadata.json)の簡易静的ファイル配信。
// 想定CWDは sim_server/ (CLAUDE.md記載の起動手順に合わせた相対パス)。
void serve_terrain_file(uWS::HttpResponse<false>* res, const char* path,
                         const char* content_type) {
    std::ifstream file(path, std::ios::binary);
    if (!file) {
        res->writeStatus("404 Not Found");
        res->writeHeader("Access-Control-Allow-Origin", "*");
        res->end("Not Found");
        return;
    }
    std::ostringstream buffer;
    buffer << file.rdbuf();
    res->writeHeader("Content-Type", content_type);
    res->writeHeader("Access-Control-Allow-Origin", "*");
    res->end(buffer.str());
}

} // namespace

struct WsServer::Impl {
    explicit Impl(uint16_t port_) : port(port_) {}

    uint16_t port;
    Simulation simulation;

    // ClientId -> WebSocket* のマップ。uWSイベントループスレッドからのみ生ポインタを
    // 触ってよいため、参照・変更は必ずclients_mutex経由で行う。
    std::unordered_map<ClientId, ServerWebSocket*> clients;
    std::mutex clients_mutex;
    std::atomic<ClientId> next_client_id{1};

    std::thread sim_thread;
    std::atomic<bool> keep_running{true};

    void broadcast(const std::string& frame) {
        std::lock_guard<std::mutex> lock(clients_mutex);
        for (auto& [id, ws] : clients) {
            ws->send(frame, uWS::OpCode::BINARY);
        }
    }

    void send_to_client(ClientId client_id, const std::string& frame) {
        std::lock_guard<std::mutex> lock(clients_mutex);
        auto it = clients.find(client_id);
        if (it != clients.end()) {
            it->second->send(frame, uWS::OpCode::BINARY);
        }
    }

    void send_to(ServerWebSocket* ws, const std::string& frame) {
        ws->send(frame, uWS::OpCode::BINARY);
    }
};

WsServer::WsServer(uint16_t port) : impl_(new Impl(port)) {}

WsServer::~WsServer() {
    impl_->keep_running = false;
    if (impl_->sim_thread.joinable()) {
        impl_->sim_thread.join();
    }
    delete impl_;
}

void WsServer::run() {
    Impl* impl = impl_;

    // このスレッド(uWSイベントループスレッド)用の Loop を先に確定させておく。
    uWS::Loop* loop = uWS::Loop::get();

    uWS::App::WebSocketBehavior<PerSocketData> behavior;
    behavior.compression = uWS::DISABLED;
    behavior.maxPayloadLength = 16 * 1024;
    behavior.idleTimeout = 120;

    behavior.open = [impl](ServerWebSocket* ws) {
        const ClientId client_id = impl->next_client_id.fetch_add(1);
        ws->getUserData()->client_id = client_id;
        {
            std::lock_guard<std::mutex> lock(impl->clients_mutex);
            impl->clients[client_id] = ws;
        }
        std::cout << "[ws_server] client connected (id=" << client_id << ")" << std::endl;

        // 接続直後に現在の状態を1回送信する(DESIGN.md 4.1節/4.4節)。
        impl->send_to(ws, protocol::encode_frame(MsgType::OriginState,
                                                   impl->simulation.snapshot_origin()));
        impl->send_to(ws,
                       protocol::encode_frame(MsgType::VabConfig, impl->simulation.vab_config()));
        impl->send_to(ws, protocol::encode_frame(MsgType::StatusPanelConfig,
                                                   impl->simulation.status_panel_config()));
    };

    behavior.message = [impl](ServerWebSocket* ws, std::string_view message, uWS::OpCode) {
        try {
            ClientCommand cmd =
                msgpack::unpack(message.data(), message.size()).get().as<ClientCommand>();
            // 指示書: 受信したコマンドは直接シミュレーション状態を書き換えず、
            // スレッドセーフなキューに積む(simスレッド側でstep()時に消費・適用する)。
            impl->simulation.enqueue_command(ws->getUserData()->client_id, std::move(cmd));
        } catch (const std::exception& e) {
            std::cerr << "[ws_server] failed to decode ClientCommand: " << e.what() << std::endl;
        }
    };

    behavior.close = [impl](ServerWebSocket* ws, int /*code*/, std::string_view /*message*/) {
        std::lock_guard<std::mutex> lock(impl->clients_mutex);
        impl->clients.erase(ws->getUserData()->client_id);
        std::cout << "[ws_server] client disconnected" << std::endl;
    };

    uWS::App app;
    app.ws<PerSocketData>("/sim", std::move(behavior));

    // 地形データの静的配信(指示書: WebSocketの/simエンドポイントとは独立したHTTPルート)。
    // フロント(trunk serve)とは別オリジンからfetchされるためCORSヘッダーを付与する。
    app.get("/terrain/metadata.json", [](uWS::HttpResponse<false>* res, uWS::HttpRequest*) {
        serve_terrain_file(res, "assets/terrain/metadata.json", "application/json");
    });
    app.get("/terrain/heightmap.bin", [](uWS::HttpResponse<false>* res, uWS::HttpRequest*) {
        serve_terrain_file(res, "assets/terrain/heightmap.bin", "application/octet-stream");
    });

    app.listen(impl->port, [impl](us_listen_socket_t* token) {
        if (token) {
            std::cout << "[ws_server] listening on ws://localhost:" << impl->port << "/sim"
                       << std::endl;
        } else {
            std::cerr << "[ws_server] failed to listen on port " << impl->port << std::endl;
        }
    });

    // simスレッド: Simulation::step()を約60Hzで進め、結果をuWSスレッドへ
    // Loop::defer()経由で配信させる(別スレッドから直接ws->send()してはならない)。
    impl->sim_thread = std::thread([impl, loop]() {
        using clock = std::chrono::steady_clock;
        const auto frame_interval = std::chrono::milliseconds(16); // 約60Hz
        auto next_tick = clock::now();

        while (impl->keep_running) {
            const double dt = std::chrono::duration<double>(frame_interval).count();
            SimulationTickResult tick = impl->simulation.step(dt);

            // 毎フレームのSimStateは全クライアントへブロードキャスト。
            std::string sim_frame =
                protocol::encode_frame(MsgType::SimState, impl->simulation.snapshot_sim_state());
            loop->defer([impl, sim_frame = std::move(sim_frame)]() { impl->broadcast(sim_frame); });

            // 原点が変化していれば新しいOriginStateを全クライアントへ再配信(4.1節)。
            if (tick.origin_changed) {
                std::string origin_frame = protocol::encode_frame(
                    MsgType::OriginState, impl->simulation.snapshot_origin());
                loop->defer([impl, origin_frame = std::move(origin_frame)]() {
                    impl->broadcast(origin_frame);
                });
            }

            // コマンド拒否は要求元クライアントにのみ返す(4.3節)。
            for (auto& err : tick.errors) {
                std::string err_frame = protocol::encode_frame(MsgType::CommandError, err.error);
                loop->defer(
                    [impl, client_id = err.client_id, err_frame = std::move(err_frame)]() {
                        impl->send_to_client(client_id, err_frame);
                    });
            }

            next_tick += frame_interval;
            std::this_thread::sleep_until(next_tick);
        }
    });

    app.run();
}

} // namespace sim3dview
