#include "ws_server.hpp"

#include "http_utils.hpp"
#include "upload_file.hpp"
#include "websocket_messaging.hpp"
#include "webtransport_protocol.hpp"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <memory>
#include <mutex>
#include <optional>
#include <stdexcept>
#include <string>
#include <string_view>
#include <thread>

#include <App.h>

namespace sim3dview {
namespace wire = webtransport_protocol;

namespace {

struct PerSocketData { ClientId client_id = 0; };
using ServerWebSocket = uWS::WebSocket<false, true, PerSocketData>;

protocol::ClientCommand to_simulation_command(const wire::ClientCommand& command) {
    protocol::ClientCommand result;
    switch (command.command) {
    case wire::Command::Pause: result.type = "pause"; break;
    case wire::Command::Resume: result.type = "resume"; break;
    case wire::Command::SetParam: result.type = "set_param"; break;
    case wire::Command::SetOrigin: result.type = "set_origin"; break;
    default: result.type = "unsupported"; break;
    }
    result.value = command.value;
    result.lat_deg = command.lat_deg;
    result.lon_deg = command.lon_deg;
    return result;
}

wire::SimState to_wire_state(const protocol::SimState& state) {
    wire::SimState result;
    result.t = state.t;
    if (state.positions.size() >= 3) {
        result.position_x = state.positions[0]; result.position_y = state.positions[1]; result.position_z = state.positions[2];
    }
    result.frame_id = state.frame_id;
    result.elapsed_time_s = state.status_values.empty() ? state.t : state.status_values[0];
    result.altitude_m = state.status_values.size() < 2 ? 0.0 : state.status_values[1];
    result.speed_mps = state.status_values.size() < 3 ? 0.0 : state.status_values[2];
    return result;
}
wire::OriginState to_wire_origin(const protocol::OriginState& state) { return {state.lat_deg, state.lon_deg}; }
wire::AppStatus to_wire_app_status(const protocol::AppStatus& state) { return {static_cast<std::uint8_t>(state.text == "シミュレーション実行中")}; }
wire::TrackList to_wire_tracks(const protocol::TrackList& list) {
    wire::TrackList result;
    result.t = list.t;
    result.count = static_cast<std::uint32_t>(std::min(list.tracks.size(), wire::kMaxTracks));
    for (std::size_t i = 0; i < result.count; ++i) {
        const auto& source = list.tracks[i]; auto& destination = result.tracks[i];
        destination.lat_deg = source.lat_deg; destination.lon_deg = source.lon_deg; destination.alt_m = source.alt_m;
        destination.heading_deg = source.heading_deg; destination.speed_mps = source.speed_mps;
        destination.pitch_deg = source.pitch_deg; destination.roll_deg = source.roll_deg;
        destination.id = source.id; destination.kind = source.kind; destination.affiliation = source.affiliation; destination.alt_ref = source.alt_ref;
        const auto length = std::min(source.label.size(), destination.label.size() - 1);
        std::copy_n(source.label.data(), length, destination.label.data());
    }
    return result;
}
wire::Command to_wire_command(const std::string& type) {
    if (type == "pause") return wire::Command::Pause;
    if (type == "resume") return wire::Command::Resume;
    if (type == "set_origin") return wire::Command::SetOrigin;
    return wire::Command::SetParam;
}
wire::CommandErrorCode to_wire_error_code(const std::string& message) {
    if (message.find("実行中") != std::string::npos) return wire::CommandErrorCode::OriginChangeWhileRunning;
    if (message.find("範囲外") != std::string::npos) return wire::CommandErrorCode::OriginOutsideTerrain;
    return wire::CommandErrorCode::UnsupportedCommand;
}

std::string make_etag(const std::string& path, std::uintmax_t size) {
    std::error_code error;
    const auto modified = std::filesystem::last_write_time(path, error);
    if (error) return {};
    return "\"" + std::to_string(size) + "-" + std::to_string(modified.time_since_epoch().count()) + "\"";
}

void serve_terrain_file(uWS::HttpResponse<false>* response, uWS::HttpRequest* request,
                        const std::string& path, const char* content_type) {
    std::ifstream file(path, std::ios::binary | std::ios::ate);
    if (!file) { response->writeStatus("404 Not Found")->writeHeader("Access-Control-Allow-Origin", "*")->end(); return; }
    const auto total = static_cast<std::uintmax_t>(file.tellg());
    const auto etag = make_etag(path, total);
    if (http_utils::etag_matches(request->getHeader("if-none-match"), etag)) {
        response->writeStatus("304 Not Modified")->writeHeader("ETag", etag)->writeHeader("Cache-Control", "no-cache")->writeHeader("Access-Control-Allow-Origin", "*")->end(); return;
    }
    std::uintmax_t start = 0, end = total == 0 ? 0 : total - 1;
    const bool partial = !request->getHeader("range").empty() && http_utils::parse_byte_range(request->getHeader("range"), total, start, end);
    std::string body(total == 0 ? 0 : static_cast<std::size_t>(end - start + 1), '\0');
    file.seekg(static_cast<std::streamoff>(start)); file.read(body.data(), static_cast<std::streamsize>(body.size()));
    if (partial) response->writeStatus("206 Partial Content")->writeHeader("Content-Range", "bytes " + std::to_string(start) + "-" + std::to_string(end) + "/" + std::to_string(total));
    response->writeHeader("Accept-Ranges", "bytes")->writeHeader("Content-Type", content_type)->writeHeader("Cache-Control", "no-cache")->writeHeader("Access-Control-Allow-Origin", "*");
    if (!etag.empty()) response->writeHeader("ETag", etag);
    response->end(body);
}

} // namespace

struct WsServer::Impl {
    Impl(std::uint16_t port_, std::string host_, std::string terrain_dir_, std::string upload_dir_)
        : port(port_), host(std::move(host_)), terrain_dir(std::move(terrain_dir_)), upload_dir(std::move(upload_dir_)), simulation(terrain_dir + "/metadata.json") {}

    std::uint16_t port;
    std::string host, terrain_dir, upload_dir;
    Simulation simulation;
    WebSocketMessaging messaging;
    std::mutex latest_state_mutex;
    std::optional<wire::SimState> latest_state;
    std::atomic_flag realtime_delivery_scheduled = ATOMIC_FLAG_INIT;
    std::atomic<ClientId> next_client_id{1};
    std::atomic<bool> keep_running{true};
    std::thread sim_thread;

    void queue_latest_state(uWS::Loop* loop, wire::SimState state) {
        { std::lock_guard lock(latest_state_mutex); latest_state = state; }
        if (realtime_delivery_scheduled.test_and_set()) return;
        loop->defer([this] {
            std::optional<wire::SimState> pending;
            { std::lock_guard lock(latest_state_mutex); pending = std::move(latest_state); latest_state.reset(); }
            realtime_delivery_scheduled.clear();
            if (pending) messaging.broadcast_struct(static_cast<wire::MessageId>(wire::Message::SimState), *pending);
        });
    }
};

WsServer::WsServer(std::uint16_t port, std::string host, std::string terrain_dir, std::string upload_dir)
    : impl_(new Impl(port, std::move(host), std::move(terrain_dir), std::move(upload_dir))) {}
WsServer::~WsServer() { impl_->keep_running = false; if (impl_->sim_thread.joinable()) impl_->sim_thread.join(); delete impl_; }

void WsServer::run() {
    auto* impl = impl_;
    impl->messaging.register_handler(static_cast<wire::MessageId>(wire::Message::ClientCommand),
        [impl](WebSocketClientId client_id, const std::byte* data, std::size_t size) {
            if (size != sizeof(wire::ClientCommand)) return;
            wire::ClientCommand command{};
            std::memcpy(&command, data, sizeof(command));
            impl->simulation.enqueue_command(client_id, to_simulation_command(command));
        });
    uWS::App app;
    auto behavior = uWS::TemplatedApp<false>::WebSocketBehavior<PerSocketData>{};
    behavior.compression = uWS::DISABLED; behavior.maxPayloadLength = 16 * 1024; behavior.idleTimeout = 120;
    behavior.open = [impl](ServerWebSocket* socket) {
        const auto client_id = impl->next_client_id.fetch_add(1); socket->getUserData()->client_id = client_id;
        impl->messaging.connect(client_id, [socket](std::string_view bytes) { socket->send(bytes, uWS::OpCode::BINARY); });
        impl->messaging.send_struct(client_id, static_cast<wire::MessageId>(wire::Message::OriginState), to_wire_origin(impl->simulation.snapshot_origin()));
        impl->messaging.send_struct(client_id, static_cast<wire::MessageId>(wire::Message::AppStatus), to_wire_app_status(impl->simulation.snapshot_app_status()));
        impl->messaging.send_struct(client_id, static_cast<wire::MessageId>(wire::Message::TrackList), to_wire_tracks(impl->simulation.snapshot_tracks()));
    };
    behavior.message = [impl](ServerWebSocket* socket, std::string_view message, uWS::OpCode) {
        impl->messaging.receive(socket->getUserData()->client_id, message);
    };
    behavior.close = [impl](ServerWebSocket* socket, int, std::string_view) { impl->messaging.disconnect(socket->getUserData()->client_id); };
    app.ws<PerSocketData>("/sim", std::move(behavior));

    app.options("/uploads", [](auto* response, auto*) { response->writeStatus("204 No Content")->writeHeader("Access-Control-Allow-Origin", "*")->writeHeader("Access-Control-Allow-Methods", "POST")->writeHeader("Access-Control-Allow-Headers", "Content-Type")->end(); });
    app.post("/uploads", [impl](auto* response, auto* request) {
        if (request->getHeader("content-type") != "application/octet-stream") { response->writeStatus("415 Unsupported Media Type")->writeHeader("Access-Control-Allow-Origin", "*")->end(); return; }
        auto pending = std::make_shared<std::unique_ptr<UploadFile>>();
        try { *pending = std::make_unique<UploadFile>(impl->upload_dir); } catch (...) { response->writeStatus("500 Internal Server Error")->writeHeader("Access-Control-Allow-Origin", "*")->end(); return; }
        response->onAborted([pending] { pending->reset(); });
        response->onData([response, pending](std::string_view bytes, bool last) {
            try {
                if (!*pending) return;
                if (!(*pending)->append(bytes)) { pending->reset(); response->writeStatus("413 Payload Too Large")->writeHeader("Access-Control-Allow-Origin", "*")->end(); return; }
                if (last) { const auto id = (*pending)->finish(); pending->reset(); response->writeStatus("201 Created")->writeHeader("Access-Control-Allow-Origin", "*")->writeHeader("Content-Type", "text/plain; charset=utf-8")->end(id); }
            } catch (...) { pending->reset(); response->writeStatus("500 Internal Server Error")->writeHeader("Access-Control-Allow-Origin", "*")->end(); }
        });
    });
    app.get("/terrain/metadata.json", [impl](auto* response, auto* request) { serve_terrain_file(response, request, impl->terrain_dir + "/metadata.json", "application/json"); });
    app.get("/terrain/tile_index.json", [impl](auto* response, auto* request) { serve_terrain_file(response, request, impl->terrain_dir + "/tile_index.json", "application/json"); });
    app.get("/terrain/base.bin", [impl](auto* response, auto* request) { serve_terrain_file(response, request, impl->terrain_dir + "/base.bin", "application/octet-stream"); });
    app.get("/terrain/tiles/:level/:name", [impl](auto* response, auto* request) {
        const std::string level(request->getParameter(0)), name(request->getParameter(1));
        if (!http_utils::is_safe_path_component(level) || !http_utils::is_safe_path_component(name)) { response->writeStatus("400 Bad Request")->writeHeader("Access-Control-Allow-Origin", "*")->end(); return; }
        serve_terrain_file(response, request, impl->terrain_dir + "/tiles/" + level + "/" + name, "application/octet-stream");
    });

    bool listening = false;
    app.listen(impl->host, impl->port, [impl, &listening](us_listen_socket_t* token) {
        if (!token) return;
        const auto actual = us_socket_local_port(0, reinterpret_cast<us_socket_t*>(token));
        if (actual <= 0) return;
        impl->port = static_cast<std::uint16_t>(actual); listening = true;
        std::cout << "SIM3DVIEW_READY " << impl->port << std::endl;
    });
    if (!listening) throw std::runtime_error("サーバーの待ち受けを開始できませんでした");

    auto* loop = uWS::Loop::get();
    impl->sim_thread = std::thread([impl, loop] {
        using clock = std::chrono::steady_clock; const auto interval = std::chrono::milliseconds(16); auto next = clock::now(); std::uint32_t frame_counter = 0;
        while (impl->keep_running) {
            const auto tick = impl->simulation.step(std::chrono::duration<double>(interval).count());
            impl->queue_latest_state(loop, to_wire_state(impl->simulation.snapshot_sim_state()));
            if (tick.time_advanced && (++frame_counter % 3) == 0) { const auto value = to_wire_tracks(impl->simulation.snapshot_tracks()); loop->defer([impl, value] { impl->messaging.broadcast_struct(static_cast<wire::MessageId>(wire::Message::TrackList), value); }); }
            if (tick.origin_changed) { const auto value = to_wire_origin(impl->simulation.snapshot_origin()); loop->defer([impl, value] { impl->messaging.broadcast_struct(static_cast<wire::MessageId>(wire::Message::OriginState), value); }); }
            if (tick.app_status_changed) { const auto value = to_wire_app_status(impl->simulation.snapshot_app_status()); loop->defer([impl, value] { impl->messaging.broadcast_struct(static_cast<wire::MessageId>(wire::Message::AppStatus), value); }); }
            for (const auto& error : tick.errors) { const auto client_id = error.client_id; const wire::CommandError value{to_wire_command(error.error.command_type), to_wire_error_code(error.error.message)}; loop->defer([impl, client_id, value] { impl->messaging.send_struct(client_id, static_cast<wire::MessageId>(wire::Message::CommandError), value); }); }
            next += interval; std::this_thread::sleep_until(next);
        }
    });
    app.run();
}

} // namespace sim3dview
