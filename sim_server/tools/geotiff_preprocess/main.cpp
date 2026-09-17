// GeoTIFF前処理ツール(単発実行CLI)。DETAILED_DESIGN.md 2節・5.5節。
//
// map_data/ 内の *_DSM.tif を機械的に列挙し(DETAILED_DESIGN.md 2.1節)、緯度経度グリッド上の
// ピクセル位置にそのまま敷き詰めてモザイクする。モザイクの外接矩形は**固定値ではなく、
// 実際に見つかったタイルの緯度経度範囲から実行時に自動的に決める**(1.2節・2.3節)。
// そのため map_data/ に別の場所のタイルを追加/削除しても、コード変更なしに追従する。
// データが存在しない領域(モザイクの外接矩形内でタイルが見つからないセル + 各タイル内の
// NODATA画素 + マスクファイル(*_MSK.tif)が海と示す画素)はNaNで埋め、標高0mとは区別する
// (海の色で塗るための目印。1.4節)。タイル内部の海域はDSM側では単に標高0mとして格納されて
// おりNODATAセンチネルでは検出できないため、マスクファイルを別途読んで補っている。
// 再投影は行わない: 入力GeoTIFF(ALOS DSM)は既にEPSG:4326の緯度経度グリッドに
// 1タイル=1度×1度ちょうどで整列しているため、単純に読み取って敷き詰め、
// 平均法でダウンサンプリングするだけでよい(2.2節)。
//
// 使い方: geotiff_preprocess [map_dataディレクトリ] [出力ディレクトリ]
//   既定値はリポジトリルートから実行する前提のパス。

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <optional>
#include <stdexcept>
#include <string>
#include <vector>

#include <gdal_priv.h>

namespace fs = std::filesystem;

namespace {

constexpr int kTargetWidth = 1024;
constexpr int kTargetHeight = 1024;

// GRS80楕円体パラメータ(DETAILED_DESIGN.md 3.2節・metadata.jsonスキーマ)。
constexpr double kEllipsoidA = 6378137.0;
constexpr double kEllipsoidInvF = 298.257222101;

// 試験用デフォルト原点(DETAILED_DESIGN.md 3.1節)。
constexpr double kDefaultOriginLat = 35.355556;
constexpr double kDefaultOriginLon = 138.859722;

// 1タイルあたりの画素数(DETAILED_DESIGN.md 1.2節)。モザイクの外接矩形自体は固定値を
// 持たず、実際に見つかったタイル群から実行時に計算する(下記MosaicBounds参照)。
constexpr int kTileFullPx = 3600; // 1タイル = 1度 × 3600px/度(1秒角)

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

// タイルの海域マスク(例: "ALPSMLC30_N035E135_DSM.tif" に対応する
// "ALPSMLC30_N035E135_MSK.tif")を読み込み、画素値が3(海)の位置をtrueとするビットマップを
// 返す。ALOS World 3D-30mのマスクファイル画素値は 0=有効データ・3=海・4/12=代替データソースで
// 補完(海ではない、代替DEMからの標高値で埋められた陸地)で、map_data/内の全17タイルの実測値
// (QAI.txtのMASK_NUM_SEA等の統計値との突き合わせ)で確認済み。DSM側は海域でも標高0mという
// 「有効に見える」値が入っており(NODATAセンチネルではない)、GetNoDataValue()だけでは
// 検出できないため、マスクファイルを別途読んで補う。マスクファイルが存在しない・サイズが
// 一致しないなど何らかの理由で読めない場合は、警告を出してNODATAベースの判定のみに
// フォールバックする(空のビットマップを返す)。
std::vector<bool> load_sea_mask(const std::string& dsm_path, int expected_w, int expected_h) {
    constexpr const char* kDsmSuffix = "_DSM.tif";
    const size_t pos = dsm_path.rfind(kDsmSuffix);
    if (pos == std::string::npos) {
        return {};
    }
    const std::string msk_path = dsm_path.substr(0, pos) + "_MSK.tif";
    if (!fs::exists(msk_path)) {
        std::cerr << "[geotiff_preprocess] warning: mask file not found, sea detection falls back "
                     "to NODATA only: "
                   << msk_path << std::endl;
        return {};
    }

    GDALDataset* dataset = static_cast<GDALDataset*>(GDALOpen(msk_path.c_str(), GA_ReadOnly));
    if (dataset == nullptr) {
        std::cerr << "[geotiff_preprocess] warning: failed to open mask file: " << msk_path
                   << std::endl;
        return {};
    }
    const int width = dataset->GetRasterXSize();
    const int height = dataset->GetRasterYSize();
    if (width != expected_w || height != expected_h) {
        std::cerr << "[geotiff_preprocess] warning: mask size mismatch (" << width << "x" << height
                   << ", expected " << expected_w << "x" << expected_h << "): " << msk_path
                   << std::endl;
        GDALClose(dataset);
        return {};
    }
    GDALRasterBand* band = dataset->GetRasterBand(1);
    if (band == nullptr) {
        GDALClose(dataset);
        return {};
    }
    std::vector<uint8_t> raw(static_cast<size_t>(width) * static_cast<size_t>(height));
    const CPLErr err = band->RasterIO(GF_Read, 0, 0, width, height, raw.data(), width, height,
                                       GDT_Byte, 0, 0);
    GDALClose(dataset);
    if (err != CE_None) {
        std::cerr << "[geotiff_preprocess] warning: failed to read mask data: " << msk_path
                   << std::endl;
        return {};
    }

    constexpr uint8_t kSeaMaskValue = 3;
    std::vector<bool> is_sea(raw.size());
    for (size_t i = 0; i < raw.size(); ++i) {
        is_sea[i] = (raw[i] == kSeaMaskValue);
    }
    return is_sea;
}

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
    // タイル内のNODATA画素(主に海域)をNaNへ置き換える。海の色で塗る目印として使う
    // (build_mosaic/downsample_averageもNaNを「データなし」として扱う)。
    int has_nodata = 0;
    const double nodata_value = band->GetNoDataValue(&has_nodata);
    GDALClose(dataset);
    if (err != CE_None) {
        throw std::runtime_error("failed to read raster data: " + path);
    }
    if (has_nodata) {
        const float nodata_f = static_cast<float>(nodata_value);
        for (float& v : elevation) {
            if (v == nodata_f) {
                v = std::numeric_limits<float>::quiet_NaN();
            }
        }
    }

    // マスクファイル(*_MSK.tif)ベースの海域検出で追加で補う(NODATAセンチネルでは
    // 検出できない、タイル内部の海域=標高0mだが有効データに見える画素への対応)。
    const std::vector<bool> sea_mask = load_sea_mask(path, width, height);
    if (!sea_mask.empty()) {
        for (size_t i = 0; i < elevation.size(); ++i) {
            if (sea_mask[i]) {
                elevation[i] = std::numeric_limits<float>::quiet_NaN();
            }
        }
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

// 見つかったタイル1枚分(タイルID + ファイルパス)。
struct DiscoveredTile {
    TileId id;
    fs::path path;
};

// map_dataディレクトリ内の*_DSM.tifを機械的に列挙する(DETAILED_DESIGN.md 2.1節)。
// この時点ではファイル名からタイルIDを読むだけで、GDALでの読み込みはまだ行わない
// (後段のcompute_mosaic_bounds()が先にモザイク全体のサイズを決める必要があるため)。
std::vector<DiscoveredTile> discover_tiles(const std::string& map_data_dir) {
    std::vector<DiscoveredTile> tiles;
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
        tiles.push_back(DiscoveredTile{*tile_id, entry.path()});
    }
    return tiles;
}

// モザイクの外接矩形(整数度)。タイルの南西角IDの最小/最大から、実際に見つかったタイル群
// 全体を覆う範囲を求める(1.2節)。固定のハードコード値は持たない: map_data/に別の場所の
// タイルが追加/削除されても、この関数の結果だけが変わりモザイクが自動的に追従する。
struct MosaicBounds {
    int min_lat = 0;
    int max_lat = 0; // 外接矩形の北端(タイル南西角緯度の最大値+1)
    int min_lon = 0;
    int max_lon = 0; // 外接矩形の東端(タイル南西角経度の最大値+1)
};

MosaicBounds compute_mosaic_bounds(const std::vector<DiscoveredTile>& tiles) {
    if (tiles.empty()) {
        throw std::runtime_error("no *_DSM.tif tiles found");
    }
    MosaicBounds bounds{tiles[0].id.lat, tiles[0].id.lat + 1, tiles[0].id.lon, tiles[0].id.lon + 1};
    for (const auto& tile : tiles) {
        bounds.min_lat = std::min(bounds.min_lat, tile.id.lat);
        bounds.max_lat = std::max(bounds.max_lat, tile.id.lat + 1);
        bounds.min_lon = std::min(bounds.min_lon, tile.id.lon);
        bounds.max_lon = std::max(bounds.max_lon, tile.id.lon + 1);
    }
    return bounds;
}

// 見つかったタイル群を、computed_mosaic_bounds()が決めた外接矩形のcanvas上に敷き詰める
// (DETAILED_DESIGN.md 2.3節)。タイルが存在しないセル・各タイル内のNODATA画素はNaNのまま
// (「データなし」=海域の目印。0mで埋めると実際の海抜0m付近の陸地と区別がつかなくなるため)。
std::vector<float> build_mosaic(const std::vector<DiscoveredTile>& tiles,
                                 const MosaicBounds& bounds, int mosaic_w, int mosaic_h) {
    std::vector<float> canvas(static_cast<size_t>(mosaic_w) * static_cast<size_t>(mosaic_h),
                               std::numeric_limits<float>::quiet_NaN());

    int tiles_found = 0;
    for (const auto& tile : tiles) {
        const std::string filename = tile.path.filename().string();
        std::cout << "[geotiff_preprocess] loading tile: " << filename << std::endl;
        const SourceGrid grid = load_dsm_tile(tile.path.string());
        if (grid.width != kTileFullPx || grid.height != kTileFullPx) {
            std::cerr << "[geotiff_preprocess] skip (unexpected size " << grid.width << "x"
                       << grid.height << ", expected " << kTileFullPx << "x" << kTileFullPx
                       << "): " << filename << std::endl;
            continue;
        }

        // タイル南西角(tile.id)から、モザイクcanvas上の配置位置(北→南、西→東)を求める。
        const int row_offset = (bounds.max_lat - (tile.id.lat + 1)) * kTileFullPx;
        const int col_offset = (tile.id.lon - bounds.min_lon) * kTileFullPx;

        for (int ty = 0; ty < kTileFullPx; ++ty) {
            const float* src_row = grid.elevation.data() + static_cast<size_t>(ty) * kTileFullPx;
            float* dst_row = canvas.data() +
                              static_cast<size_t>(row_offset + ty) * static_cast<size_t>(mosaic_w) +
                              static_cast<size_t>(col_offset);
            std::copy(src_row, src_row + kTileFullPx, dst_row);
        }
        ++tiles_found;
    }

    std::cout << "[geotiff_preprocess] composited " << tiles_found << " tile(s) into "
               << mosaic_w << "x" << mosaic_h << " mosaic (missing cells left as NaN = ocean)"
               << std::endl;
    return canvas;
}

// 平均法(average)でダウンサンプリングする(DETAILED_DESIGN.md 2.4節)。行順は入力と同じ(北→南)。
// NaN(データなし=海域、load_dsm_tile/build_mosaic参照)は平均から除外する。ブロック内に
// 実データが1画素でもあればその平均を採用し、ブロック全体がNaNの場合のみ出力もNaN(海)にする
// (陸地の縁で実データを最大限活かすため)。
std::vector<float> downsample_average(const std::vector<float>& src, int src_w, int src_h,
                                       int dst_w, int dst_h) {
    std::vector<float> dst(static_cast<size_t>(dst_w) * static_cast<size_t>(dst_h),
                            std::numeric_limits<float>::quiet_NaN());

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
                    const float v = src[row_offset + static_cast<size_t>(sx)];
                    if (!std::isnan(v)) {
                        sum += v;
                        ++count;
                    }
                }
            }
            dst[static_cast<size_t>(dy) * static_cast<size_t>(dst_w) + static_cast<size_t>(dx)] =
                count > 0 ? static_cast<float>(sum / static_cast<double>(count))
                          : std::numeric_limits<float>::quiet_NaN();
        }
    }
    return dst;
}

// 行順をGDAL標準(北→南、row0=max_lat)からDETAILED_DESIGN.md 2.6節の座標復元式が前提とする
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

// DETAILED_DESIGN.md 2.6節のスキーマに従ってmetadata.jsonを書き出す。
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
        const std::vector<DiscoveredTile> tiles = discover_tiles(map_data_dir);
        const MosaicBounds mosaic_bounds = compute_mosaic_bounds(tiles);
        const int mosaic_w = (mosaic_bounds.max_lon - mosaic_bounds.min_lon) * kTileFullPx;
        const int mosaic_h = (mosaic_bounds.max_lat - mosaic_bounds.min_lat) * kTileFullPx;
        std::cout << "[geotiff_preprocess] found " << tiles.size() << " tile(s), mosaic bounds: "
                   << "lat " << mosaic_bounds.min_lat << ".." << mosaic_bounds.max_lat << ", lon "
                   << mosaic_bounds.min_lon << ".." << mosaic_bounds.max_lon << " (" << mosaic_w
                   << "x" << mosaic_h << "px)" << std::endl;

        std::vector<float> mosaic = build_mosaic(tiles, mosaic_bounds, mosaic_w, mosaic_h);

        std::cout << "[geotiff_preprocess] downsampling to " << kTargetWidth << "x"
                   << kTargetHeight << " (average)" << std::endl;
        const std::vector<float> downsampled =
            downsample_average(mosaic, mosaic_w, mosaic_h, kTargetWidth, kTargetHeight);
        // 巨大な中間canvas(モザイク全体)はもう不要なので明示的に解放する。
        mosaic.clear();
        mosaic.shrink_to_fit();

        const std::vector<float> output =
            flip_rows_north_to_south(downsampled, kTargetWidth, kTargetHeight);

        // std::minmax_elementはNaNを正しく除外できない(NaNとの比較は常にfalseになるため)。
        // 海(NaN)を除いた実データだけでmin/maxを求める。
        float elevation_min = std::numeric_limits<float>::infinity();
        float elevation_max = -std::numeric_limits<float>::infinity();
        for (float v : output) {
            if (std::isnan(v)) {
                continue;
            }
            elevation_min = std::min(elevation_min, v);
            elevation_max = std::max(elevation_max, v);
        }

        const GeodeticBounds bounds{
            static_cast<double>(mosaic_bounds.min_lat), static_cast<double>(mosaic_bounds.max_lat),
            static_cast<double>(mosaic_bounds.min_lon), static_cast<double>(mosaic_bounds.max_lon)};

        const fs::path out_dir(output_dir);
        fs::create_directories(out_dir);

        write_heightmap_bin(out_dir / "heightmap.bin", output);
        write_metadata_json(out_dir / "metadata.json", kTargetWidth, kTargetHeight, elevation_min,
                             elevation_max, bounds);

        std::cout << "[geotiff_preprocess] wrote " << (out_dir / "heightmap.bin").string()
                   << " and " << (out_dir / "metadata.json").string() << std::endl;
        std::cout << "[geotiff_preprocess] elevation range: " << elevation_min << " .. "
                   << elevation_max << std::endl;
    } catch (const std::exception& e) {
        std::cerr << "[geotiff_preprocess] error: " << e.what() << std::endl;
        return 1;
    }

    return 0;
}
