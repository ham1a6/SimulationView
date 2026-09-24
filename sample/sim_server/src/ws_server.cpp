#include "ws_server.hpp"
#include "http_utils.hpp"
#include "upload_file.hpp"
#include <memory>

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
#include <stdexcept>
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
    return http_utils::parse_byte_range(header, total, start, end);
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
    return http_utils::etag_matches(if_none_match, etag);
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
    return http_utils::is_safe_path_component(name);
}

} // namespace

struct WsServer::Impl {
    Impl(uint16_t port_, TlsConfig tls_, std::string host_, std::string terrain_dir_, std::string upload_dir_)
        : port(port_), tls(std::move(tls_)), host(std::move(host_)),
          terrain_dir(std::move(terrain_dir_)), upload_dir(std::move(upload_dir_)), simulation(terrain_dir + "/metadata.json") {}

    uint16_t port;
    TlsConfig tls;
    std::string host;
    std::string terrain_dir;
    std::string upload_dir;
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

WsServer::WsServer(uint16_t port, TlsConfig tls, std::string host, std::string terrain_dir, std::string upload_dir)
    : impl_(new Impl(port, std::move(tls), std::move(host), std::move(terrain_dir), std::move(upload_dir))) {}

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
        impl->send_to(ws, protocol::encode_frame(MsgType::StatusPanelConfig,
                                                   impl->simulation.status_panel_config()));
        impl->send_to(ws, protocol::encode_frame(MsgType::AppStatus,
                                                   impl->simulation.snapshot_app_status()));
        impl->send_to(ws, protocol::encode_frame(MsgType::TrackList,
                                                   impl->simulation.snapshot_tracks()));
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
    // 生バイトを受信する。通常のHTMLフォームからの書き込みは拒否する。
    app.options("/uploads", [](uWS::HttpResponse<SSL>* res, uWS::HttpRequest*) {
        res->writeStatus("204 No Content")
            ->writeHeader("Access-Control-Allow-Origin", "*")
            ->writeHeader("Access-Control-Allow-Methods", "POST")
            ->writeHeader("Access-Control-Allow-Headers", "Content-Type")->end();
    });
    app.post("/uploads", [impl](uWS::HttpResponse<SSL>* res, uWS::HttpRequest* req) {
        if (req->getHeader("content-type") != "application/octet-stream") {
            res->writeStatus("415 Unsupported Media Type")
                ->writeHeader("Access-Control-Allow-Origin", "*")->end();
            return;
        }
        // 切断・上限超過・I/Oエラー時に受信途中のファイルを破棄する。
        auto pending = std::make_shared<std::unique_ptr<UploadFile>>();
        try {
            *pending = std::make_unique<UploadFile>(impl->upload_dir);
        } catch (const std::exception&) {
            res->writeStatus("500 Internal Server Error")
                ->writeHeader("Access-Control-Allow-Origin", "*")->end();
            return;
        }
        res->onAborted([pending]() { pending->reset(); });
        res->onData([res, pending](std::string_view bytes, bool last) {
            if (!*pending) return;
            try {
                if (!(*pending)->append(bytes)) {
                    pending->reset();
                    res->writeStatus("413 Payload Too Large")
                        ->writeHeader("Access-Control-Allow-Origin", "*")->end();
                    return;
                }
                if (last) {
                    const auto id = (*pending)->finish();
                    pending->reset();
                    res->writeStatus("201 Created")
                        ->writeHeader("Access-Control-Allow-Origin", "*")
                        ->writeHeader("Content-Type", "text/plain; charset=utf-8")->end(id);
                }
            } catch (const std::exception&) {
                pending->reset();
                res->writeStatus("500 Internal Server Error")
                    ->writeHeader("Access-Control-Allow-Origin", "*")->end();
            }
        });
    });

    app.get("/terrain/metadata.json", [impl](uWS::HttpResponse<SSL>* res, uWS::HttpRequest* req) {
        serve_terrain_file(res, req, impl->terrain_dir + "/metadata.json", "application/json");
    });
    app.get("/terrain/tile_index.json", [impl](uWS::HttpResponse<SSL>* res, uWS::HttpRequest* req) {
        serve_terrain_file(res, req, impl->terrain_dir + "/tile_index.json", "application/json");
    });
    // 全タイルの最粗レベルを連結したもの(起動時にフロントが一度だけ取得する)。
    app.get("/terrain/base.bin", [impl](uWS::HttpResponse<SSL>* res, uWS::HttpRequest* req) {
        serve_terrain_file(res, req, impl->terrain_dir + "/base.bin", "application/octet-stream");
    });
    // 細かいレベルのタイル(例: /terrain/tiles/L2/N035E138.bin)。カメラに近いチャンクだけ
    // フロントが必要に応じて取得する(大きいファイルはHTTP Rangeで部分取得)。
    app.get("/terrain/tiles/:level/:name", [impl](uWS::HttpResponse<SSL>* res, uWS::HttpRequest* req) {
        const std::string level(req->getParameter(0));
        const std::string name(req->getParameter(1));
        if (!is_safe_path_component(level) || !is_safe_path_component(name)) {
            res->writeStatus("400 Bad Request");
            res->writeHeader("Access-Control-Allow-Origin", "*");
            res->end("Bad Request");
            return;
        }
        serve_terrain_file(res, req, impl->terrain_dir + "/tiles/" + level + "/" + name,
                            "application/octet-stream");
    });

    // 通常はLAN公開、Electronは127.0.0.1とポート0を指定する(設計書7.9節)。
    bool listening = false;
    app.listen(impl->host, impl->port, [impl, &listening](us_listen_socket_t* token) {
        if (token) {
            const int actual_port = us_socket_local_port(0, reinterpret_cast<us_socket_t*>(token));
            if (actual_port <= 0) {
                us_listen_socket_close(SSL, token);
                return;
            }
            impl->port = static_cast<uint16_t>(actual_port);
            listening = true;
            std::cout << "[ws_server] listening on " << (SSL ? "wss" : "ws") << "://"
                      << impl->host << ":" << impl->port << "/sim" << std::endl;
            // Electronは自身が起動したプロセスの標準出力で起動完了を判定する。
            std::cout << "SIM3DVIEW_READY " << impl->port << std::endl;
        } else {
            std::cerr << "[ws_server] failed to listen on port " << impl->port << std::endl;
        }
    });
    if (!listening) {
        throw std::runtime_error("サーバーの待ち受けを開始できませんでした");
    }

    // simスレッド: Simulation::step()を約60Hzで進め、結果をuWSスレッドへ
    // Loop::defer()経由で配信させる(別スレッドから直接ws->send()してはならない)。
    impl->sim_thread = std::thread([impl, loop]() {
        using clock = std::chrono::steady_clock;
        const auto frame_interval = std::chrono::milliseconds(16); // 約60Hz
        auto next_tick = clock::now();
        // TrackListはSimState(60Hz)より低い頻度で配信する(トラックは数が多くなりうるので、
        // 表示に十分な約20Hz)。
        constexpr uint32_t kTrackListEveryNFrames = 3;
        uint32_t frame_counter = 0;

        while (impl->keep_running) {
            const double dt = std::chrono::duration<double>(frame_interval).count();
            SimulationTickResult tick = impl->simulation.step(dt);

            // 毎フレームのSimStateは全クライアントへブロードキャスト。
            std::string sim_frame =
                protocol::encode_frame(MsgType::SimState, impl->simulation.snapshot_sim_state());
            loop->defer([impl, sim_frame = std::move(sim_frame)]() { impl->broadcast(sim_frame); });

            // シミュレーション時刻が進んでいる間、一定間隔で航跡(TrackList)を全クライアントへ配信する。
            // 一時停止中は位置が変わらないので送らない(新規接続には接続直後に1回送る)。
            if (tick.time_advanced && (++frame_counter % kTrackListEveryNFrames) == 0) {
                std::string track_frame = protocol::encode_frame(
                    MsgType::TrackList, impl->simulation.snapshot_tracks());
                loop->defer([impl, track_frame = std::move(track_frame)]() {
                    impl->broadcast(track_frame);
                });
            }

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
