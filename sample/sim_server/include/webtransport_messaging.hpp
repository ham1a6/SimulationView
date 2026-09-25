#pragma once

// C++ 側の WebTransport 送受信専用クラス。
// 詳細は sample/docs/DETAILED_DESIGN.md 4節・5節。

#include <cstddef>
#include <cstdint>
#include <functional>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>

#include "webtransport_protocol.hpp"

namespace sim3dview {

using TransportClientId = std::uint64_t;

enum class DeliveryMode : std::uint8_t {
    ReliableStream = 0,
    UnreliableDatagram = 1,
};

class WebTransportMessaging {
public:
    using ReceiveHandler = std::function<void(TransportClientId, const std::byte*, std::size_t)>;
    using ConnectionHandler = std::function<void(TransportClientId)>;

    WebTransportMessaging();
    ~WebTransportMessaging();
    WebTransportMessaging(const WebTransportMessaging&) = delete;
    WebTransportMessaging& operator=(const WebTransportMessaging&) = delete;

    // UDPのportでHTTP/3 WebTransportを開始する。cert_file/key_file は必須。
    bool start(std::uint16_t port, const std::string& cert_file, const std::string& key_file,
               std::string& error);

    void set_connection_handlers(ConnectionHandler on_connect, ConnectionHandler on_disconnect);
    void register_handler(webtransport_protocol::MessageId message_id, ReceiveHandler handler);

    // 単一クライアントへの送信。data/size はヘッダを除く C++ 構造体の生バイト列。
    bool send(TransportClientId client_id, webtransport_protocol::MessageId message_id,
              DeliveryMode delivery, const void* data, std::size_t size);
    // 接続済み全クライアントへの一斉送信。
    bool broadcast(webtransport_protocol::MessageId message_id, DeliveryMode delivery,
                   const void* data, std::size_t size);

    template <typename T>
    bool send_struct(TransportClientId client_id, webtransport_protocol::MessageId message_id,
                     DeliveryMode delivery, const T& value) {
        static_assert(webtransport_protocol::kWirePayload<T>);
        return send(client_id, message_id, delivery, &value, sizeof(value));
    }

    template <typename T>
    bool broadcast_struct(webtransport_protocol::MessageId message_id, DeliveryMode delivery,
                          const T& value) {
        static_assert(webtransport_protocol::kWirePayload<T>);
        return broadcast(message_id, delivery, &value, sizeof(value));
    }

private:
    static void on_connected(void* context, TransportClientId client_id);
    static void on_disconnected(void* context, TransportClientId client_id);
    static void on_received(void* context, TransportClientId client_id, std::uint8_t delivery,
                            const std::uint8_t* data, std::size_t size);
    void dispatch(TransportClientId client_id, DeliveryMode delivery, const std::uint8_t* data,
                  std::size_t size);
    std::vector<std::uint8_t> make_frame(webtransport_protocol::MessageId message_id,
                                         const void* data, std::size_t size) const;

    std::mutex mutex_;
    std::unordered_map<webtransport_protocol::MessageId, ReceiveHandler> handlers_;
    ConnectionHandler on_connect_;
    ConnectionHandler on_disconnect_;
    bool started_ = false;
};

} // namespace sim3dview
