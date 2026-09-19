// GeoTIFF前処理ツール(単発実行CLI)。DETAILED_DESIGN.md 2節・5.5節。
//
// map_data/ 内の *_DSM.tif(ALOS World 3D-30m、1タイル=1度x1度=3600x3600画素)を機械的に列挙し
// (DETAILED_DESIGN.md 2.1節)、**タイルごとに複数の解像度レベルの標高グリッド**を書き出す
// (地形LOD。フロントはカメラに近いタイルだけ細かいレベルを取得する)。以前は全タイルを1枚の
// モザイクに敷き詰めて2048x2048へ縮小していたが、対象域が広がる(現在は30度四方・390タイル)と
// 1セル約1.6kmまで粗くなり、メモリ(全域モザイクは約46GB)も限界だったため、タイル単位の
// ストリーミング処理に変更した(1タイルずつ読み込むので、メモリは数百MB程度で済む)。
//
// データが存在しない領域(タイルが無い/各タイル内のNODATA画素のうち埋められなかったもの/
// マスクファイル(*_MSK.tif)が海と示す画素)はNaN(出力上はint16の最小値)にして標高0mとは
// 区別する(1.4節)。タイル内部の海域はDSM側では単に標高0mとして格納されておりNODATA
// センチネルでは検出できないため、マスクファイルを別途読んで補っている。タイル内のNODATA画素の
// うち、海ではなくかつ小さい(kMaxFillableHolePixels以下の)ものは、周囲の有効画素から補間して
// 埋める(雲の影・センサー欠損等を想定)。
// 再投影は行わない: 入力GeoTIFFは既にEPSG:4326の緯度経度グリッドに1タイル=1度x1度ちょうどで
// 整列しているため、単純に読み取って平均法で縮小するだけでよい(2.2節)。
//
// 出力(出力ディレクトリ直下):
//   metadata.json            ... 全体の範囲・標高の最小最大・楕円体・解像度レベル・チャンク分割数
//   tile_index.json          ... 存在するタイルの一覧(緯度経度・標高範囲)
//   base.bin                 ... レベル0(最粗、タイル全体で1枚)を全タイル分連結したもの(tile_index.jsonの順)
//   tiles/L{k}/N035E138.bin  ... レベルk(1以上)のタイル別ファイル(下記のチャンク単位のグリッドを連結)
// グリッドは(N+1)x(N+1)ノード(Nは一辺のセル数)のint16(メートル、リトルエンディアン)で、
// 行は南→北・列は西→東のrow-major、NaN(データなし)はint16の最小値(-32768)。
// ノード(i,j)は1度タイル内の(i/N, j/N)の位置にあり、値はそのノードを中心とする
// 1セル幅の窓内の有効画素の平均(窓内に有効画素が1つでもあれば陸、全部NaNなら海)。
//
// レベル1以上は、1度タイルを`kChunksPerTile`x`kChunksPerTile`のチャンクに分け、チャンクごとに
// (N/kChunksPerTile+1)^2ノードのグリッドを持つ(隣のチャンクと縁のノードを共有するので、同じ
// レベルのチャンク同士は縁が一致する)。ファイルには、チャンクを行(南→北)・列(西→東)の順に
// 固定サイズのレコードとして連結する(フロントはHTTP Rangeで必要なチャンクだけ取得できる)。
// これは最細レベルを元データの解像度(30m)にするため: 1度タイル全体の30mグリッドは約1300万頂点で
// GPUバッファの上限を超えるので、カメラのすぐ近くのチャンクだけを細かくする。
//
// 使い方: geotiff_preprocess [map_dataディレクトリ] [出力ディレクトリ]
//   既定値はリポジトリルートから実行する前提のパス。

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <mutex>
#include <optional>
#include <queue>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

#include <gdal_priv.h>

namespace fs = std::filesystem;

namespace {

// 解像度レベル(1度タイル1枚の一辺を何セルに分割するか)。レベル0はフロントが全タイルを常時
// 保持する最粗のベース(約1.85km/セル)、以降ほど細かい(約620m/185m/62m/31m)。最細の3600は
// 元データ(ALOS 30m、1度=3600画素)の解像度そのもの。フロントはカメラに近いチャンクにだけ
// 細かいレベルを取得する。各値は`kChunksPerTile`で割り切れること(レベル1以上のチャンク分割のため)。
// 値を変えたらフロント側のレベル情報(metadata.jsonのtile_levels)は自動で追従する。
constexpr int kLevelCells[] = {60, 180, 600, 1800, 3600};
constexpr int kNumLevels = static_cast<int>(sizeof(kLevelCells) / sizeof(kLevelCells[0]));
// 1度タイルを何x何のチャンクに分けるか(レベル1以上)。1チャンクは緯度経度とも1/6度(約18km x 15km)。
constexpr int kChunksPerTile = 6;
constexpr int16_t kNoDataInt16 = std::numeric_limits<int16_t>::min();

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

// 「小さなNODATA穴」とみなす連結成分の最大画素数。これを超える(=大きすぎる)穴は
// 周囲からの補間対象にせず、従来通りNODATA(最終的にNaN=データなし)のまま残す。
// 2000画素は、ネイティブ解像度(1px=1秒角≒約30m)で半径25px前後(直径約1.5km)の
// 円に相当し、雲の影・センサーの局所的な欠損程度の「小さな穴」を想定した値。
// タイル自体が丸ごと存在しない欠損(1タイル=3600×3600=1296万画素)とは
// 桁違いに小さく、意図せず大きな空白を埋めてしまうことはない。
constexpr int kMaxFillableHolePixels = 2000;

// 「小さなNODATA穴」(海ではない、周囲を有効な陸地画素に囲まれた小さな欠損領域)を、
// 周囲の有効画素の平均値で埋める。海(`is_sea`)は補間の材料にも対象にもしない
// (海は「データが欠損している」のではなく「実際に海である」ため、要望により対象外にした)。
// タイル自体が丸ごと存在しない大きな欠損は、このタイル内補間の対象外(タイルが
// 出力されず、フロントでは海と同じ「データなし」として扱われる)。
//
// アルゴリズム: 8連結で穴画素を連結成分に分け、`kMaxFillableHolePixels`を超える成分は
// 埋めずスキップする。埋める成分は、穴の境界(有効画素に隣接する穴画素)からBFSで内側へ
// 波及させながら、その時点で確定済み(有効、または既に埋め済み)の8近傍画素の平均値で
// 順に埋めていく。これにより、常に実データ(または実データから補間済みの値)だけを
// 材料にでき、穴の内部からいきなり値を作ることがない。
// 戻り値: 実際に埋めた画素数。
int fill_small_nodata_holes(std::vector<float>& elevation, int width, int height,
                             const std::vector<bool>& hole_candidate,
                             const std::vector<bool>& is_sea) {
    const size_t n = elevation.size();
    std::vector<int> component_id(n, -1);
    int filled_total = 0;

    const int neighbor_dy[8] = {-1, -1, -1, 0, 0, 1, 1, 1};
    const int neighbor_dx[8] = {-1, 0, 1, -1, 1, -1, 0, 1};

    for (size_t start = 0; start < n; ++start) {
        if (!hole_candidate[start] || component_id[start] != -1) {
            continue;
        }
        // 8連結で連結成分を収集する(単純なスタックDFS)。
        std::vector<int> members;
        std::vector<int> stack{static_cast<int>(start)};
        component_id[start] = 0; // 訪問済みの目印(値自体は使わない)
        while (!stack.empty()) {
            const int cur = stack.back();
            stack.pop_back();
            members.push_back(cur);
            const int cy = cur / width;
            const int cx = cur % width;
            for (int k = 0; k < 8; ++k) {
                const int ny = cy + neighbor_dy[k];
                const int nx = cx + neighbor_dx[k];
                if (ny < 0 || ny >= height || nx < 0 || nx >= width) {
                    continue;
                }
                const int ni = ny * width + nx;
                if (hole_candidate[ni] && component_id[ni] == -1) {
                    component_id[ni] = 0;
                    stack.push_back(ni);
                }
            }
        }

        if (members.size() > static_cast<size_t>(kMaxFillableHolePixels)) {
            continue; // 大きすぎる穴は対象外(NODATAのまま残す)。
        }

        std::vector<bool> in_component(n, false);
        for (int i : members) {
            in_component[i] = true;
        }
        std::vector<bool> filled(n, false);

        // 有効な(穴でも海でもない)近傍を持つ穴画素を、その平均値で埋めてBFSの種にする。
        auto try_fill = [&](int i) -> bool {
            const int y = i / width;
            const int x = i % width;
            double sum = 0.0;
            int count = 0;
            for (int k = 0; k < 8; ++k) {
                const int ny = y + neighbor_dy[k];
                const int nx = x + neighbor_dx[k];
                if (ny < 0 || ny >= height || nx < 0 || nx >= width) {
                    continue;
                }
                const int ni = ny * width + nx;
                if (is_sea[ni]) {
                    continue; // 海は補間材料にしない。
                }
                if (in_component[ni] && !filled[ni]) {
                    continue; // まだ埋まっていない穴画素は使わない。
                }
                sum += elevation[ni];
                ++count;
            }
            if (count == 0) {
                return false;
            }
            elevation[i] = static_cast<float>(sum / count);
            filled[i] = true;
            return true;
        };

        std::queue<int> queue;
        for (int i : members) {
            if (try_fill(i)) {
                queue.push(i);
                ++filled_total;
            }
        }
        while (!queue.empty()) {
            const int cur = queue.front();
            queue.pop();
            const int cy = cur / width;
            const int cx = cur % width;
            for (int k = 0; k < 8; ++k) {
                const int ny = cy + neighbor_dy[k];
                const int nx = cx + neighbor_dx[k];
                if (ny < 0 || ny >= height || nx < 0 || nx >= width) {
                    continue;
                }
                const int ni = ny * width + nx;
                if (in_component[ni] && !filled[ni]) {
                    if (try_fill(ni)) {
                        queue.push(ni);
                        ++filled_total;
                    }
                }
            }
        }
        // filledのまま残った成分内画素(周囲が全て他の穴・海で、有効画素に到達できない
        // 場合。8連結で成分を作っている以上理論上起こらないはずだが、念のため何もしない
        // =NODATAのまま残す)。
    }
    return filled_total;
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
    int has_nodata = 0;
    const double nodata_value = band->GetNoDataValue(&has_nodata);
    GDALClose(dataset);
    if (err != CE_None) {
        throw std::runtime_error("failed to read raster data: " + path);
    }

    // マスクファイル(*_MSK.tif)ベースの海域検出(NODATAセンチネルでは検出できない、
    // タイル内部の海域=標高0mだが有効データに見える画素への対応)。
    const std::vector<bool> sea_mask = load_sea_mask(path, width, height);
    const std::vector<bool> is_sea =
        sea_mask.empty() ? std::vector<bool>(elevation.size(), false) : sea_mask;

    // NODATA画素のうち、海ではないもの(=雲の影・センサー欠損等による「小さな穴」の候補)を
    // 周囲の有効画素から補間して埋める(要望により追加。海は対象外、埋めた後もNODATAの
    // ままの画素だけを次のステップでNaNにする)。
    if (has_nodata) {
        const float nodata_f = static_cast<float>(nodata_value);
        std::vector<bool> hole_candidate(elevation.size(), false);
        for (size_t i = 0; i < elevation.size(); ++i) {
            if (elevation[i] == nodata_f && !is_sea[i]) {
                hole_candidate[i] = true;
            }
        }
        const int filled = fill_small_nodata_holes(elevation, width, height, hole_candidate, is_sea);
        if (filled > 0) {
            std::cout << "[geotiff_preprocess] filled " << filled
                       << " small NODATA hole pixel(s) in " << fs::path(path).filename().string()
                       << std::endl;
        }
    }

    // タイル内に残ったNODATA画素(大きすぎて埋められなかった穴を含む)をNaNへ置き換える。
    // 「データなし」の目印として使う(レベルグリッドの平均計算もNaNを除外する)。
    if (has_nodata) {
        const float nodata_f = static_cast<float>(nodata_value);
        for (float& v : elevation) {
            if (v == nodata_f) {
                v = std::numeric_limits<float>::quiet_NaN();
            }
        }
    }

    // 海は(補間の材料にはしたが)引き続きNaNにする。海は「データが欠損している」のではなく
    // 「実際に海である」ため、周囲から補間して陸地にすることはしない。
    for (size_t i = 0; i < elevation.size(); ++i) {
        if (is_sea[i]) {
            elevation[i] = std::numeric_limits<float>::quiet_NaN();
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

// タイルID(南西角の整数度)から "N035E138" 形式の名前を作る(フロントのタイルURLと一致させる)。
std::string tile_name(const TileId& id) {
    char buf[16];
    std::snprintf(buf, sizeof(buf), "%c%03d%c%03d", id.lat >= 0 ? 'N' : 'S', std::abs(id.lat),
                  id.lon >= 0 ? 'E' : 'W', std::abs(id.lon));
    return buf;
}

// 1度タイルの標高(row0=北端の3600x3600、NaN=データなし)から、cells分割の(cells+1)x(cells+1)
// ノードのint16グリッド(行は南→北、列は西→東)を作る。ノードは1度タイル内のi/cells, j/cellsの
// 位置にあり、値はそのノードを中心とする1セル幅の窓(タイルの縁では半分)内の有効画素の平均。
// 窓内に有効画素が1つでもあれば陸(平均)、全部NaNならデータなし。
std::vector<int16_t> build_level_grid(const std::vector<float>& src, int cells) {
    const int nodes = cells + 1;
    const double cell_px = static_cast<double>(kTileFullPx) / cells;

    // ノード位置(画素座標、0=タイル端=画素の境界)を中心とする窓の画素範囲[start, end)。
    // 窓の半幅は1セル幅の半分だが、最低でも1画素(セルが1画素以下のとき、画素の中心ではなく
    // 境界にあるノードを、周囲2x2画素の平均で決める)。
    const double half_px = std::max(cell_px / 2.0, 1.0);
    auto window = [&](double center) {
        int start = static_cast<int>(std::floor(center - half_px + 0.5));
        int end = static_cast<int>(std::floor(center + half_px + 0.5));
        start = std::clamp(start, 0, kTileFullPx - 1);
        end = std::clamp(end, start + 1, kTileFullPx);
        return std::pair<int, int>(start, end);
    };
    std::vector<std::pair<int, int>> col_windows(nodes);
    std::vector<std::pair<int, int>> row_windows(nodes); // 南→北のノード番号jに対する、北端基準の行範囲
    for (int i = 0; i < nodes; ++i) {
        col_windows[i] = window(i * cell_px);
        row_windows[i] = window((cells - i) * cell_px);
    }

    std::vector<int16_t> grid(static_cast<size_t>(nodes) * nodes, kNoDataInt16);
    for (int j = 0; j < nodes; ++j) {
        const auto [r0, r1] = row_windows[j];
        for (int i = 0; i < nodes; ++i) {
            const auto [c0, c1] = col_windows[i];
            double sum = 0.0;
            int count = 0;
            for (int r = r0; r < r1; ++r) {
                const float* row = src.data() + static_cast<size_t>(r) * kTileFullPx;
                for (int c = c0; c < c1; ++c) {
                    const float v = row[c];
                    if (!std::isnan(v)) {
                        sum += v;
                        ++count;
                    }
                }
            }
            if (count > 0) {
                const long rounded = std::lround(sum / count);
                grid[static_cast<size_t>(j) * nodes + i] =
                    static_cast<int16_t>(std::clamp<long>(rounded, -32767, 32767));
            }
        }
    }
    return grid;
}

void write_int16_file(const fs::path& path, const std::vector<int16_t>& data) {
    std::ofstream out(path, std::ios::binary);
    if (!out) {
        throw std::runtime_error("failed to open output file: " + path.string());
    }
    // int16のリトルエンディアン(x86/x64ホスト前提。フロントもリトルエンディアンで読む)。
    out.write(reinterpret_cast<const char*>(data.data()),
              static_cast<std::streamsize>(data.size() * sizeof(int16_t)));
}

// タイル全体のグリッド(cells+1)^2を、kChunksPerTile x kChunksPerTileのチャンク(それぞれ
// (cells/kChunksPerTile+1)^2ノード。隣のチャンクと縁のノードを共有)に分け、行(南→北)・
// 列(西→東)の順に連結して書き出す。
void write_chunked_level_file(const fs::path& path, const std::vector<int16_t>& grid, int cells) {
    const int nodes = cells + 1;
    const int chunk_cells = cells / kChunksPerTile;
    std::ofstream out(path, std::ios::binary);
    if (!out) {
        throw std::runtime_error("failed to open output file: " + path.string());
    }
    std::vector<int16_t> record(static_cast<size_t>(chunk_cells + 1) * (chunk_cells + 1));
    for (int cy = 0; cy < kChunksPerTile; ++cy) {
        for (int cx = 0; cx < kChunksPerTile; ++cx) {
            for (int j = 0; j <= chunk_cells; ++j) {
                const int16_t* src =
                    grid.data() + static_cast<size_t>(cy * chunk_cells + j) * nodes + cx * chunk_cells;
                std::copy(src, src + chunk_cells + 1,
                          record.begin() + static_cast<size_t>(j) * (chunk_cells + 1));
            }
            out.write(reinterpret_cast<const char*>(record.data()),
                      static_cast<std::streamsize>(record.size() * sizeof(int16_t)));
        }
    }
}

// 1タイル分の処理結果。level0はベース(base.bin)へ連結するためメインスレッドへ返す。
struct TileResult {
    TileId id;
    bool has_land = false;
    float elevation_min = 0.0f;
    float elevation_max = 0.0f;
    std::vector<int16_t> level0;
};

// タイル1枚を読み込み、全レベルのグリッドを作る。レベル1以上はその場でファイルへ書き出す。
TileResult process_tile(const DiscoveredTile& tile, const fs::path& out_dir) {
    TileResult result;
    result.id = tile.id;

    const SourceGrid grid = load_dsm_tile(tile.path.string());
    if (grid.width != kTileFullPx || grid.height != kTileFullPx) {
        std::cerr << "[geotiff_preprocess] skip (unexpected size " << grid.width << "x"
                   << grid.height << ", expected " << kTileFullPx << "x" << kTileFullPx
                   << "): " << tile.path.filename().string() << std::endl;
        return result;
    }

    float lo = std::numeric_limits<float>::infinity();
    float hi = -std::numeric_limits<float>::infinity();
    for (float v : grid.elevation) {
        if (!std::isnan(v)) {
            lo = std::min(lo, v);
            hi = std::max(hi, v);
        }
    }
    if (!(lo <= hi)) {
        return result; // 全域がデータなし(海のみのタイル)。
    }
    result.has_land = true;
    result.elevation_min = lo;
    result.elevation_max = hi;

    const std::string name = tile_name(tile.id);
    for (int level = 0; level < kNumLevels; ++level) {
        std::vector<int16_t> level_grid = build_level_grid(grid.elevation, kLevelCells[level]);
        if (level == 0) {
            result.level0 = std::move(level_grid);
        } else {
            write_chunked_level_file(
                out_dir / "tiles" / ("L" + std::to_string(level)) / (name + ".bin"), level_grid,
                kLevelCells[level]);
        }
    }
    return result;
}

void write_tile_index_json(const fs::path& path, const std::vector<TileResult>& tiles) {
    std::ofstream out(path);
    if (!out) {
        throw std::runtime_error("failed to open output file: " + path.string());
    }
    out << "{\n  \"tiles\": [\n";
    for (size_t i = 0; i < tiles.size(); ++i) {
        const TileResult& t = tiles[i];
        out << "    {\"lat\": " << t.id.lat << ", \"lon\": " << t.id.lon
            << ", \"elevation_min\": " << t.elevation_min << ", \"elevation_max\": "
            << t.elevation_max << "}" << (i + 1 < tiles.size() ? "," : "") << "\n";
    }
    out << "  ]\n}\n";
}

// DETAILED_DESIGN.md 2.6節のスキーマに従ってmetadata.jsonを書き出す。
// フィールド構成が固定・少数なので、JSONライブラリは使わず手書きで組み立てる。
void write_metadata_json(const fs::path& path, float elevation_min, float elevation_max,
                          const GeodeticBounds& bounds) {
    std::ofstream out(path);
    if (!out) {
        throw std::runtime_error("failed to open output file: " + path.string());
    }
    // ostreamの既定精度(有効6桁)だと ellipsoid.a_m や default_origin の緯度経度が
    // 途中で丸められてしまう(例: 138.859722 -> 138.86)ため、明示的に精度を上げておく。
    out << std::setprecision(15);
    out << "{\n"
        << "  \"tile_levels\": [";
    for (int level = 0; level < kNumLevels; ++level) {
        out << kLevelCells[level] << (level + 1 < kNumLevels ? ", " : "");
    }
    out << "],\n"
        << "  \"chunks_per_tile\": " << kChunksPerTile << ",\n"
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
    std::string output_dir = "sample/sim_server/assets/terrain";
    if (argc > 1) {
        map_data_dir = argv[1];
    }
    if (argc > 2) {
        output_dir = argv[2];
    }

    GDALAllRegister();

    try {
        for (int level = 1; level < kNumLevels; ++level) {
            if (kLevelCells[level] % kChunksPerTile != 0) {
                throw std::runtime_error("kLevelCells must be divisible by kChunksPerTile");
            }
        }
        std::cout << "[geotiff_preprocess] scanning: " << map_data_dir << std::endl;
        std::vector<DiscoveredTile> tiles = discover_tiles(map_data_dir);
        const MosaicBounds mosaic_bounds = compute_mosaic_bounds(tiles);
        // 出力順(tile_index.json・base.bin)を実行ごとに安定させる。
        std::sort(tiles.begin(), tiles.end(), [](const DiscoveredTile& a, const DiscoveredTile& b) {
            return a.id.lat != b.id.lat ? a.id.lat < b.id.lat : a.id.lon < b.id.lon;
        });
        std::cout << "[geotiff_preprocess] found " << tiles.size() << " tile(s), bounds: "
                   << "lat " << mosaic_bounds.min_lat << ".." << mosaic_bounds.max_lat << ", lon "
                   << mosaic_bounds.min_lon << ".." << mosaic_bounds.max_lon << std::endl;

        const fs::path out_dir(output_dir);
        for (int level = 1; level < kNumLevels; ++level) {
            fs::create_directories(out_dir / "tiles" / ("L" + std::to_string(level)));
        }

        // タイルを並列処理する。1スレッドあたりのメモリは数百MB(標高52MB+マスク13MB+穴埋め等)
        // なので、コア数の半分(上限8)に抑える。
        const unsigned hardware = std::max(1u, std::thread::hardware_concurrency());
        const unsigned num_workers = std::min(8u, std::max(1u, hardware / 2));
        std::vector<TileResult> results(tiles.size());
        std::atomic<size_t> next_tile{0};
        std::atomic<size_t> done_tiles{0};
        std::mutex log_mutex;
        std::string first_error;
        std::vector<std::thread> workers;
        for (unsigned w = 0; w < num_workers; ++w) {
            workers.emplace_back([&]() {
                while (true) {
                    const size_t index = next_tile.fetch_add(1);
                    if (index >= tiles.size()) {
                        return;
                    }
                    try {
                        results[index] = process_tile(tiles[index], out_dir);
                    } catch (const std::exception& e) {
                        std::lock_guard<std::mutex> lock(log_mutex);
                        if (first_error.empty()) {
                            first_error = e.what();
                        }
                        return;
                    }
                    const size_t done = done_tiles.fetch_add(1) + 1;
                    std::lock_guard<std::mutex> lock(log_mutex);
                    std::cout << "[geotiff_preprocess] (" << done << "/" << tiles.size() << ") "
                               << tile_name(tiles[index].id)
                               << (results[index].has_land ? "" : " (no land, skipped)")
                               << std::endl;
                }
            });
        }
        for (auto& worker : workers) {
            worker.join();
        }
        if (!first_error.empty()) {
            throw std::runtime_error(first_error);
        }

        // 陸のあるタイルだけを索引・ベースへ書き出す。
        std::vector<TileResult> land_tiles;
        std::vector<int16_t> base;
        float elevation_min = std::numeric_limits<float>::infinity();
        float elevation_max = -std::numeric_limits<float>::infinity();
        for (TileResult& r : results) {
            if (!r.has_land) {
                continue;
            }
            elevation_min = std::min(elevation_min, r.elevation_min);
            elevation_max = std::max(elevation_max, r.elevation_max);
            base.insert(base.end(), r.level0.begin(), r.level0.end());
            r.level0.clear();
            r.level0.shrink_to_fit();
            land_tiles.push_back(std::move(r));
        }
        if (land_tiles.empty()) {
            throw std::runtime_error("no tile with land data");
        }

        const GeodeticBounds bounds{
            static_cast<double>(mosaic_bounds.min_lat), static_cast<double>(mosaic_bounds.max_lat),
            static_cast<double>(mosaic_bounds.min_lon), static_cast<double>(mosaic_bounds.max_lon)};

        write_int16_file(out_dir / "base.bin", base);
        write_tile_index_json(out_dir / "tile_index.json", land_tiles);
        write_metadata_json(out_dir / "metadata.json", elevation_min, elevation_max, bounds);

        // 旧方式(全域を1枚のheightmap.binへ縮小)の出力は使われなくなったので削除する。
        std::error_code ec;
        if (fs::remove(out_dir / "heightmap.bin", ec)) {
            std::cout << "[geotiff_preprocess] removed obsolete heightmap.bin" << std::endl;
        }

        std::cout << "[geotiff_preprocess] wrote " << land_tiles.size() << " tile(s) with "
                   << kNumLevels << " level(s) to " << out_dir.string() << std::endl;
        std::cout << "[geotiff_preprocess] elevation range: " << elevation_min << " .. "
                   << elevation_max << std::endl;
    } catch (const std::exception& e) {
        std::cerr << "[geotiff_preprocess] error: " << e.what() << std::endl;
        return 1;
    }

    return 0;
}
