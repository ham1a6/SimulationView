#pragma once

// シミュレーション本体。DETAILED_DESIGN.md 5.1〜5.2節:
// 「実シミュレーションループとの結合、コマンドキュー実装、原点状態の保持・変更ガード実装」
//
// スレッドモデル(DETAILED_DESIGN.md 5.1節):
// - WebTransport受信コールバックで受信したコマンドは、直接状態を書き換えず
//   enqueue_command()でスレッドセーフなキューに積むだけにする。
// - simスレッド側でstep()を呼ぶたびに、キューを消費してから物理状態を1ステップ進める。
// - 状態の読み取り(snapshot系)はWebTransport送信側(broadcast用)からも呼ばれるため、
//   mutexで保護する。

#include <cstdint>
#include <deque>
#include <mutex>
#include <optional>
#include <string>
#include <vector>

#include "protocol.hpp"

namespace sim3dview {

// 接続クライアントを識別する不透明なID。
// WebTransport接続はランタイム固有であるため、
// simスレッドとのやり取りにはこのIDだけを使う(接続表の管理は送信層の責務)。
using ClientId = uint64_t;

// enqueue_command()で積まれる1件分。
struct QueuedCommand {
    ClientId client_id;
    protocol::ClientCommand cmd;
};

// step()の結果、要求元クライアントに送り返すべきCommandError。
struct OutgoingCommandError {
    ClientId client_id;
    protocol::CommandError error;
};

// step()1回分の結果。ws_server側はこれを見て、必要な配信(defer経由)を行う。
struct SimulationTickResult {
    // 送信元クライアントだけに返すエラー(DETAILED_DESIGN.md 4.3節)。
    std::vector<OutgoingCommandError> errors;
    // trueなら新しいOriginStateを全クライアントへ再配信する(DETAILED_DESIGN.md 4.3節)。
    bool origin_changed = false;
    // trueなら新しいAppStatusを全クライアントへ再配信する(pause/resumeで変化した時)。
    bool app_status_changed = false;
    // trueならシミュレーション時刻が進んだ(=航跡の位置が変わった)。ws_server側はこれを見て、
    // 一定間隔でTrackListを配信する。
    bool time_advanced = false;
};

class Simulation {
public:
    // terrain_metadata_path: 地形データのmetadata.json。原点の受理範囲(geodetic_bounds)を
    // ここから読む(想定CWDはsim_server/。ws_server.cppの地形配信と同じ相対パス)。
    explicit Simulation(const std::string& terrain_metadata_path = "assets/terrain/metadata.json");

    // コマンドをキューに積む。どのスレッドからでも呼べる(スレッドセーフ)。
    void enqueue_command(ClientId client_id, protocol::ClientCommand cmd);

    // 1ステップ進める: キュー内のコマンドを消費して適用してから、物理状態を更新する。
    // simスレッド専用(呼び出しスレッドを固定すること)。
    SimulationTickResult step(double dt);

    // 以下はすべてスレッドセーフ(uWSスレッドからのbroadcast用スナップショット取得)。
    protocol::SimState snapshot_sim_state() const;
    protocol::OriginState snapshot_origin() const;
    protocol::AppStatus snapshot_app_status() const;
    // デモシナリオの航跡(航空機・ヘリ・艦船・車両)の、現在のシミュレーション時刻での状態。
    protocol::TrackList snapshot_tracks() const;

    // StatusPanelConfigは起動後不変(v1はダミー固定値)なので、
    // 生成後は読み取り専用として扱い、mutex保護なしで直接返してよい。
    const protocol::StatusPanelConfig& status_panel_config() const {
        return status_panel_config_;
    }

private:
    void apply_queued_commands(std::vector<OutgoingCommandError>& out_errors,
                                bool& out_origin_changed, bool& out_app_status_changed);
    void apply_command(const protocol::ClientCommand& cmd, ClientId client_id,
                        std::vector<OutgoingCommandError>& out_errors, bool& out_origin_changed,
                        bool& out_app_status_changed);
    void apply_set_origin(const protocol::ClientCommand& cmd, ClientId client_id,
                           std::vector<OutgoingCommandError>& out_errors,
                           bool& out_origin_changed);

    // 地形データの緯度経度範囲(metadata.jsonのgeodetic_bounds)。起動時に一度だけ読む。
    // 読めなかった場合は範囲チェックを行わない(警告を出す。フロント側の入力段階でも
    // 同じ範囲でブロックしているため、サーバー側は防御的な二重チェックの位置づけ)。
    struct GeodeticBounds {
        double min_lat, max_lat, min_lon, max_lon;
    };
    std::optional<GeodeticBounds> bounds_;

    mutable std::mutex state_mutex_; // origin_ / running_ / t_ / frame_id_ を保護

    protocol::OriginState origin_{35.355556, 138.859722}; // DETAILED_DESIGN.md 3.1節: 試験用デフォルト原点
    bool running_ = false; // DETAILED_DESIGN.md 3.4節: 原点変更ガード用
    double t_ = 0.0;
    uint32_t frame_id_ = 0;

    protocol::StatusPanelConfig status_panel_config_;

    // デモシナリオ: 航跡1個分の周回軌道。中心(center)のまわりの楕円(東西radius_east_m・南北radius_north_m)を、
    // 平均半径での周回速度がspeed_mpsになる角速度で回る(位置はシミュレーション時刻の関数なので、
    // 一時停止・再開しても状態は不要)。起動後は不変。
    struct ScenarioTrack {
        uint32_t id;
        protocol::TrackKind kind;
        protocol::TrackAffiliation affiliation;
        std::string label;
        double center_lat_deg;
        double center_lon_deg;
        double radius_east_m;
        double radius_north_m;
        double speed_mps;
        double phase_rad;        // t=0での軌道上の位置(0=中心の真北。時計回りに増える)
        bool clockwise;
        protocol::AltitudeRef alt_ref;
        double altitude_m;       // alt_refがMslなら海抜、AboveGroundなら地表からの高さ
        double altitude_swing_m; // 高度が±この値だけ上下する(0なら一定)
    };
    std::vector<ScenarioTrack> scenario_;
    static std::vector<ScenarioTrack> make_demo_scenario(const protocol::OriginState& center);

    std::mutex queue_mutex_;
    std::deque<QueuedCommand> command_queue_;
};

} // namespace sim3dview
