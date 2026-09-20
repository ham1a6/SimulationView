#include "ws_server.hpp"

#include <algorithm>
#include <atomic>
#include <cctype>
#include <chrono>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <functional>
#include <iostream>
#include <mutex>
#include <sstream>
#include <string>
#include <string_view>
#include <system_error>
#include <thread>
#include <unordered_map>

// uWebSockets。TLS(HTTPS/WSS)対応のため、SSLありのSSLAppとSSLなしのAppを両方使う
// (LIBUS_USE_OPENSSL は CMake 側で定義する)。
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

// SSL=trueがHTTPS/WSS(uWS::SSLApp)、falseが平文HTTP/WS(uWS::App)。
template <bool SSL>
using ServerWebSocket = uWS::WebSocket<SSL, true, PerSocketData>;

// "bytes=START-END"または"bytes=START-"(単一範囲のみ)を解釈する。解釈できなければfalse。
// endは含む(HTTPのRangeの仕様どおり)。END省略時は末尾まで。
bool parse_byte_range(std::string_view header, std::uintmax_t total, std::uintmax_t& start,
                      std::uintmax_t& end) {
    constexpr std::string_view kPrefix = "bytes=";
    if (header.substr(0, kPrefix.size()) != kPrefix) {
        return false;
    }
    const std::string spec(header.substr(kPrefix.size()));
    const size_t dash = spec.find('-');
    if (dash == std::string::npos || dash == 0 || spec.find(',') != std::string::npos) {
        return false; // "-N"(末尾からN)と複数範囲は使わない。
    }
    try {
        start = std::stoull(spec.substr(0, dash));
        end = (dash + 1 < spec.size()) ? std::stoull(spec.substr(dash + 1)) : total - 1;
    } catch (const std::exception&) {
        return false;
    }
    if (total == 0 || start >= total) {
        return false;
    }
    end = std::min<std::uintmax_t>(end, total - 1);
    return start <= end;
}

// ファイルの大きさと更新時刻から、キャッシュの検証用ETagを作る(ファイルが変われば値も変わる)。
// 更新時刻が取れなければ空文字列を返し、ETagなしで配信する。
std::string make_etag(const std::string& path, std::uintmax_t size) {
    std::error_code ec;
    const auto mtime = std::filesystem::last_write_time(path, ec);
    if (ec) {
        return {};
    }
    char buf[64];
    std::snprintf(buf, sizeof(buf), "\"%llx-%llx\"", static_cast<unsigned long long>(size),
                  static_cast<unsigned long long>(mtime.time_since_epoch().count()));
    return buf;
}

// If-None-Matchの値(カンマ区切りの複数・`W/`付き・`*`)に、現在のETagが含まれるか。
bool etag_matches(std::string_view if_none_match, const std::string& etag) {
    if (if_none_match.empty() || etag.empty()) {
        return false;
    }
    if (if_none_match == "*") {
        return true;
    }
    size_t pos = 0;
    while (pos < if_none_match.size()) {
        size_t end = if_none_match.find(',', pos);
        if (end == std::string_view::npos) {
            end = if_none_match.size();
        }
        std::string_view token = if_none_match.substr(pos, end - pos);
        while (!token.empty() && token.front() == ' ') {
            token.remove_prefix(1);
        }
        while (!token.empty() && token.back() == ' ') {
            token.remove_suffix(1);
        }
        if (token.substr(0, 2) == "W/") {
            token.remove_prefix(2);
        }
        if (token == etag) {
            return true;
        }
        pos = end + 1;
    }
    return false;
}

// 地形データ(metadata.json/tile_index.json/base.bin/tiles/L*/*.bin)の簡易静的ファイル配信。
// 想定CWDは sim_server/ (README.md記載の起動手順に合わせた相対パス)。
// HTTP Range(単一範囲)に対応する: 細かいレベルのタイルファイルは大きい(最細で1タイル約26MB)
// ので、フロントは必要なチャンク1個分だけをRangeで取得する。
//
// キャッシュ: `ETag`(大きさ+更新時刻)と`Cache-Control: no-cache`を付ける。ブラウザは毎回
// `If-None-Match`で問い合わせ、変わっていなければ本体なしの304が返る。2回目以降の表示で数十MBの
// 地形を取り直さずに済み、`map_data/`を作り直したときは(ETagが変わるので)必ず新しいものになる
// (有効期限で古いまま使い続ける事故がない)。
template <bool SSL>
void serve_terrain_file(uWS::HttpResponse<SSL>* res, uWS::HttpRequest* req,
                         const std::string& path, const char* content_type) {
    std::ifstream file(path, std::ios::binary | std::ios::ate);
    if (!file) {
        res->writeStatus("404 Not Found");
        res->writeHeader("Access-Control-Allow-Origin", "*");
        res->end("Not Found");
        return;
    }
    const auto total = static_cast<std::uintmax_t>(file.tellg());

    const std::string etag = make_etag(path, total);
    if (etag_matches(req->getHeader("if-none-match"), etag)) {
        res->writeStatus("304 Not Modified");
        res->writeHeader("ETag", etag);
        res->writeHeader("Cache-Control", "no-cache");
        res->writeHeader("Access-Control-Allow-Origin", "*");
        res->end();
        return;
    }

    std::uintmax_t start = 0;
    std::uintmax_t end = total > 0 ? total - 1 : 0;
    bool partial = false;
    const std::string_view range_header = req->getHeader("range");
    if (!range_header.empty()) {
        partial = parse_byte_range(range_header, total, start, end);
    }

    std::string body(total == 0 ? 0 : static_cast<size_t>(end - start + 1), '\0');
    file.seekg(static_cast<std::streamoff>(start));
    file.read(body.data(), static_cast<std::streamsize>(body.size()));

    if (partial) {
        res->writeStatus("206 Partial Content");
        res->writeHeader("Content-Range", "bytes " + std::to_string(start) + "-" +
                                              std::to_string(end) + "/" + std::to_string(total));
    }
    res->writeHeader("Accept-Ranges", "bytes");
    res->writeHeader("Content-Type", content_type);
    if (!etag.empty()) {
        res->writeHeader("ETag", etag);
        res->writeHeader("Cache-Control", "no-cache"); // 保存はするが、使う前に毎回ETagで確認させる
    }
    res->writeHeader("Access-Control-Allow-Origin", "*");
    res->end(body);
}

// URLのパス要素(タイルのレベル・ファイル名)として安全か: 英数字・'_'・'.'だけで、".."を
// 含まない(パストラバーサルで任意のファイルを読ませないための入力検証)。
bool is_safe_path_component(const std::string& name) {
    if (name.empty() || name.size() > 64 || name.find("..") != std::string::npos) {
        return false;
    }
    return std::all_of(name.begin(), name.end(), [](unsigned char c) {
        return std::isalnum(c) || c == '_' || c == '.';
    });
}

} // namespace

struct WsServer::Impl {
    Impl(uint16_t port_, TlsConfig tls_) : port(port_), tls(std::move(tls_)) {}

    uint16_t port;
    TlsConfig tls;
    Simulation simulation;

    // ClientId -> 送信関数 のマップ。uWSイベントループスレッドからのみ生ポインタ(WebSocket*)を
    // 触ってよいため、参照・変更は必ずclients_mutex経由で行う。SSLの有無(WebSocketの型)を
    // この構造体の外へ漏らさないよう、WebSocket*を捕捉した送信関数として持つ。
    std::unordered_map<ClientId, std::function<void(const std::string&)>> clients;
    std::mutex clients_mutex;
    std::atomic<ClientId> next_client_id{1};

    std::thread sim_thread;
    std::atomic<bool> keep_running{true};

    void broadcast(const std::string& frame) {
        std::lock_guard<std::mutex> lock(clients_mutex);
        for (auto& [id, send] : clients) {
            send(frame);
        }
    }

    void send_to_client(ClientId client_id, const std::string& frame) {
        std::lock_guard<std::mutex> lock(clients_mutex);
        auto it = clients.find(client_id);
        if (it != clients.end()) {
            it->second(frame);
        }
    }

    template <bool SSL>
    void send_to(ServerWebSocket<SSL>* ws, const std::string& frame) {
        ws->send(frame, uWS::OpCode::BINARY);
    }

    template <bool SSL, typename AppT>
    void run_app(AppT app);
};

WsServer::WsServer(uint16_t port, TlsConfig tls) : impl_(new Impl(port, std::move(tls))) {}

WsServer::~WsServer() {
    impl_->keep_running = false;
    if (impl_->sim_thread.joinable()) {
        impl_->sim_thread.join();
    }
    delete impl_;
}

void WsServer::run() {
    if (impl_->tls.enabled()) {
        uWS::SocketContextOptions options{};
        options.key_file_name = impl_->tls.key_file.c_str();
        options.cert_file_name = impl_->tls.cert_file.c_str();
        impl_->run_app<true>(uWS::SSLApp(options));
    } else {
        impl_->run_app<false>(uWS::App());
    }
}

// SSL=trueならHTTPS/WSS、falseなら平文HTTP/WSでサーバーを起動する(呼び出しスレッドをブロックする)。
template <bool SSL, typename AppT>
void WsServer::Impl::run_app(AppT app) {
    Impl* impl = this;

    uWS::Loop* loop = uWS::Loop::get();

    typename AppT::template WebSocketBehavior<PerSocketData> behavior;
    behavior.compression = uWS::DISABLED;
    behavior.maxPayloadLength = 16 * 1024;
    behavior.idleTimeout = 120;

    behavior.open = [impl](ServerWebSocket<SSL>* ws) {
        const ClientId client_id = impl->next_client_id.fetch_add(1);
        ws->getUserData()->client_id = client_id;
        {
            std::lock_guard<std::mutex> lock(impl->clients_mutex);
            impl->clients[client_id] = [ws](const std::string& frame) {
                ws->send(frame, uWS::OpCode::BINARY);
            };
        }
        std::cout << "[ws_server] client connected (id=" << client_id << ")" << std::endl;

        // 接続直後に現在の状態を1回送信する(DETAILED_DESIGN.md 4.2節/4.3節)。
        impl->send_to(ws, protocol::encode_frame(MsgType::OriginState,
                                                   impl->simulation.snapshot_origin()));
        impl->send_to(ws,
                       protocol::encode_frame(MsgType::VabConfig, impl->simulation.vab_config()));
        impl->send_to(ws, protocol::encode_frame(MsgType::StatusPanelConfig,
                                                   impl->simulation.status_panel_config()));
        impl->send_to(ws, protocol::encode_frame(MsgType::AppStatus,
                                                   impl->simulation.snapshot_app_status()));
    };

    behavior.message = [impl](ServerWebSocket<SSL>* ws, std::string_view message, uWS::OpCode) {
        try {
            ClientCommand cmd =
                msgpack::unpack(message.data(), message.size()).get().as<ClientCommand>();
            // DETAILED_DESIGN.md 5.2節: 受信したコマンドは直接シミュレーション状態を書き換えず、
            // スレッドセーフなキューに積む(simスレッド側でstep()時に消費・適用する)。
            impl->simulation.enqueue_command(ws->getUserData()->client_id, std::move(cmd));
        } catch (const std::exception& e) {
            std::cerr << "[ws_server] failed to decode ClientCommand: " << e.what() << std::endl;
        }
    };

    behavior.close = [impl](ServerWebSocket<SSL>* ws, int /*code*/, std::string_view /*message*/) {
        std::lock_guard<std::mutex> lock(impl->clients_mutex);
        impl->clients.erase(ws->getUserData()->client_id);
        std::cout << "[ws_server] client disconnected" << std::endl;
    };

    app.template ws<PerSocketData>("/sim", std::move(behavior));

    // 地形データの静的配信(DETAILED_DESIGN.md 5.4節: WebSocketの/simエンドポイントとは独立したHTTPルート)。
    // フロント(trunk serve)とは別オリジンからfetchされるためCORSヘッダーを付与する。
    app.get("/terrain/metadata.json", [](uWS::HttpResponse<SSL>* res, uWS::HttpRequest* req) {
        serve_terrain_file(res, req, "assets/terrain/metadata.json", "application/json");
    });
    app.get("/terrain/tile_index.json", [](uWS::HttpResponse<SSL>* res, uWS::HttpRequest* req) {
        serve_terrain_file(res, req, "assets/terrain/tile_index.json", "application/json");
    });
    // 全タイルの最粗レベルを連結したもの(起動時にフロントが一度だけ取得する)。
    app.get("/terrain/base.bin", [](uWS::HttpResponse<SSL>* res, uWS::HttpRequest* req) {
        serve_terrain_file(res, req, "assets/terrain/base.bin", "application/octet-stream");
    });
    // 細かいレベルのタイル(例: /terrain/tiles/L2/N035E138.bin)。カメラに近いチャンクだけ
    // フロントが必要に応じて取得する(大きいファイルはHTTP Rangeで部分取得)。
    app.get("/terrain/tiles/:level/:name", [](uWS::HttpResponse<SSL>* res, uWS::HttpRequest* req) {
        const std::string level(req->getParameter(0));
        const std::string name(req->getParameter(1));
        if (!is_safe_path_component(level) || !is_safe_path_component(name)) {
            res->writeStatus("400 Bad Request");
            res->writeHeader("Access-Control-Allow-Origin", "*");
            res->end("Bad Request");
            return;
        }
        serve_terrain_file(res, req, "assets/terrain/tiles/" + level + "/" + name,
                            "application/octet-stream");
    });

    // "0.0.0.0"を明示し、全ネットワークインターフェースでバインドする
    // (LAN上の別端末からもsim_frontendで接続できるようにするため)。
    app.listen("0.0.0.0", impl->port, [impl](us_listen_socket_t* token) {
        if (token) {
            std::cout << "[ws_server] listening on " << (SSL ? "wss" : "ws") << "://0.0.0.0:"
                       << impl->port << "/sim (all interfaces)" << std::endl;
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

            // pause/resumeでアプリ状態が変化していれば新しいAppStatusを全クライアントへ再配信。
            if (tick.app_status_changed) {
                std::string app_status_frame = protocol::encode_frame(
                    MsgType::AppStatus, impl->simulation.snapshot_app_status());
                loop->defer([impl, app_status_frame = std::move(app_status_frame)]() {
                    impl->broadcast(app_status_frame);
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
