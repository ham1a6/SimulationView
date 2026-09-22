#pragma once

#include <algorithm>
#include <cctype>
#include <cstdint>
#include <string>
#include <string_view>

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

} // namespace sim3dview::http_utils
