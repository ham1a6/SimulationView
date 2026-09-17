// GeoTIFF前処理ツール(単発実行CLI)。DESIGN.md 2節・7節。
//
// map_data/ 内の *_DSM.tif を機械的に列挙し(DESIGN.md 2.1節)、緯度経度グリッド上の
// ピクセル位置にそのまま敷き詰めてモザイクする(北緯35〜40度・東経135〜140度、
// 18000×18000px)。実在しない8タイル分は標高0mで埋める(1.4節・2.3節)。
// 再投影は行わない: 入力GeoTIFF(ALOS DSM)は既にEPSG:4326の緯度経度グリッドに
// 1タイル=1度×1度ちょうどで整列しているため、単純に読み取って敷き詰め、
// 平均法でダウンサンプリングするだけでよい(2.2節)。
//
// 使い方: geotiff_preprocess [map_dataディレクトリ] [出力ディレクトリ]
//   既定値はリポジトリルートから実行する前提のパス。

#include <algorithm>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <optional>
#include <stdexcept>
#include <string>
#include <vector>

#include <gdal_priv.h>

namespace fs = std::filesystem;

namespace {

constexpr int kTargetWidth = 1024;
constexpr int kTargetHeight = 1024;

// GRS80楕円体パラメータ(DESIGN.md 3.2節・metadata.jsonスキーマ)。
constexpr double kEllipsoidA = 6378137.0;
constexpr double kEllipsoidInvF = 298.257222101;

// 試験用デフォルト原点(DESIGN.md 3.1節)。
constexpr double kDefaultOriginLat = 35.355556;
constexpr double kDefaultOriginLon = 138.859722;

// モザイク対象の外接矩形(DESIGN.md 1.2節・2.3節)。
constexpr int kTileFullPx = 3600; // 1タイル = 1度 × 3600px/度(1秒角)
constexpr int kMosaicMinLat = 35;
constexpr int kMosaicMaxLat = 40;
constexpr int kMosaicMinLon = 135;
constexpr int kMosaicMaxLon = 140;
constexpr int kMosaicWidth = (kMosaicMaxLon - kMosaicMinLon) * kTileFullPx;  // 18000
constexpr int kMosaicHeight = (kMosaicMaxLat - kMosaicMinLat) * kTileFullPx; // 18000

struct GeodeticBounds {
    double min_lat = 0.0;
    double max_lat = 0.0;
    double min_lon = 0.0;
    double max_lon = 0.0;
};

// GDALで読み込んだ1タイル分の標高グリッド。
// GDALの標準に従い row 0 = 北端(max_lat)、row(height-1) = 南端付近、で格納される。
struct SourceGrid {
    int width = 0;
    int height = 0;
    std::vector<float> elevation; // row-major, GDAL標準順(北→南)
    GeodeticBounds bounds;
};

// GeoTIFF(DSM)を1枚読み込む。再投影は行わない。
SourceGrid load_dsm_tile(const std::string& path) {
    GDALDataset* dataset = static_cast<GDALDataset*>(GDALOpen(path.c_str(), GA_ReadOnly));
    if (dataset == nullptr) {
        throw std::runtime_error("failed to open GeoTIFF: " + path);
    }

    const int width = dataset->GetRasterXSize();
    const int height = dataset->GetRasterYSize();

    double geo_transform[6];
    if (dataset->GetGeoTransform(geo_transform) != CE_None) {
        GDALClose(dataset);
        throw std::runtime_error("GeoTIFF has no geotransform: " + path);
    }
    // geo_transform: [0]=左上X(経度), [1]=画素幅(度/px),
    //                [3]=左上Y(緯度), [5]=画素高さ(度/px、北上図では負値)
    const double min_lon = geo_transform[0];
    const double max_lat = geo_transform[3];
    const double max_lon = min_lon + geo_transform[1] * width;
    const double min_lat = max_lat + geo_transform[5] * height;

    GDALRasterBand* band = dataset->GetRasterBand(1);
    if (band == nullptr) {
        GDALClose(dataset);
        throw std::runtime_error("GeoTIFF has no raster band: " + path);
    }

    std::vector<float> elevation(static_cast<size_t>(width) * static_cast<size_t>(height));
    // GDT_Float32を指定することで、元データが16bit整数でもGDALが自動的にfloatへ変換して読む。
    const CPLErr err = band->RasterIO(GF_Read, 0, 0, width, height, elevation.data(), width,
                                       height, GDT_Float32, 0, 0);
    GDALClose(dataset);
    if (err != CE_None) {
        throw std::runtime_error("failed to read raster data: " + path);
    }

    SourceGrid grid;
    grid.width = width;
    grid.height = height;
    grid.elevation = std::move(elevation);
    grid.bounds = GeodeticBounds{min_lat, max_lat, min_lon, max_lon};
    return grid;
}

struct TileId {
    int lat = 0; // タイル南西角の緯度(整数度)
    int lon = 0; // タイル南西角の経度(整数度)
};

// ファイル名からタイルIDを取り出す。例: "ALPSMLC30_N035E138_DSM.tif" -> lat=35, lon=138。
std::optional<TileId> parse_tile_filename(const std::string& filename) {
    const size_t suffix_pos = filename.find("_DSM.tif");
    if (suffix_pos == std::string::npos || suffix_pos < 8) {
        return std::nullopt;
    }
    // "_DSM.tif" の直前8文字が "N035E138" のような形式のはず。
    const std::string token = filename.substr(suffix_pos - 8, 8);
    const char lat_hemi = token[0];
    const char lon_hemi = token[4];
    if ((lat_hemi != 'N' && lat_hemi != 'S') || (lon_hemi != 'E' && lon_hemi != 'W')) {
        return std::nullopt;
    }
    try {
        int lat = std::stoi(token.substr(1, 3));
        int lon = std::stoi(token.substr(5, 3));
        if (lat_hemi == 'S') {
            lat = -lat;
        }
        if (lon_hemi == 'W') {
            lon = -lon;
        }
        return TileId{lat, lon};
    } catch (const std::exception&) {
        return std::nullopt;
    }
}

// map_dataディレクトリ内の*_DSM.tifを機械的に列挙してモザイクする(DESIGN.md 2.1節・2.3節)。
// 見つからないタイル分は標高0mのまま(呼び出し前にcanvasを0初期化しておくこと)。
std::vector<float> build_mosaic(const std::string& map_data_dir) {
    std::vector<float> canvas(static_cast<size_t>(kMosaicWidth) * static_cast<size_t>(kMosaicHeight),
                               0.0f);

    int tiles_found = 0;
    for (const auto& entry : fs::directory_iterator(map_data_dir)) {
        if (!entry.is_regular_file()) {
            continue;
        }
        const std::string filename = entry.path().filename().string();
        if (filename.find("_DSM.tif") == std::string::npos) {
            continue;
        }

        const auto tile_id = parse_tile_filename(filename);
        if (!tile_id) {
            std::cerr << "[geotiff_preprocess] skip (cannot parse tile id): " << filename
                       << std::endl;
            continue;
        }
        if (tile_id->lat < kMosaicMinLat || tile_id->lat >= kMosaicMaxLat ||
            tile_id->lon < kMosaicMinLon || tile_id->lon >= kMosaicMaxLon) {
            std::cerr << "[geotiff_preprocess] skip (outside mosaic bounds): " << filename
                       << std::endl;
            continue;
        }

        std::cout << "[geotiff_preprocess] loading tile: " << filename << std::endl;
        const SourceGrid grid = load_dsm_tile(entry.path().string());
        if (grid.width != kTileFullPx || grid.height != kTileFullPx) {
            std::cerr << "[geotiff_preprocess] skip (unexpected size " << grid.width << "x"
                       << grid.height << ", expected " << kTileFullPx << "x" << kTileFullPx
                       << "): " << filename << std::endl;
            continue;
        }

        // タイル南西角(tile_id)から、モザイクcanvas上の配置位置(北→南、西→東)を求める。
        const int row_offset = (kMosaicMaxLat - (tile_id->lat + 1)) * kTileFullPx;
        const int col_offset = (tile_id->lon - kMosaicMinLon) * kTileFullPx;

        for (int ty = 0; ty < kTileFullPx; ++ty) {
            const float* src_row = grid.elevation.data() + static_cast<size_t>(ty) * kTileFullPx;
            float* dst_row = canvas.data() +
                              static_cast<size_t>(row_offset + ty) * static_cast<size_t>(kMosaicWidth) +
                              static_cast<size_t>(col_offset);
            std::copy(src_row, src_row + kTileFullPx, dst_row);
        }
        ++tiles_found;
    }

    std::cout << "[geotiff_preprocess] composited " << tiles_found << " tile(s) into "
               << kMosaicWidth << "x" << kMosaicHeight << " mosaic (missing tiles filled with 0m)"
               << std::endl;
    return canvas;
}

// 平均法(average)でダウンサンプリングする(DESIGN.md 2.4節)。行順は入力と同じ(北→南)。
std::vector<float> downsample_average(const std::vector<float>& src, int src_w, int src_h,
                                       int dst_w, int dst_h) {
    std::vector<float> dst(static_cast<size_t>(dst_w) * static_cast<size_t>(dst_h), 0.0f);

    for (int dy = 0; dy < dst_h; ++dy) {
        const int sy0 = static_cast<int>(static_cast<int64_t>(dy) * src_h / dst_h);
        const int sy1 =
            std::max(sy0 + 1, static_cast<int>(static_cast<int64_t>(dy + 1) * src_h / dst_h));
        for (int dx = 0; dx < dst_w; ++dx) {
            const int sx0 = static_cast<int>(static_cast<int64_t>(dx) * src_w / dst_w);
            const int sx1 =
                std::max(sx0 + 1, static_cast<int>(static_cast<int64_t>(dx + 1) * src_w / dst_w));

            double sum = 0.0;
            int64_t count = 0;
            for (int sy = sy0; sy < sy1 && sy < src_h; ++sy) {
                const size_t row_offset = static_cast<size_t>(sy) * static_cast<size_t>(src_w);
                for (int sx = sx0; sx < sx1 && sx < src_w; ++sx) {
                    sum += src[row_offset + static_cast<size_t>(sx)];
                    ++count;
                }
            }
            dst[static_cast<size_t>(dy) * static_cast<size_t>(dst_w) + static_cast<size_t>(dx)] =
                count > 0 ? static_cast<float>(sum / static_cast<double>(count)) : 0.0f;
        }
    }
    return dst;
}

// 行順をGDAL標準(北→南、row0=max_lat)からDESIGN.md 2.6節の座標復元式が前提とする
// 順序(南→北、row0=min_lat: `lat = min_lat + (j/(height-1))*(max_lat-min_lat)`)へ反転する。
// これを行わないと、フロント側で地形が南北反転して描画されてしまう。
std::vector<float> flip_rows_north_to_south(const std::vector<float>& src, int w, int h) {
    std::vector<float> dst(src.size());
    for (int y = 0; y < h; ++y) {
        const auto begin = src.begin() + static_cast<std::ptrdiff_t>(y) * w;
        std::copy(begin, begin + w,
                  dst.begin() + static_cast<std::ptrdiff_t>(h - 1 - y) * w);
    }
    return dst;
}

void write_heightmap_bin(const fs::path& path, const std::vector<float>& data) {
    std::ofstream out(path, std::ios::binary);
    if (!out) {
        throw std::runtime_error("failed to open output file: " + path.string());
    }
    out.write(reinterpret_cast<const char*>(data.data()),
              static_cast<std::streamsize>(data.size() * sizeof(float)));
}

// DESIGN.md 2.6節のスキーマに従ってmetadata.jsonを書き出す。
// フィールド構成が固定・少数なので、JSONライブラリは使わず手書きで組み立てる。
void write_metadata_json(const fs::path& path, int width, int height, float elevation_min,
                          float elevation_max, const GeodeticBounds& bounds) {
    std::ofstream out(path);
    if (!out) {
        throw std::runtime_error("failed to open output file: " + path.string());
    }
    // ostreamの既定精度(有効6桁)だと ellipsoid.a_m や default_origin の緯度経度が
    // 途中で丸められてしまう(例: 138.859722 -> 138.86)ため、明示的に精度を上げておく。
    out << std::setprecision(15);
    out << "{\n"
        << "  \"width\": " << width << ",\n"
        << "  \"height\": " << height << ",\n"
        << "  \"elevation_min\": " << elevation_min << ",\n"
        << "  \"elevation_max\": " << elevation_max << ",\n"
        << "  \"geodetic_bounds\": {\n"
        << "    \"min_lat\": " << bounds.min_lat << ",\n"
        << "    \"max_lat\": " << bounds.max_lat << ",\n"
        << "    \"min_lon\": " << bounds.min_lon << ",\n"
        << "    \"max_lon\": " << bounds.max_lon << "\n"
        << "  },\n"
        << "  \"source_crs\": \"EPSG:4326 (WGS84相当, GRS80楕円体)\",\n"
        << "  \"height_datum\": \"orthometric height (EGM96 geoid)\",\n"
        << "  \"ellipsoid\": { \"a_m\": " << kEllipsoidA << ", \"inv_f\": " << kEllipsoidInvF
        << " },\n"
        << "  \"has_texture\": false,\n"
        << "  \"default_origin\": {\n"
        << "    \"lat_dms\": \"35\\u00b021'20\\\"N\",\n"
        << "    \"lon_dms\": \"138\\u00b051'35\\\"E\",\n"
        << "    \"lat_deg\": " << kDefaultOriginLat << ",\n"
        << "    \"lon_deg\": " << kDefaultOriginLon << "\n"
        << "  }\n"
        << "}\n";
}

} // namespace

int main(int argc, char** argv) {
    std::string map_data_dir = "map_data";
    std::string output_dir = "sim_server/assets/terrain";
    if (argc > 1) {
        map_data_dir = argv[1];
    }
    if (argc > 2) {
        output_dir = argv[2];
    }

    GDALAllRegister();

    try {
        std::cout << "[geotiff_preprocess] scanning: " << map_data_dir << std::endl;
        std::vector<float> mosaic = build_mosaic(map_data_dir);

        std::cout << "[geotiff_preprocess] downsampling to " << kTargetWidth << "x"
                   << kTargetHeight << " (average)" << std::endl;
        const std::vector<float> downsampled =
            downsample_average(mosaic, kMosaicWidth, kMosaicHeight, kTargetWidth, kTargetHeight);
        // 巨大な中間canvas(18000x18000)はもう不要なので明示的に解放する。
        mosaic.clear();
        mosaic.shrink_to_fit();

        const std::vector<float> output =
            flip_rows_north_to_south(downsampled, kTargetWidth, kTargetHeight);

        const auto [min_it, max_it] = std::minmax_element(output.begin(), output.end());

        const GeodeticBounds bounds{kMosaicMinLat, kMosaicMaxLat, kMosaicMinLon, kMosaicMaxLon};

        const fs::path out_dir(output_dir);
        fs::create_directories(out_dir);

        write_heightmap_bin(out_dir / "heightmap.bin", output);
        write_metadata_json(out_dir / "metadata.json", kTargetWidth, kTargetHeight, *min_it,
                             *max_it, bounds);

        std::cout << "[geotiff_preprocess] wrote " << (out_dir / "heightmap.bin").string()
                   << " and " << (out_dir / "metadata.json").string() << std::endl;
        std::cout << "[geotiff_preprocess] elevation range: " << *min_it << " .. " << *max_it
                   << std::endl;
    } catch (const std::exception& e) {
        std::cerr << "[geotiff_preprocess] error: " << e.what() << std::endl;
        return 1;
    }

    return 0;
}
