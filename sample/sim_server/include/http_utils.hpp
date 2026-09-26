#pragma once

#include <algorithm>
#include <cctype>
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>

#include <zlib.h>

namespace sim3dview::http_utils {

inline bool parse_byte_range(std::string_view header, std::uintmax_t total,
                             std::uintmax_t& start, std::uintmax_t& end) {
    constexpr std::string_view prefix = "bytes=";
    if (header.substr(0, prefix.size()) != prefix) return false;
    const std::string spec(header.substr(prefix.size()));
    const size_t dash = spec.find('-');
    if (dash == std::string::npos || dash == 0 || spec.find(',') != std::string::npos) return false;
    try {
        start = std::stoull(spec.substr(0, dash));
        end = dash + 1 < spec.size() ? std::stoull(spec.substr(dash + 1)) : total - 1;
    } catch (const std::exception&) {
        return false;
    }
    if (total == 0 || start >= total) return false;
    end = std::min<std::uintmax_t>(end, total - 1);
    return start <= end;
}

inline bool etag_matches(std::string_view candidates, std::string_view etag) {
    if (candidates.empty() || etag.empty()) return false;
    if (candidates == "*") return true;
    size_t pos = 0;
    while (pos < candidates.size()) {
        size_t end = candidates.find(',', pos);
        if (end == std::string_view::npos) end = candidates.size();
        std::string_view token = candidates.substr(pos, end - pos);
        while (!token.empty() && token.front() == ' ') token.remove_prefix(1);
        while (!token.empty() && token.back() == ' ') token.remove_suffix(1);
        if (token.substr(0, 2) == "W/") token.remove_prefix(2);
        if (token == etag) return true;
        pos = end + 1;
    }
    return false;
}

inline bool is_safe_path_component(std::string_view name) {
    if (name.empty() || name.size() > 64 || name.find("..") != std::string_view::npos) return false;
    return std::all_of(name.begin(), name.end(), [](unsigned char c) {
        return std::isalnum(c) || c == '_' || c == '.';
    });
}

// `Accept-Encoding`ヘッダに、重み0(`;q=0`)ではない`gzip`(または`*`)が含まれるか。
// 地形データはHTTP Rangeでの部分取得(206)を除き、この判定に応じてgzip圧縮して返す。
inline bool accepts_gzip(std::string_view accept_encoding) {
    size_t pos = 0;
    while (pos <= accept_encoding.size()) {
        size_t comma = accept_encoding.find(',', pos);
        if (comma == std::string_view::npos) comma = accept_encoding.size();
        std::string_view token = accept_encoding.substr(pos, comma - pos);
        pos = comma + 1;
        const size_t semi = token.find(';');
        std::string_view name = token.substr(0, semi);
        while (!name.empty() && name.front() == ' ') name.remove_prefix(1);
        while (!name.empty() && name.back() == ' ') name.remove_suffix(1);
        std::string lower(name);
        std::transform(lower.begin(), lower.end(), lower.begin(),
                       [](unsigned char c) { return static_cast<char>(std::tolower(c)); });
        if (lower != "gzip" && lower != "*") continue;
        if (semi != std::string_view::npos) {
            const std::string_view params = token.substr(semi);
            // "q=0"かつ"q=0.xxx"ではない(=完全な重み0)場合だけ拒否扱いにする。
            if (params.find("q=0") != std::string_view::npos && params.find("q=0.") == std::string_view::npos) {
                continue;
            }
        }
        return true;
    }
    return false;
}

// gzip形式(zlibのwindowBits=15+16)で圧縮する。失敗したら`std::nullopt`
// (呼び出し側は無圧縮のまま返す)。
inline std::optional<std::string> gzip_compress(std::string_view data) {
    z_stream stream{};
    if (deflateInit2(&stream, Z_DEFAULT_COMPRESSION, Z_DEFLATED, 15 + 16, 8, Z_DEFAULT_STRATEGY) != Z_OK) {
        return std::nullopt;
    }
    std::string out(deflateBound(&stream, static_cast<uLong>(data.size())), '\0');
    stream.next_in = reinterpret_cast<Bytef*>(const_cast<char*>(data.data()));
    stream.avail_in = static_cast<uInt>(data.size());
    stream.next_out = reinterpret_cast<Bytef*>(out.data());
    stream.avail_out = static_cast<uInt>(out.size());
    const int result = deflate(&stream, Z_FINISH);
    const auto produced = out.size() - stream.avail_out;
    deflateEnd(&stream);
    if (result != Z_STREAM_END) return std::nullopt;
    out.resize(produced);
    return out;
}

} // namespace sim3dview::http_utils
