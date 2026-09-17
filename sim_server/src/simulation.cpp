#include "simulation.hpp"

#include <cmath>
#include <iostream>

namespace sim3dview {

namespace {

protocol::VabConfig make_dummy_vab_config() {
    // DESIGN.md 6.2節: 開発用ダミー値は縦4行×横6列。
    constexpr uint32_t kRows = 4;
    constexpr uint32_t kCols = 6;

    protocol::VabConfig config;
    config.rows = kRows;
    config.cols = kCols;
    config.buttons.reserve(kRows * kCols);
    for (uint32_t row = 0; row < kRows; ++row) {
        for (uint32_t col = 0; col < kCols; ++col) {
            protocol::VabButton button;
            button.id = "btn_" + std::to_string(row) + "_" + std::to_string(col);
            // 6.2節: 空ラベルのボタンは「未使用の穴」。ダミーとして右下の1個を穴にしておく。
            const bool is_hole = (row == kRows - 1) && (col == kCols - 1);
            button.label = is_hole ? "" : ("B" + std::to_string(row * kCols + col + 1));
            button.enabled = !is_hole;
            config.buttons.push_back(std::move(button));
        }
    }
    return config;
}

protocol::StatusPanelConfig make_dummy_status_panel_config() {
    // DESIGN.md 4.4節: 表示項目はC++側が自由に定義する。開発用ダミー項目。
    protocol::StatusPanelConfig config;
    config.items = {
        protocol::StatusItem{"elapsed_time", "経過時間", "s"},
        protocol::StatusItem{"altitude", "高度", "m"},
        protocol::StatusItem{"speed", "速度", "m/s"},
    };
    return config;
}

} // namespace

Simulation::Simulation()
    : vab_config_(make_dummy_vab_config()),
      status_panel_config_(make_dummy_status_panel_config()) {}

void Simulation::enqueue_command(ClientId client_id, protocol::ClientCommand cmd) {
    std::lock_guard<std::mutex> lock(queue_mutex_);
    command_queue_.push_back(QueuedCommand{client_id, std::move(cmd)});
}

SimulationTickResult Simulation::step(double dt) {
    SimulationTickResult result;
    apply_queued_commands(result.errors, result.origin_changed);

    // 物理状態の更新。実シミュレーションロジック自体は本プロジェクトのスコープ外のため、
    // フェーズ1と同じダミーの円軌道を、ここでは正式なsimスレッドのstep()内部状態として進める。
    // running中のみ時間を進める(pause/resumeが意味を持つように)。
    std::lock_guard<std::mutex> lock(state_mutex_);
    if (running_) {
        t_ += dt;
    }
    ++frame_id_;

    return result;
}

void Simulation::apply_queued_commands(std::vector<OutgoingCommandError>& out_errors,
                                        bool& out_origin_changed) {
    std::deque<QueuedCommand> pending;
    {
        std::lock_guard<std::mutex> lock(queue_mutex_);
        pending.swap(command_queue_);
    }
    for (const auto& queued : pending) {
        apply_command(queued.cmd, queued.client_id, out_errors, out_origin_changed);
    }
}

void Simulation::apply_command(const protocol::ClientCommand& cmd, ClientId client_id,
                                std::vector<OutgoingCommandError>& out_errors,
                                bool& out_origin_changed) {
    if (cmd.type == "set_origin") {
        apply_set_origin(cmd, client_id, out_errors, out_origin_changed);
    } else if (cmd.type == "pause") {
        std::lock_guard<std::mutex> lock(state_mutex_);
        running_ = false;
    } else if (cmd.type == "resume") {
        std::lock_guard<std::mutex> lock(state_mutex_);
        running_ = true;
    } else if (cmd.type == "vab_press") {
        // 実アクチュエーション対象がないため、フェーズ5時点でも押下ログのみ。
        std::cout << "[vab_press] button_id=" << cmd.button_id << std::endl;
    } else if (cmd.type == "set_param") {
        std::cout << "[set_param] value=" << cmd.value << std::endl;
    } else {
        std::cerr << "[simulation] unknown ClientCommand.type: " << cmd.type << std::endl;
    }
}

void Simulation::apply_set_origin(const protocol::ClientCommand& cmd, ClientId client_id,
                                   std::vector<OutgoingCommandError>& out_errors,
                                   bool& out_origin_changed) {
    std::lock_guard<std::mutex> lock(state_mutex_);

    // DESIGN.md 3.4節: シミュレーション実行中は原点変更不可。
    if (running_) {
        out_errors.push_back(OutgoingCommandError{
            client_id,
            protocol::CommandError{cmd.type, "シミュレーション実行中は原点を変更できません"}});
        return;
    }

    // DESIGN.md 3.5節: geodetic_bounds範囲外はサーバー側でも拒否する(防御的チェック)。
    if (cmd.lat_deg < kMinLat || cmd.lat_deg > kMaxLat || cmd.lon_deg < kMinLon ||
        cmd.lon_deg > kMaxLon) {
        out_errors.push_back(OutgoingCommandError{
            client_id,
            protocol::CommandError{cmd.type, "指定された緯度経度が地形データの範囲外です"}});
        return;
    }

    origin_.lat_deg = cmd.lat_deg;
    origin_.lon_deg = cmd.lon_deg;
    out_origin_changed = true;
}

protocol::SimState Simulation::snapshot_sim_state() const {
    double t;
    uint32_t frame_id;
    {
        std::lock_guard<std::mutex> lock(state_mutex_);
        t = t_;
        frame_id = frame_id_;
    }

    protocol::SimState state;
    state.t = t;
    state.frame_id = frame_id;

    // ダミーの単一点位置(円軌道)。DESIGN.md 5節の座標系(東=X,北=Y,上=Z)に合わせる。
    const float radius = 100.0f;
    const float x = radius * std::cos(static_cast<float>(t));
    const float y = radius * std::sin(static_cast<float>(t));
    const float z = 50.0f + 10.0f * std::sin(static_cast<float>(t) * 0.5f);
    state.positions = {x, y, z};

    // status_panel_config_.items と同じ順序・同じ数のダミー値を生成する。
    state.status_values.reserve(status_panel_config_.items.size());
    for (size_t i = 0; i < status_panel_config_.items.size(); ++i) {
        state.status_values.push_back(t + static_cast<double>(i));
    }
    return state;
}

protocol::OriginState Simulation::snapshot_origin() const {
    std::lock_guard<std::mutex> lock(state_mutex_);
    return origin_;
}

} // namespace sim3dview
