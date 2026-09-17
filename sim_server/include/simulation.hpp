#pragma once

// シミュレーション本体。DESIGN.md 9節フェーズ5:
// 「実シミュレーションループとの結合、コマンドキュー実装、原点状態の保持・変更ガード実装」
//
// スレッドモデル(指示書 C++側実装要件 / DESIGN.md):
// - .messageハンドラ(uWSイベントループスレッド)で受信したコマンドは、直接状態を書き換えず
//   enqueue_command()でスレッドセーフなキューに積むだけにする。
// - simスレッド側でstep()を呼ぶたびに、キューを消費してから物理状態を1ステップ進める。
// - 状態の読み取り(snapshot系)はuWSスレッド(broadcast用)からも呼ばれるため、
//   mutexで保護する。

#include <cstdint>
#include <deque>
#include <mutex>
#include <string>
#include <vector>

#include "protocol.hpp"

namespace sim3dview {

// 接続クライアントを識別する不透明なID。
// uWebSocketsの`WebSocket*`はuWSイベントループスレッドでしか安全に扱えないため、
// simスレッドとのやり取りにはこのIDだけを使う(実際のポインタへの変換はws_server側の責務)。
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
    // 送信元クライアントだけに返すエラー(DESIGN.md 4.3節)。
    std::vector<OutgoingCommandError> errors;
    // trueなら新しいOriginStateを全クライアントへ再配信する(DESIGN.md 4.1節)。
    bool origin_changed = false;
};

class Simulation {
public:
    Simulation();

    // コマンドをキューに積む。どのスレッドからでも呼べる(スレッドセーフ)。
    void enqueue_command(ClientId client_id, protocol::ClientCommand cmd);

    // 1ステップ進める: キュー内のコマンドを消費して適用してから、物理状態を更新する。
    // simスレッド専用(呼び出しスレッドを固定すること)。
    SimulationTickResult step(double dt);

    // 以下はすべてスレッドセーフ(uWSスレッドからのbroadcast用スナップショット取得)。
    protocol::SimState snapshot_sim_state() const;
    protocol::OriginState snapshot_origin() const;

    // VabConfig/StatusPanelConfigは起動後不変(v1はダミー固定値)なので、
    // 生成後は読み取り専用として扱い、mutex保護なしで直接返してよい。
    const protocol::VabConfig& vab_config() const { return vab_config_; }
    const protocol::StatusPanelConfig& status_panel_config() const {
        return status_panel_config_;
    }

private:
    void apply_queued_commands(std::vector<OutgoingCommandError>& out_errors,
                                bool& out_origin_changed);
    void apply_command(const protocol::ClientCommand& cmd, ClientId client_id,
                        std::vector<OutgoingCommandError>& out_errors, bool& out_origin_changed);
    void apply_set_origin(const protocol::ClientCommand& cmd, ClientId client_id,
                           std::vector<OutgoingCommandError>& out_errors,
                           bool& out_origin_changed);

    // 地形データの緯度経度範囲。前処理ツール未実装のため、DESIGN.md 1.2節の実測値を
    // 暫定的にここへハードコードする(前処理ツール実装後はmetadata.jsonから読む想定)。
    static constexpr double kMinLat = 35.0;
    static constexpr double kMaxLat = 40.0;
    static constexpr double kMinLon = 135.0;
    static constexpr double kMaxLon = 140.0;

    mutable std::mutex state_mutex_; // origin_ / running_ / t_ / frame_id_ を保護

    protocol::OriginState origin_{35.355556, 138.859722}; // DESIGN.md 3.1節: 試験用デフォルト原点
    bool running_ = false; // DESIGN.md 3.4節: 原点変更ガード用
    double t_ = 0.0;
    uint32_t frame_id_ = 0;

    protocol::VabConfig vab_config_;
    protocol::StatusPanelConfig status_panel_config_;

    std::mutex queue_mutex_;
    std::deque<QueuedCommand> command_queue_;
};

} // namespace sim3dview
