#include "websocket_messaging.hpp"

#include <cstring>
#include <limits>
#include <vector>

namespace sim3dview {

void WebSocketMessaging::connect(WebSocketClientId client_id, SendFunction send) {
    std::lock_guard lock(mutex_);
    clients_[client_id] = std::move(send);
}

void WebSocketMessaging::disconnect(WebSocketClientId client_id) {
    std::lock_guard lock(mutex_);
    clients_.erase(client_id);
}

void WebSocketMessaging::register_handler(webtransport_protocol::MessageId message_id,
                                           ReceiveHandler handler) {
    std::lock_guard lock(mutex_);
    handlers_[message_id] = std::move(handler);
}

bool WebSocketMessaging::send(WebSocketClientId client_id, webtransport_protocol::MessageId message_id,
                               const void* data, std::size_t size) {
    const auto message = make_frame(message_id, data, size);
    if (message.empty()) return false;
    SendFunction send_function;
    {
        std::lock_guard lock(mutex_);
        const auto found = clients_.find(client_id);
        if (found == clients_.end()) return false;
        send_function = found->second;
    }
    send_function(message);
    return true;
}

bool WebSocketMessaging::broadcast(webtransport_protocol::MessageId message_id,
                                    const void* data, std::size_t size) {
    const auto message = make_frame(message_id, data, size);
    if (message.empty()) return false;
    std::vector<SendFunction> send_functions;
    {
        std::lock_guard lock(mutex_);
        send_functions.reserve(clients_.size());
        for (const auto& [_, send_function] : clients_) send_functions.push_back(send_function);
    }
    for (const auto& send_function : send_functions) send_function(message);
    return true;
}

void WebSocketMessaging::receive(WebSocketClientId client_id, std::string_view message) {
    using webtransport_protocol::FrameHeader;
    if (message.size() < sizeof(FrameHeader)) return;
    FrameHeader header{};
    std::memcpy(&header, message.data(), sizeof(header));
    if (header.reserved != 0 || header.payload_size != message.size() - sizeof(header)) return;
    ReceiveHandler handler;
    {
        std::lock_guard lock(mutex_);
        const auto found = handlers_.find(header.message_id);
        if (found == handlers_.end()) return;
        handler = found->second;
    }
    handler(client_id, reinterpret_cast<const std::byte*>(message.data() + sizeof(header)), header.payload_size);
}

std::string WebSocketMessaging::make_frame(webtransport_protocol::MessageId message_id,
                                            const void* data, std::size_t size) {
    if (data == nullptr || size > std::numeric_limits<std::uint32_t>::max()) return {};
    const webtransport_protocol::FrameHeader header{message_id, 0, static_cast<std::uint32_t>(size)};
    std::string message(sizeof(header) + size, '\0');
    std::memcpy(message.data(), &header, sizeof(header));
    std::memcpy(message.data() + sizeof(header), data, size);
    return message;
}

} // namespace sim3dview
