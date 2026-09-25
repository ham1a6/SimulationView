#pragma once

// WebSocket 上で送る固定長バイナリプロトコル。
//
// C++ の可変長型(std::string/std::vector 等)をメモリコピーして送ることは ABI・ポインタを
// 送ってしまうため禁止する。ここには数値と固定長配列だけから成る標準レイアウト型だけを置く。
// すべて little endian、IEEE 754 binary64/binary32、下記の sizeof を通信契約とする。

#include <array>
#include <cstddef>
#include <cstdint>
#include <type_traits>

namespace sim3dview::webtransport_protocol {

using MessageId = std::uint16_t;

enum class Message : MessageId {
    ClientCommand = 0x0001,
    SimState = 0x0002,
    OriginState = 0x0003,
    CommandError = 0x0004,
    AppStatus = 0x0005,
    TrackList = 0x0006,
};

enum class Command : std::uint8_t {
    Pause = 1,
    Resume = 2,
    SetParam = 3,
    SetOrigin = 4,
};

enum class CommandErrorCode : std::uint8_t {
    UnsupportedCommand = 1,
    OriginChangeWhileRunning = 2,
    OriginOutsideTerrain = 3,
};

// WebSocketはメッセージ境界を保持するが、メッセージIDとサイズを明示するため
// 同一ヘッダを付ける。payload_size はヘッダを除くバイト数。
struct FrameHeader {
    MessageId message_id;
    std::uint16_t reserved = 0;
    std::uint32_t payload_size;
};

struct ClientCommand {
    Command command;
    std::array<std::uint8_t, 7> reserved{};
    double value = 0.0;
    double lat_deg = 0.0;
    double lon_deg = 0.0;
};

struct SimState {
    double t = 0.0;
    float position_x = 0.0F;
    float position_y = 0.0F;
    float position_z = 0.0F;
    std::uint32_t frame_id = 0;
    double elapsed_time_s = 0.0;
    double altitude_m = 0.0;
    double speed_mps = 0.0;
};

struct OriginState {
    double lat_deg = 0.0;
    double lon_deg = 0.0;
};

struct CommandError {
    Command command;
    CommandErrorCode code;
    std::array<std::uint8_t, 6> reserved{};
};

struct AppStatus {
    // 0=停止中、1=実行中。表示文字列のローカライズはクライアントの責務。
    std::uint8_t running = 0;
    std::array<std::uint8_t, 7> reserved{};
};

constexpr std::size_t kMaxTracks = 16;
constexpr std::size_t kTrackLabelBytes = 24;

struct Track {
    double lat_deg = 0.0;
    double lon_deg = 0.0;
    double alt_m = 0.0;
    double heading_deg = 0.0;
    double speed_mps = 0.0;
    double pitch_deg = 0.0;
    double roll_deg = 0.0;
    std::uint32_t id = 0;
    std::uint8_t kind = 0;
    std::uint8_t affiliation = 0;
    std::uint8_t alt_ref = 0;
    std::uint8_t reserved = 0;
    std::array<char, kTrackLabelBytes> label{}; // UTF-8、NUL終端。長ければ切り詰める。
};

struct TrackList {
    double t = 0.0;
    std::uint32_t count = 0;
    std::uint32_t reserved = 0;
    std::array<Track, kMaxTracks> tracks{};
};

template <typename T>
inline constexpr bool kWirePayload = std::is_standard_layout_v<T> && std::is_trivially_copyable_v<T>;

static_assert(kWirePayload<FrameHeader> && sizeof(FrameHeader) == 8);
static_assert(kWirePayload<ClientCommand> && sizeof(ClientCommand) == 32);
static_assert(kWirePayload<SimState> && sizeof(SimState) == 48);
static_assert(kWirePayload<OriginState> && sizeof(OriginState) == 16);
static_assert(kWirePayload<CommandError> && sizeof(CommandError) == 8);
static_assert(kWirePayload<AppStatus> && sizeof(AppStatus) == 8);
static_assert(kWirePayload<Track> && sizeof(Track) == 88);
static_assert(kWirePayload<TrackList> && sizeof(TrackList) == 1424);

} // namespace sim3dview::webtransport_protocol
