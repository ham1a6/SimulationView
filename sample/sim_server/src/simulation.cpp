#include "simulation.hpp"

#include <cmath>
#include <fstream>
#include <iostream>
#include <sstream>

namespace sim3dview {

namespace {

protocol::VabConfig make_dummy_vab_config() {
    // DETAILED_DESIGN.md 7.4節: 開発用ダミー値は縦6行(1行+4行+1行のグループ分け)×横4列。
    // フロント側(vab.rs)が「最初の行・最後の行」を汎用的に離して描画するため、
    // rows=6にするだけで見た目上は1+4+1のグループ構成になる(グループ数自体を
    // サーバー側で持つ必要はない)。
    constexpr uint32_t kRows = 6;
    constexpr uint32_t kCols = 4;

    protocol::VabConfig config;
    config.rows = kRows;
    config.cols = kCols;
    config.buttons.reserve(kRows * kCols);
    for (uint32_t row = 0; row < kRows; ++row) {
        for (uint32_t col = 0; col < kCols; ++col) {
            protocol::VabButton button;
            button.id = "btn_" + std::to_string(row) + "_" + std::to_string(col);
            // DETAILED_DESIGN.md 7.4節: 空ラベルのボタンは「未使用の穴」。ダミーとして
            // 中央4行ブロックの右下1個を穴にしておく(先頭行・末尾行は単独ボタン行として
            // 常に埋めておいたほうが見た目が自然なため)。
            const bool is_hole = (row == kRows - 2) && (col == kCols - 1);
            button.label = is_hole ? "" : ("B" + std::to_string(row * kCols + col + 1));
            button.enabled = !is_hole;
            config.buttons.push_back(std::move(button));
        }
    }
    return config;
}

protocol::StatusPanelConfig make_dummy_status_panel_config() {
    // DETAILED_DESIGN.md 7.5節: 表示項目はC++側が自由に定義する。開発用ダミー項目。
    protocol::StatusPanelConfig config;
    config.items = {
        protocol::StatusItem{"elapsed_time", "経過時間", "s"},
        protocol::StatusItem{"altitude", "高度", "m"},
        protocol::StatusItem{"speed", "速度", "m/s"},
    };
    return config;
}

// metadata.jsonから`"<key>": <数値>`の数値を取り出す最小限のパーサ(JSONライブラリを
// 足すほどではないため。geotiff_preprocessが書き出す単純な固定形式だけを想定する)。
// startより後ろで最初に見つかったkeyを対象にする。
std::optional<double> find_number(const std::string& text, const std::string& key, size_t start) {
    const size_t key_pos = text.find("\"" + key + "\"", start);
    if (key_pos == std::string::npos) {
        return std::nullopt;
    }
    const size_t colon = text.find(':', key_pos);
    if (colon == std::string::npos) {
        return std::nullopt;
    }
    try {
        return std::stod(text.substr(colon + 1));
    } catch (const std::exception&) {
        return std::nullopt;
    }
}

} // namespace

Simulation::Simulation(const std::string& terrain_metadata_path)
    : vab_config_(make_dummy_vab_config()),
      status_panel_config_(make_dummy_status_panel_config()) {
    std::ifstream file(terrain_metadata_path);
    std::ostringstream buffer;
    buffer << file.rdbuf();
    const std::string text = buffer.str();

    const size_t section = text.find("\"geodetic_bounds\"");
    if (section != std::string::npos) {
        const auto min_lat = find_number(text, "min_lat", section);
        const auto max_lat = find_number(text, "max_lat", section);
        const auto min_lon = find_number(text, "min_lon", section);
        const auto max_lon = find_number(text, "max_lon", section);
        if (min_lat && max_lat && min_lon && max_lon) {
            bounds_ = GeodeticBounds{*min_lat, *max_lat, *min_lon, *max_lon};
            std::cout << "[simulation] terrain bounds: lat " << *min_lat << ".." << *max_lat
                      << ", lon " << *min_lon << ".." << *max_lon << std::endl;
        }
    }
    if (!bounds_) {
        std::cerr << "[simulation] WARNING: failed to read geodetic_bounds from "
                  << terrain_metadata_path << " (origin range check disabled)" << std::endl;
    }
}

void Simulation::enqueue_command(ClientId client_id, protocol::ClientCommand cmd) {
    std::lock_guard<std::mutex> lock(queue_mutex_);
    command_queue_.push_back(QueuedCommand{client_id, std::move(cmd)});
}

SimulationTickResult Simulation::step(double dt) {
    SimulationTickResult result;
    apply_queued_commands(result.errors, result.origin_changed, result.app_status_changed);

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
                                        bool& out_origin_changed, bool& out_app_status_changed) {
    std::deque<QueuedCommand> pending;
    {
        std::lock_guard<std::mutex> lock(queue_mutex_);
        pending.swap(command_queue_);
    }
    for (const auto& queued : pending) {
        apply_command(queued.cmd, queued.client_id, out_errors, out_origin_changed,
                      out_app_status_changed);
    }
}

void Simulation::apply_command(const protocol::ClientCommand& cmd, ClientId client_id,
                                std::vector<OutgoingCommandError>& out_errors,
                                bool& out_origin_changed, bool& out_app_status_changed) {
    if (cmd.type == "set_origin") {
        apply_set_origin(cmd, client_id, out_errors, out_origin_changed);
    } else if (cmd.type == "pause") {
        std::lock_guard<std::mutex> lock(state_mutex_);
        if (running_) {
            running_ = false;
            out_app_status_changed = true;
        }
    } else if (cmd.type == "resume") {
        std::lock_guard<std::mutex> lock(state_mutex_);
        if (!running_) {
            running_ = true;
            out_app_status_changed = true;
        }
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

    // DETAILED_DESIGN.md 3.4節: シミュレーション実行中は原点変更不可。
    if (running_) {
        out_errors.push_back(OutgoingCommandError{
            client_id,
            protocol::CommandError{cmd.type, "シミュレーション実行中は原点を変更できません"}});
        return;
    }

    // DETAILED_DESIGN.md 3.5節: geodetic_bounds範囲外はサーバー側でも拒否する(防御的チェック)。
    if (bounds_ && (cmd.lat_deg < bounds_->min_lat || cmd.lat_deg > bounds_->max_lat ||
                    cmd.lon_deg < bounds_->min_lon || cmd.lon_deg > bounds_->max_lon)) {
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

    // ダミーの単一点位置(円軌道)。DETAILED_DESIGN.md 6.6節の座標系(東=X,北=Y,上=Z)に合わせる。
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

protocol::AppStatus Simulation::snapshot_app_status() const {
    std::lock_guard<std::mutex> lock(state_mutex_);
    protocol::AppStatus status;
    status.text = running_ ? "シミュレーション実行中" : "一時停止中";
    return status;
}

} // namespace sim3dview
