#include "webtransport_messaging.hpp"

#include <cstring>
#include <limits>

// RustのHTTP/3/WebTransport実装とのC ABI。C++から見える公開APIは
// WebTransportMessagingだけであり、Rustの非同期ランタイム詳細を漏らさない。
extern "C" {
struct sim3dview_wt_callbacks {
    void (*connected)(void*, std::uint64_t);
    void (*disconnected)(void*, std::uint64_t);
    void (*received)(void*, std::uint64_t, std::uint8_t, const std::uint8_t*, std::size_t);
};

int sim3dview_wt_start(std::uint16_t port, const char* cert_file, const char* key_file,
                        void* context, sim3dview_wt_callbacks callbacks, char* error,
                        std::size_t error_size);
int sim3dview_wt_send(std::uint64_t client_id, std::uint8_t delivery, const std::uint8_t* data,
                      std::size_t size);
int sim3dview_wt_broadcast(std::uint8_t delivery, const std::uint8_t* data, std::size_t size);
}

namespace sim3dview {

WebTransportMessaging::WebTransportMessaging() = default;
WebTransportMessaging::~WebTransportMessaging() = default;

bool WebTransportMessaging::start(std::uint16_t port, const std::string& cert_file,
                                   const std::string& key_file, std::string& error) {
    if (cert_file.empty() || key_file.empty()) {
        error = "WebTransportにはTLS証明書と秘密鍵が必要です";
        return false;
    }
    std::array<char, 512> error_buffer{};
    const sim3dview_wt_callbacks callbacks{&on_connected, &on_disconnected, &on_received};
    if (sim3dview_wt_start(port, cert_file.c_str(), key_file.c_str(), this, callbacks,
                           error_buffer.data(), error_buffer.size()) != 0) {
        error = error_buffer.data();
        return false;
    }
    std::lock_guard lock(mutex_);
    started_ = true;
    return true;
}

void WebTransportMessaging::set_connection_handlers(ConnectionHandler on_connect,
                                                     ConnectionHandler on_disconnect) {
    std::lock_guard lock(mutex_);
    on_connect_ = std::move(on_connect);
    on_disconnect_ = std::move(on_disconnect);
}

void WebTransportMessaging::register_handler(webtransport_protocol::MessageId message_id,
                                              ReceiveHandler handler) {
    std::lock_guard lock(mutex_);
    handlers_[message_id] = std::move(handler);
}

bool WebTransportMessaging::send(TransportClientId client_id,
                                  webtransport_protocol::MessageId message_id,
                                  DeliveryMode delivery, const void* data, std::size_t size) {
    const auto frame = make_frame(message_id, data, size);
    return !frame.empty() &&
           sim3dview_wt_send(client_id, static_cast<std::uint8_t>(delivery), frame.data(),
                              frame.size()) == 0;
}

bool WebTransportMessaging::broadcast(webtransport_protocol::MessageId message_id,
                                       DeliveryMode delivery, const void* data, std::size_t size) {
    const auto frame = make_frame(message_id, data, size);
    return !frame.empty() &&
           sim3dview_wt_broadcast(static_cast<std::uint8_t>(delivery), frame.data(), frame.size()) == 0;
}

void WebTransportMessaging::on_connected(void* context, TransportClientId client_id) {
    auto* self = static_cast<WebTransportMessaging*>(context);
    ConnectionHandler handler;
    {
        std::lock_guard lock(self->mutex_);
        handler = self->on_connect_;
    }
    if (handler) handler(client_id);
}

void WebTransportMessaging::on_disconnected(void* context, TransportClientId client_id) {
    auto* self = static_cast<WebTransportMessaging*>(context);
    ConnectionHandler handler;
    {
        std::lock_guard lock(self->mutex_);
        handler = self->on_disconnect_;
    }
    if (handler) handler(client_id);
}

void WebTransportMessaging::on_received(void* context, TransportClientId client_id,
                                         std::uint8_t delivery, const std::uint8_t* data,
                                         std::size_t size) {
    auto* self = static_cast<WebTransportMessaging*>(context);
    if (delivery > static_cast<std::uint8_t>(DeliveryMode::UnreliableDatagram)) return;
    self->dispatch(client_id, static_cast<DeliveryMode>(delivery), data, size);
}

void WebTransportMessaging::dispatch(TransportClientId client_id, DeliveryMode,
                                     const std::uint8_t* data, std::size_t size) {
    using webtransport_protocol::FrameHeader;
    if (data == nullptr || size < sizeof(FrameHeader)) return;
    FrameHeader header{};
    std::memcpy(&header, data, sizeof(header));
    if (header.reserved != 0 || header.payload_size != size - sizeof(header)) return;

    ReceiveHandler handler;
    {
        std::lock_guard lock(mutex_);
        const auto it = handlers_.find(header.message_id);
        if (it == handlers_.end()) return;
        handler = it->second;
    }
    handler(client_id, reinterpret_cast<const std::byte*>(data + sizeof(header)), header.payload_size);
}

std::vector<std::uint8_t> WebTransportMessaging::make_frame(
    webtransport_protocol::MessageId message_id, const void* data, std::size_t size) const {
    using webtransport_protocol::FrameHeader;
    if (data == nullptr || size > std::numeric_limits<std::uint32_t>::max()) return {};
    FrameHeader header{message_id, 0, static_cast<std::uint32_t>(size)};
    std::vector<std::uint8_t> frame(sizeof(header) + size);
    std::memcpy(frame.data(), &header, sizeof(header));
    std::memcpy(frame.data() + sizeof(header), data, size);
    return frame;
}

} // namespace sim3dview
