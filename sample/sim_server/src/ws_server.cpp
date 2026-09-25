#include "ws_server.hpp"
#include "http_utils.hpp"
#include "upload_file.hpp"
#include "webtransport_messaging.hpp"
#include "webtransport_protocol.hpp"
#include <memory>

#include <algorithm>
#include <atomic>
#include <cctype>
#include <chrono>
#include <cstring>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <sstream>
#include <stdexcept>
#include <string>
#include <string_view>
#include <system_error>
#include <thread>

// uWebSocketsはHTTPSの静的配信にだけ用いる。WebTransportはRustランタイムがHTTP/3で処理する。
// (LIBUS_USE_OPENSSL は CMake 側で定義する)。
#include <App.h>
#include <openssl/pem.h>
#include <openssl/x509.h>

namespace sim3dview {

namespace wt = webtransport_protocol;

namespace {

protocol::ClientCommand to_simulation_command(const wt::ClientCommand& command) {
    protocol::ClientCommand result;
    switch (command.command) {
    case wt::Command::Pause: result.type = "pause"; break;
    case wt::Command::Resume: result.type = "resume"; break;
    case wt::Command::SetParam: result.type = "set_param"; break;
    case wt::Command::SetOrigin: result.type = "set_origin"; break;
    default: result.type = "unsupported"; break;
    }
    result.value = command.value;
    result.lat_deg = command.lat_deg;
    result.lon_deg = command.lon_deg;
    return result;
}

wt::SimState to_wire_state(const protocol::SimState& state) {
    wt::SimState result;
    result.t = state.t;
    if (state.positions.size() >= 3) {
        result.position_x = state.positions[0];
        result.position_y = state.positions[1];
        result.position_z = state.positions[2];
    }
    result.frame_id = state.frame_id;
    result.elapsed_time_s = state.status_values.empty() ? state.t : state.status_values[0];
    result.altitude_m = state.status_values.size() < 2 ? 0.0 : state.status_values[1];
    result.speed_mps = state.status_values.size() < 3 ? 0.0 : state.status_values[2];
    return result;
}

wt::OriginState to_wire_origin(const protocol::OriginState& state) {
    return {state.lat_deg, state.lon_deg};
}

wt::AppStatus to_wire_app_status(const protocol::AppStatus& state) {
    return {static_cast<std::uint8_t>(state.text == "シミュレーション実行中")};
}

wt::TrackList to_wire_tracks(const protocol::TrackList& list) {
    wt::TrackList result;
    result.t = list.t;
    result.count = static_cast<std::uint32_t>(std::min(list.tracks.size(), wt::kMaxTracks));
    for (std::size_t i = 0; i < result.count; ++i) {
        const auto& source = list.tracks[i];
        auto& destination = result.tracks[i];
        destination.lat_deg = source.lat_deg;
        destination.lon_deg = source.lon_deg;
        destination.alt_m = source.alt_m;
        destination.heading_deg = source.heading_deg;
        destination.speed_mps = source.speed_mps;
        destination.pitch_deg = source.pitch_deg;
        destination.roll_deg = source.roll_deg;
        destination.id = source.id;
        destination.kind = source.kind;
        destination.affiliation = source.affiliation;
        destination.alt_ref = source.alt_ref;
        const auto label_size = std::min(source.label.size(), destination.label.size() - 1);
        std::copy_n(source.label.data(), label_size, destination.label.data());
    }
    return result;
}

wt::Command to_wire_command(const std::string& type) {
    if (type == "set_origin") return wt::Command::SetOrigin;
    if (type == "pause") return wt::Command::Pause;
    if (type == "resume") return wt::Command::Resume;
    if (type == "set_param") return wt::Command::SetParam;
    return wt::Command::SetParam;
}

wt::CommandErrorCode to_wire_error_code(const std::string& message) {
    if (message.find("実行中") != std::string::npos) return wt::CommandErrorCode::OriginChangeWhileRunning;
    if (message.find("範囲外") != std::string::npos) return wt::CommandErrorCode::OriginOutsideTerrain;
    return wt::CommandErrorCode::UnsupportedCommand;
}

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

struct WebTransportServer::Impl {
    Impl(uint16_t port_, TlsConfig tls_, std::string host_, std::string terrain_dir_, std::string upload_dir_)
        : port(port_), tls(std::move(tls_)), host(std::move(host_)),
          terrain_dir(std::move(terrain_dir_)), upload_dir(std::move(upload_dir_)), simulation(terrain_dir + "/metadata.json") {}

    uint16_t port;
    TlsConfig tls;
    std::string host;
    std::string terrain_dir;
    std::string upload_dir;
    Simulation simulation;
    WebTransportMessaging transport;

    std::thread sim_thread;
    std::atomic<bool> keep_running{true};

    template <bool SSL, typename AppT>
    void run_app(AppT app);
};

WebTransportServer::WebTransportServer(uint16_t port, TlsConfig tls, std::string host, std::string terrain_dir, std::string upload_dir)
    : impl_(new Impl(port, std::move(tls), std::move(host), std::move(terrain_dir), std::move(upload_dir))) {}

WebTransportServer::~WebTransportServer() {
    impl_->keep_running = false;
    if (impl_->sim_thread.joinable()) {
        impl_->sim_thread.join();
    }
    delete impl_;
}

void WebTransportServer::run() {
    // WebTransport は HTTP/3/QUIC と TLS 1.3 を前提とする。従来の平文WS互換モードは持たない。
    if (!impl_->tls.enabled()) {
        throw std::runtime_error("WebTransportには --cert と --key の指定が必要です");
    }
    uWS::SocketContextOptions options{};
    options.key_file_name = impl_->tls.key_file.c_str();
    options.cert_file_name = impl_->tls.cert_file.c_str();
    impl_->run_app<true>(uWS::SSLApp(options));
}

// ChromiumのWebTransport用 `serverCertificateHashes` に渡す、葉証明書DERのSHA-256を
// URL-safe Base64で返す。通常の公開CA証明書では不要だが、Electron同梱の開発用自己署名証明書で
// ループバック接続を安全にピン留めするために使う。
std::string certificate_sha256_base64url(const std::string& cert_file) {
    BIO* bio = BIO_new_file(cert_file.c_str(), "r");
    if (!bio) return {};
    X509* cert = PEM_read_bio_X509(bio, nullptr, nullptr, nullptr);
    BIO_free(bio);
    if (!cert) return {};
    std::array<unsigned char, EVP_MAX_MD_SIZE> digest{};
    unsigned int digest_size = 0;
    const bool ok = X509_digest(cert, EVP_sha256(), digest.data(), &digest_size) == 1;
    X509_free(cert);
    if (!ok) return {};
    std::array<unsigned char, 64> encoded{};
    const int encoded_size = EVP_EncodeBlock(encoded.data(), digest.data(), digest_size);
    if (encoded_size <= 0) return {};
    std::string result(reinterpret_cast<const char*>(encoded.data()), encoded_size);
    std::replace(result.begin(), result.end(), '+', '-');
    std::replace(result.begin(), result.end(), '/', '_');
    while (!result.empty() && result.back() == '=') result.pop_back();
    return result;
}

// HTTPS静的配信を起動する(呼び出しスレッドをブロックする)。
template <bool SSL, typename AppT>
void WebTransportServer::Impl::run_app(AppT app) {
    Impl* impl = this;

    // 地形データの静的配信。WebTransportの /sim は同じポート番号の UDP/HTTP3 側で受ける。
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
    std::string webtransport_error;
    app.listen(impl->host, impl->port, [impl, &listening](us_listen_socket_t* token) {
        if (token) {
            const int actual_port = us_socket_local_port(0, reinterpret_cast<us_socket_t*>(token));
            if (actual_port <= 0) {
                us_listen_socket_close(SSL, token);
                return;
            }
            impl->port = static_cast<uint16_t>(actual_port);
            listening = true;
            std::cout << "[sim_server] HTTPS static server listening on https://" << impl->host
                      << ":" << impl->port << std::endl;
        } else {
            std::cerr << "[ws_server] failed to listen on port " << impl->port << std::endl;
        }
    });
    if (!listening) {
        throw std::runtime_error("サーバーの待ち受けを開始できませんでした");
    }

    // TCPの実ポートが決まってから、同じ番号のUDPポートでHTTP/3 WebTransportを開始する。
    // TCP/UDPは別のトランスポートなので、同一ポート番号を安全に共有できる。
    impl->transport.register_handler(static_cast<wt::MessageId>(wt::Message::ClientCommand),
                                     [impl](TransportClientId client_id, const std::byte* data,
                                            std::size_t size) {
        if (size != sizeof(wt::ClientCommand)) return;
        wt::ClientCommand command{};
        std::memcpy(&command, data, sizeof(command));
        impl->simulation.enqueue_command(client_id, to_simulation_command(command));
    });
    impl->transport.set_connection_handlers(
        [impl](TransportClientId client_id) {
            std::cout << "[webtransport] client connected (id=" << client_id << ")" << std::endl;
            impl->transport.send_struct(client_id, static_cast<wt::MessageId>(wt::Message::OriginState),
                                        DeliveryMode::ReliableStream,
                                        to_wire_origin(impl->simulation.snapshot_origin()));
            impl->transport.send_struct(client_id, static_cast<wt::MessageId>(wt::Message::AppStatus),
                                        DeliveryMode::ReliableStream,
                                        to_wire_app_status(impl->simulation.snapshot_app_status()));
            impl->transport.send_struct(client_id, static_cast<wt::MessageId>(wt::Message::TrackList),
                                        DeliveryMode::ReliableStream,
                                        to_wire_tracks(impl->simulation.snapshot_tracks()));
        },
        [](TransportClientId client_id) {
            std::cout << "[webtransport] client disconnected (id=" << client_id << ")" << std::endl;
        });
    if (!impl->transport.start(impl->port, impl->tls.cert_file, impl->tls.key_file,
                               webtransport_error)) {
        throw std::runtime_error("WebTransportの開始に失敗: " + webtransport_error);
    }
    // Electronは自身が起動したプロセスの標準出力で起動完了を判定する。
    std::cout << "SIM3DVIEW_READY " << impl->port << " "
              << certificate_sha256_base64url(impl->tls.cert_file) << std::endl;

    // simスレッド: Simulation::step()を約60Hzで進める。WebTransportMessagingは送信を
    // HTTP/3ランタイムへ非同期で委譲するので、uWS::Loop::deferは不要である。
    impl->sim_thread = std::thread([impl]() {
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

            // 60Hzの状態だけは最新値で上書きできればよいので、到達保証なしのデータグラムにする。
            impl->transport.broadcast_struct(static_cast<wt::MessageId>(wt::Message::SimState),
                                              DeliveryMode::UnreliableDatagram,
                                              to_wire_state(impl->simulation.snapshot_sim_state()));

            // シミュレーション時刻が進んでいる間、一定間隔で航跡(TrackList)を全クライアントへ配信する。
            // 一時停止中は位置が変わらないので送らない(新規接続には接続直後に1回送る)。
            if (tick.time_advanced && (++frame_counter % kTrackListEveryNFrames) == 0) {
                impl->transport.broadcast_struct(static_cast<wt::MessageId>(wt::Message::TrackList),
                                                  DeliveryMode::ReliableStream,
                                                  to_wire_tracks(impl->simulation.snapshot_tracks()));
            }

            // 原点が変化していれば新しいOriginStateを全クライアントへ再配信(4.1節)。
            if (tick.origin_changed) {
                impl->transport.broadcast_struct(static_cast<wt::MessageId>(wt::Message::OriginState),
                                                  DeliveryMode::ReliableStream,
                                                  to_wire_origin(impl->simulation.snapshot_origin()));
            }

            // pause/resumeでアプリ状態が変化していれば新しいAppStatusを全クライアントへ再配信。
            if (tick.app_status_changed) {
                impl->transport.broadcast_struct(static_cast<wt::MessageId>(wt::Message::AppStatus),
                                                  DeliveryMode::ReliableStream,
                                                  to_wire_app_status(impl->simulation.snapshot_app_status()));
            }

            // コマンド拒否は要求元クライアントにのみ返す(4.3節)。
            for (auto& err : tick.errors) {
                const wt::CommandError error{to_wire_command(err.error.command_type),
                                             to_wire_error_code(err.error.message)};
                impl->transport.send_struct(err.client_id,
                                            static_cast<wt::MessageId>(wt::Message::CommandError),
                                            DeliveryMode::ReliableStream, error);
            }

            next_tick += frame_interval;
            std::this_thread::sleep_until(next_tick);
        }
    });

    app.run();
}

} // namespace sim3dview
