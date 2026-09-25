#pragma once

// シミュレーション内部で使う値オブジェクト。
// WebTransportのワイヤ形式は webtransport_protocol.hpp が唯一の定義である。

#include <cstdint>
#include <string>
#include <vector>

namespace sim3dview::protocol {

// --- Server -> Client ------------------------------------------------

// シミュレーション状態(高頻度、約60Hz)。DETAILED_DESIGN.md 4.3節。
struct SimState {
    double t = 0.0;
    std::vector<float> positions;
    uint32_t frame_id = 0;
    // StatusPanelConfig.items と同じ順序・同じ数(v1では数値項目のみ)。
    std::vector<double> status_values;

};

// 基準位置(原点)。DETAILED_DESIGN.md 3節・4.3節。サーバーが保持する状態が正。
struct OriginState {
    double lat_deg = 0.0;
    double lon_deg = 0.0;

};

// 状況パネル項目1個分。DETAILED_DESIGN.md 4.3節・7.5節。
struct StatusItem {
    std::string id;
    std::string label;
    std::string unit; // 単位。なければ空文字列

};

// 状況パネル項目定義(状態変化時のみ送信)。実際の値は SimState.status_values で配信する。
struct StatusPanelConfig {
    std::vector<StatusItem> items;

};

// コマンド拒否応答。要求元クライアントのみに送信する。DETAILED_DESIGN.md 4.3節。
struct CommandError {
    std::string command_type; // 拒否された ClientCommand.type
    std::string message;      // エラー内容(人間可読)

};

// シミュレータアプリケーション自体の状態を表す表示用文字列(状態変化時+接続直後)。
// フロント側のシミュレーションステータスパネルは、WebTransport接続が確立している間は
// この文字列をそのまま表示する(接続そのものの状態はConnectionStatusとして
// フロント側が自前で計算する、別レイヤーの情報)。
struct AppStatus {
    std::string text; // 例: "シミュレーション実行中" / "一時停止中"

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
    double pitch_deg = 0.0;   // ピッチ(機首上げが正)。フロントの3Dモデル表示の向きに使う
    double roll_deg = 0.0;    // ロール(右翼が下がる向きが正)。同上

};

// 航跡の一覧(全トラックの最新状態をまとめて送る)。トラックが消えたら次の一覧から抜ける。
struct TrackList {
    double t = 0.0; // シミュレーション時刻
    std::vector<Track> tracks;

};

// --- Client -> Server --------------------------------------------------

// クライアントからの操作コマンド。DETAILED_DESIGN.md 4.3節。
struct ClientCommand {
    std::string type; // "pause" / "resume" / "set_param" / "set_origin"
    std::string reserved; // 旧ボタンIDの予約スロット(空文字)。配列位置の互換性を維持する。
    double value = 0.0;    // set_param時のみ使用
    double lat_deg = 0.0;  // set_origin時のみ使用
    double lon_deg = 0.0;  // set_origin時のみ使用

};

} // namespace sim3dview::protocol
