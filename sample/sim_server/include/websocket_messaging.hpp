#pragma once

// WebSocketの送受信とメッセージID振り分けだけを担うクラス。
// uWSのWebSocket実体は送信関数として受け取り、HTTPやシミュレーション状態を知らない。

#include <cstddef>
#include <cstdint>
#include <functional>
#include <mutex>
#include <string>
#include <string_view>
#include <unordered_map>

#include "webtransport_protocol.hpp"

namespace sim3dview {

using WebSocketClientId = std::uint64_t;

class WebSocketMessaging {
public:
    using SendFunction = std::function<void(std::string_view)>;
    using ReceiveHandler = std::function<void(WebSocketClientId, const std::byte*, std::size_t)>;

    void connect(WebSocketClientId client_id, SendFunction send);
    void disconnect(WebSocketClientId client_id);
    void register_handler(webtransport_protocol::MessageId message_id, ReceiveHandler handler);

    // data/sizeは固定長構造体のpayloadだけを渡す。ヘッダはこのクラスが付与する。
    bool send(WebSocketClientId client_id, webtransport_protocol::MessageId message_id,
              const void* data, std::size_t size);
    bool broadcast(webtransport_protocol::MessageId message_id, const void* data, std::size_t size);

    template <typename T>
    bool send_struct(WebSocketClientId client_id, webtransport_protocol::MessageId message_id,
                     const T& value) {
        static_assert(webtransport_protocol::kWirePayload<T>);
        return send(client_id, message_id, &value, sizeof(value));
    }
    template <typename T>
    bool broadcast_struct(webtransport_protocol::MessageId message_id, const T& value) {
        static_assert(webtransport_protocol::kWirePayload<T>);
        return broadcast(message_id, &value, sizeof(value));
    }

    // WebSocketの受信メッセージを検証し、message_idに登録されたハンドラへ振り分ける。
    void receive(WebSocketClientId client_id, std::string_view message);

private:
    static std::string make_frame(webtransport_protocol::MessageId message_id,
                                  const void* data, std::size_t size);

    std::mutex mutex_;
    std::unordered_map<WebSocketClientId, SendFunction> clients_;
    std::unordered_map<webtransport_protocol::MessageId, ReceiveHandler> handlers_;
};

} // namespace sim3dview
