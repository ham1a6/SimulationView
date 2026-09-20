#pragma once

// Sim3dView 通信プロトコル定義。
// フレーミング: [1 byte: msg_type][MessagePack body]
// 詳細: DETAILED_DESIGN.md 4節。C++側/Rust側で内容を一致させること。

#include <cstdint>
#include <string>
#include <vector>

#include <msgpack.hpp>

namespace sim3dview::protocol {

enum class MsgType : uint8_t {
    SimState = 0x01,          // Server -> Client (高頻度)
    VabConfig = 0x02,         // Server -> Client (状態変化時)
    OriginState = 0x03,       // Server -> Client (状態変化時、接続直後にも送信)
    StatusPanelConfig = 0x04, // Server -> Client (状態変化時、接続直後にも送信)
    CommandError = 0x05,      // Server -> Client (要求元クライアントのみ)
    AppStatus = 0x06,         // Server -> Client (状態変化時、接続直後にも送信)
    TrackList = 0x07,         // Server -> Client (シミュレーション進行中は約20Hz、接続直後にも送信)
};

// --- Server -> Client ------------------------------------------------

// シミュレーション状態(高頻度、約60Hz)。DETAILED_DESIGN.md 4.3節。
struct SimState {
    double t = 0.0;
    std::vector<float> positions;
    uint32_t frame_id = 0;
    // StatusPanelConfig.items と同じ順序・同じ数(v1では数値項目のみ)。
    std::vector<double> status_values;

    MSGPACK_DEFINE(t, positions, frame_id, status_values);
};

// VAB(操作ボタン)1個分。ラベルが空文字のボタンは「未使用の穴」(DETAILED_DESIGN.md 7.4節)。
struct VabButton {
    std::string id;
    std::string label;
    bool enabled = true;

    MSGPACK_DEFINE(id, label, enabled);
};

// VABボタン配置設定(状態変化時のみ送信)。
struct VabConfig {
    uint32_t rows = 0;
    uint32_t cols = 0;
    std::vector<VabButton> buttons;

    MSGPACK_DEFINE(rows, cols, buttons);
};

// 基準位置(原点)。DETAILED_DESIGN.md 3節・4.3節。サーバーが保持する状態が正。
struct OriginState {
    double lat_deg = 0.0;
    double lon_deg = 0.0;

    MSGPACK_DEFINE(lat_deg, lon_deg);
};

// 状況パネル項目1個分。DETAILED_DESIGN.md 4.3節・7.5節。
struct StatusItem {
    std::string id;
    std::string label;
    std::string unit; // 単位。なければ空文字列

    MSGPACK_DEFINE(id, label, unit);
};

// 状況パネル項目定義(状態変化時のみ送信)。実際の値は SimState.status_values で配信する。
struct StatusPanelConfig {
    std::vector<StatusItem> items;

    MSGPACK_DEFINE(items);
};

// コマンド拒否応答。要求元クライアントのみに送信する。DETAILED_DESIGN.md 4.3節。
struct CommandError {
    std::string command_type; // 拒否された ClientCommand.type
    std::string message;      // エラー内容(人間可読)

    MSGPACK_DEFINE(command_type, message);
};

// シミュレータアプリケーション自体の状態を表す表示用文字列(状態変化時+接続直後)。
// フロント側のシミュレーションステータスパネルは、WebSocket接続が確立している間は
// この文字列をそのまま表示する(接続そのものの状態はConnectionStatusとして
// フロント側が自前で計算する、別レイヤーの情報)。
struct AppStatus {
    std::string text; // 例: "シミュレーション実行中" / "一時停止中"

    MSGPACK_DEFINE(text);
};

// 航跡(トラック)1個分: 航空機・艦船・車両等の現在位置とシンボル情報。DETAILED_DESIGN.md 4.3節。
// kind/affiliation/alt_refは下のenumの値をそのまま送る(Rust側protocol.rsと値を一致させること)。
enum class TrackKind : uint8_t {
    Unknown = 0,
    Aircraft = 1,   // 固定翼機
    Helicopter = 2,
    Ship = 3,
    Vehicle = 4,    // 地上車両
    Missile = 5,
};
enum class TrackAffiliation : uint8_t {
    Unknown = 0,
    Friendly = 1,   // 友軍
    Hostile = 2,    // 敵
    Neutral = 3,    // 中立
};
enum class AltitudeRef : uint8_t {
    Msl = 0,          // alt_m は海抜(地形データの標高と同じ基準)
    AboveGround = 1,  // alt_m は地表からの高さ(サーバーが地形の高さを持たない車両などに使う)
};

struct Track {
    uint32_t id = 0;         // 同じ実体には常に同じID(フロントの航跡・ラベルの対応づけに使う)
    uint8_t kind = 0;        // TrackKind
    uint8_t affiliation = 0; // TrackAffiliation
    std::string label;       // 表示名(コールサイン等)
    double lat_deg = 0.0;
    double lon_deg = 0.0;
    double alt_m = 0.0;
    uint8_t alt_ref = 0;     // AltitudeRef
    double heading_deg = 0.0; // 進行方向(北から時計回り)
    double speed_mps = 0.0;   // 対地速度

    MSGPACK_DEFINE(id, kind, affiliation, label, lat_deg, lon_deg, alt_m, alt_ref, heading_deg,
                   speed_mps);
};

// 航跡の一覧(全トラックの最新状態をまとめて送る)。トラックが消えたら次の一覧から抜ける。
struct TrackList {
    double t = 0.0; // シミュレーション時刻
    std::vector<Track> tracks;

    MSGPACK_DEFINE(t, tracks);
};

// --- Client -> Server --------------------------------------------------

// クライアントからの操作コマンド。DETAILED_DESIGN.md 4.3節。
struct ClientCommand {
    std::string type; // "vab_press" / "pause" / "resume" / "set_param" / "set_origin"
    std::string button_id; // vab_press時のみ使用
    double value = 0.0;    // set_param時のみ使用
    double lat_deg = 0.0;  // set_origin時のみ使用
    double lon_deg = 0.0;  // set_origin時のみ使用

    MSGPACK_DEFINE(type, button_id, value, lat_deg, lon_deg);
};

// --- フレーミング用ヘルパー ---------------------------------------------

// [1 byte msg_type][MessagePack body] 形式のバイナリフレームを構築する。
template <typename T>
std::string encode_frame(MsgType type, const T& payload) {
    msgpack::sbuffer buf;
    msgpack::pack(buf, payload);

    std::string frame;
    frame.reserve(buf.size() + 1);
    frame.push_back(static_cast<char>(static_cast<uint8_t>(type)));
    frame.append(buf.data(), buf.size());
    return frame;
}

// 受信フレームの先頭1バイトから msg_type を読み取る。フレームが空なら false を返す。
inline bool read_msg_type(const char* data, size_t size, MsgType& out_type) {
    if (size < 1) {
        return false;
    }
    out_type = static_cast<MsgType>(static_cast<uint8_t>(data[0]));
    return true;
}

// フレームの MessagePack body 部分(先頭1バイトを除いた領域)を T にデコードする。
template <typename T>
T decode_body(const char* data, size_t size) {
    // data[0] は msg_type なので、body は data+1 から size-1 バイト。
    msgpack::object_handle handle = msgpack::unpack(data + 1, size - 1);
    return handle.get().as<T>();
}

} // namespace sim3dview::protocol
