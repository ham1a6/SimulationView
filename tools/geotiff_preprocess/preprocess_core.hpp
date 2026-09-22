#pragma once

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <optional>
#include <stdexcept>
#include <string>
#include <vector>

namespace sim3dview::preprocess {

struct TileId {
    int lat = 0;
    int lon = 0;
};

struct MosaicBounds {
    int min_lat = 0;
    int max_lat = 0;
    int min_lon = 0;
    int max_lon = 0;
};

inline std::optional<TileId> parse_tile_filename(const std::string& filename) {
    const size_t suffix = filename.find("_DSM.tif");
    if (suffix == std::string::npos || suffix < 8) return std::nullopt;
    const std::string token = filename.substr(suffix - 8, 8);
    const char lat_hemi = token[0], lon_hemi = token[4];
    if ((lat_hemi != 'N' && lat_hemi != 'S') || (lon_hemi != 'E' && lon_hemi != 'W')) {
        return std::nullopt;
    }
    try {
        int lat = std::stoi(token.substr(1, 3));
        int lon = std::stoi(token.substr(5, 3));
        return TileId{lat_hemi == 'S' ? -lat : lat, lon_hemi == 'W' ? -lon : lon};
    } catch (const std::exception&) {
        return std::nullopt;
    }
}

inline std::string tile_name(const TileId& id) {
    char buffer[16];
    std::snprintf(buffer, sizeof(buffer), "%c%03d%c%03d", id.lat >= 0 ? 'N' : 'S',
                  std::abs(id.lat), id.lon >= 0 ? 'E' : 'W', std::abs(id.lon));
    return buffer;
}

inline MosaicBounds compute_mosaic_bounds(const std::vector<TileId>& tiles) {
    if (tiles.empty()) throw std::runtime_error("no *_DSM.tif tiles found");
    MosaicBounds bounds{tiles[0].lat, tiles[0].lat + 1, tiles[0].lon, tiles[0].lon + 1};
    for (const auto& tile : tiles) {
        bounds.min_lat = std::min(bounds.min_lat, tile.lat);
        bounds.max_lat = std::max(bounds.max_lat, tile.lat + 1);
        bounds.min_lon = std::min(bounds.min_lon, tile.lon);
        bounds.max_lon = std::max(bounds.max_lon, tile.lon + 1);
    }
    return bounds;
}

inline std::vector<std::vector<int16_t>> split_chunks(const std::vector<int16_t>& grid,
                                                       int cells, int chunks_per_tile) {
    if (cells <= 0 || chunks_per_tile <= 0 || cells % chunks_per_tile != 0 ||
        grid.size() != static_cast<size_t>(cells + 1) * (cells + 1)) {
        throw std::invalid_argument("invalid grid or chunk layout");
    }
    const int nodes = cells + 1;
    const int chunk_cells = cells / chunks_per_tile;
    std::vector<std::vector<int16_t>> chunks;
    chunks.reserve(static_cast<size_t>(chunks_per_tile) * chunks_per_tile);
    for (int cy = 0; cy < chunks_per_tile; ++cy) {
        for (int cx = 0; cx < chunks_per_tile; ++cx) {
            std::vector<int16_t> record(static_cast<size_t>(chunk_cells + 1) * (chunk_cells + 1));
            for (int row = 0; row <= chunk_cells; ++row) {
                const auto begin = grid.begin() +
                    static_cast<size_t>(cy * chunk_cells + row) * nodes + cx * chunk_cells;
                std::copy(begin, begin + chunk_cells + 1,
                          record.begin() + static_cast<size_t>(row) * (chunk_cells + 1));
            }
            chunks.push_back(std::move(record));
        }
    }
    return chunks;
}

} // namespace sim3dview::preprocess
