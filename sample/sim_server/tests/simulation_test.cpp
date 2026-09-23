#include "simulation.hpp"

#include <cassert>
#include <filesystem>
#include <fstream>

namespace {
sim3dview::protocol::ClientCommand command(const char* type) {
    sim3dview::protocol::ClientCommand cmd;
    cmd.type = type;
    return cmd;
}
}

int main() {
    const auto metadata = std::filesystem::temp_directory_path() / "sim3dview_simulation_test.json";
    {
        std::ofstream out(metadata);
        out << R"({"geodetic_bounds":{"min_lat":20,"max_lat":50,"min_lon":120,"max_lon":150}})";
    }
    sim3dview::Simulation sim(metadata.string());
    sim.enqueue_command(1, command("resume"));
    auto tick = sim.step(0.5);
    assert(tick.app_status_changed && tick.time_advanced);
    assert(sim.snapshot_sim_state().t == 0.5);

    auto origin = command("set_origin");
    origin.lat_deg = 35.0;
    origin.lon_deg = 139.0;
    sim.enqueue_command(7, origin);
    tick = sim.step(0.0);
    assert(tick.errors.size() == 1 && tick.errors[0].client_id == 7);
    assert(!tick.origin_changed);

    sim.enqueue_command(1, command("pause"));
    sim.enqueue_command(7, origin);
    tick = sim.step(1.0);
    assert(tick.errors.empty() && tick.app_status_changed && tick.origin_changed);
    assert(!tick.time_advanced);
    assert(sim.snapshot_origin().lat_deg == 35.0 && sim.snapshot_origin().lon_deg == 139.0);

    origin.lat_deg = 90.0;
    sim.enqueue_command(9, origin);
    tick = sim.step(0.0);
    assert(tick.errors.size() == 1 && !tick.origin_changed);
    // 廃止したUI操作や未知のコマンドも、要求元に明示的に拒否を返す。
    for (const char* type : {"vab_press", "unknown"}) {
        sim.enqueue_command(11, command(type));
        tick = sim.step(0.0);
        assert(tick.errors.size() == 1 && tick.errors[0].client_id == 11);
        assert(tick.errors[0].error.command_type == type);
        assert(!tick.time_advanced && !tick.origin_changed && !tick.app_status_changed);
    }
    std::filesystem::remove(metadata);
}
