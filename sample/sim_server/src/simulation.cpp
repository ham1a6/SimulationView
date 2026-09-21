#include "simulation.hpp"

#include <algorithm>
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

// デモシナリオ(航跡表示の確認用)。sim3dviewのデフォルト原点(Simulation::origin_の初期値、富士山の近く)の
// まわりに、種別・所属の違うトラックを周回させる。実際のシミュレータでは、ここを自分のシミュレーション結果
// (機体の位置・速度など)からTrackを作る処理に置き換える。
double normalize_deg(double deg) {
    deg = std::fmod(deg, 360.0);
    return deg < 0.0 ? deg + 360.0 : deg;
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
      status_panel_config_(make_dummy_status_panel_config()),
      scenario_(make_demo_scenario(origin_)) {
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
        result.time_advanced = true;
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

// デモシナリオ。`center`(デフォルト原点。富士山の近く)の東西・南北の距離(メートル)で軌道の中心を決める。
// 海(駿河湾)を行く艦船や、地形の高さをサーバーが持たない地上車両(AboveGround)も含める。
std::vector<Simulation::ScenarioTrack> Simulation::make_demo_scenario(
    const protocol::OriginState& center) {
    using protocol::AltitudeRef;
    using protocol::TrackAffiliation;
    using protocol::TrackKind;
    // 中心から(east_m, north_m)離れた点の緯度経度(近似。デモ用なので球面の誤差は無視する)。
    constexpr double kMetersPerDegree = 111320.0;
    const double cos_lat = std::cos(center.lat_deg * 3.14159265358979323846 / 180.0);
    auto at = [&](double east_m, double north_m) {
        return std::pair<double, double>{center.lat_deg + north_m / kMetersPerDegree,
                                          center.lon_deg + east_m / (kMetersPerDegree * cos_lat)};
    };
    std::vector<ScenarioTrack> tracks;
    auto add = [&](uint32_t id, TrackKind kind, TrackAffiliation affiliation, const char* label,
                   double east_m, double north_m, double radius_east_m, double radius_north_m,
                   double speed_mps, double phase_rad, bool clockwise, AltitudeRef alt_ref,
                   double altitude_m, double altitude_swing_m) {
        const auto [lat, lon] = at(east_m, north_m);
        tracks.push_back(ScenarioTrack{id, kind, affiliation, label, lat, lon, radius_east_m,
                                        radius_north_m, speed_mps, phase_rad, clockwise, alt_ref,
                                        altitude_m, altitude_swing_m});
    };
    //  id  種別                     所属                          名前      中心(東,北)m         半径(東,北)m     速さ  位相  回り  高度基準          高度   上下
    add(1, TrackKind::Aircraft,   TrackAffiliation::Friendly, "AC101",  0,      0,       25000,  25000,  200, 0.0,  true,  AltitudeRef::Msl,          4000, 0);
    add(2, TrackKind::Aircraft,   TrackAffiliation::Hostile,  "BOGEY1", -20000, 10000,   45000,  15000,  250, 2.0,  false, AltitudeRef::Msl,          8000, 1500);
    add(3, TrackKind::Helicopter, TrackAffiliation::Friendly, "HELI1",  6000,   6000,    6000,   6000,   50,  1.0,  true,  AltitudeRef::AboveGround,  300,  100);
    add(4, TrackKind::Ship,       TrackAffiliation::Neutral,  "SHIP1",  -25000, -70000,  15000,  8000,   12,  0.5,  true,  AltitudeRef::Msl,          0,    0);
    add(5, TrackKind::Vehicle,    TrackAffiliation::Friendly, "TRK1",   12000,  -6000,   3000,   3000,   15,  3.0,  false, AltitudeRef::AboveGround,  0,    0);
    add(6, TrackKind::Aircraft,   TrackAffiliation::Unknown,  "UNK1",   -10000, 30000,   12000,  30000,  220, 4.0,  true,  AltitudeRef::Msl,          6000, 0);
    add(7, TrackKind::Missile,    TrackAffiliation::Hostile,  "MSL1",   15000,  15000,   9000,   9000,   600, 1.5,  true,  AltitudeRef::Msl,          5000, 2000);
    return tracks;
}

protocol::TrackList Simulation::snapshot_tracks() const {
    double t;
    {
        std::lock_guard<std::mutex> lock(state_mutex_);
        t = t_;
    }
    constexpr double kMetersPerDegree = 111320.0;
    constexpr double kPi = 3.14159265358979323846;

    protocol::TrackList list;
    list.t = t;
    list.tracks.reserve(scenario_.size());
    for (const ScenarioTrack& s : scenario_) {
        // 楕円軌道: 位相phi(0=中心の真北、増えると時計回り)。平均半径での周回速度がspeed_mpsになる角速度。
        const double direction = s.clockwise ? 1.0 : -1.0;
        const double omega = s.speed_mps / (0.5 * (s.radius_east_m + s.radius_north_m));
        const double phi = s.phase_rad + direction * omega * t;
        const double east_m = s.radius_east_m * std::sin(phi);
        const double north_m = s.radius_north_m * std::cos(phi);
        // 速度ベクトル(東・北成分)から、進行方向(北から時計回り)と対地速度を求める。
        const double velocity_east = s.radius_east_m * std::cos(phi) * omega * direction;
        const double velocity_north = -s.radius_north_m * std::sin(phi) * omega * direction;

        protocol::Track track;
        track.id = s.id;
        track.kind = static_cast<uint8_t>(s.kind);
        track.affiliation = static_cast<uint8_t>(s.affiliation);
        track.label = s.label;
        track.lat_deg = s.center_lat_deg + north_m / kMetersPerDegree;
        track.lon_deg = s.center_lon_deg +
                        east_m / (kMetersPerDegree * std::cos(s.center_lat_deg * kPi / 180.0));
        track.alt_ref = static_cast<uint8_t>(s.alt_ref);
        track.alt_m = s.altitude_m + s.altitude_swing_m * std::sin(0.5 * omega * t + s.phase_rad);
        track.heading_deg = normalize_deg(std::atan2(velocity_east, velocity_north) * 180.0 / kPi);
        track.speed_mps = std::hypot(velocity_east, velocity_north);

        // ピッチ・ロール(フロントの3Dモデル表示の向き。デモ用の簡易な値)。
        // 飛翔体・航空機: ピッチ=上昇・降下の角度(高度の変化率と対地速度から)、ロール=旋回のバンク角(旋回の角速度×速度/重力)。
        // 艦船・車両: 波・路面による小さな揺れ。
        constexpr double kGravity = 9.80665;
        const double vertical_speed =
            s.altitude_swing_m * std::cos(0.5 * omega * t + s.phase_rad) * 0.5 * omega;
        const double climb_deg = std::atan2(vertical_speed, track.speed_mps) * 180.0 / kPi;
        const double bank_deg =
            std::clamp(std::atan(track.speed_mps * direction * omega / kGravity) * 180.0 / kPi, -60.0, 60.0);
        switch (s.kind) {
        case protocol::TrackKind::Aircraft:
        case protocol::TrackKind::Missile:
            track.pitch_deg = climb_deg;
            track.roll_deg = bank_deg;
            break;
        case protocol::TrackKind::Helicopter:
            track.pitch_deg = climb_deg - 6.0; // 前進飛行で機首を少し下げる
            track.roll_deg = bank_deg;
            break;
        case protocol::TrackKind::Ship:
            track.pitch_deg = 1.5 * std::sin(0.35 * t + 2.0 * s.phase_rad);
            track.roll_deg = 4.0 * std::sin(0.5 * t + s.phase_rad);
            break;
        case protocol::TrackKind::Vehicle:
            track.pitch_deg = 2.0 * std::sin(0.9 * t + s.phase_rad);
            track.roll_deg = 2.0 * std::sin(1.3 * t + 2.0 * s.phase_rad);
            break;
        default:
            break;
        }
        list.tracks.push_back(std::move(track));
    }
    return list;
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
