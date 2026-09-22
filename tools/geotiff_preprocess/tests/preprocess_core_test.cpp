#include "preprocess_core.hpp"

#include <cassert>

int main() {
    using namespace sim3dview::preprocess;
    assert(parse_tile_filename("ALPSMLC30_N035E138_DSM.tif").value().lat == 35);
    const auto southwest = parse_tile_filename("ALPSMLC30_S012W077_DSM.tif").value();
    assert(southwest.lat == -12 && southwest.lon == -77);
    assert(!parse_tile_filename("N35E138.tif"));
    assert(tile_name(southwest) == "S012W077");

    const auto bounds = compute_mosaic_bounds({TileId{35, 138}, TileId{33, 140}});
    assert(bounds.min_lat == 33 && bounds.max_lat == 36);
    assert(bounds.min_lon == 138 && bounds.max_lon == 141);

    const std::vector<int16_t> grid = {
        0, 1, 2, 3, 4,
        10, 11, 12, 13, 14,
        20, 21, 22, 23, 24,
        30, 31, 32, 33, 34,
        40, 41, 42, 43, 44,
    };
    const auto chunks = split_chunks(grid, 4, 2);
    assert(chunks.size() == 4);
    assert((chunks[0] == std::vector<int16_t>{0, 1, 2, 10, 11, 12, 20, 21, 22}));
    assert((chunks[3] == std::vector<int16_t>{22, 23, 24, 32, 33, 34, 42, 43, 44}));
}
