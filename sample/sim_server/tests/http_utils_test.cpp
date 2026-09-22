#include "http_utils.hpp"

#include <cassert>

int main() {
    using namespace sim3dview::http_utils;
    std::uintmax_t start = 0, end = 0;
    assert(parse_byte_range("bytes=10-19", 100, start, end) && start == 10 && end == 19);
    assert(parse_byte_range("bytes=90-", 100, start, end) && start == 90 && end == 99);
    assert(parse_byte_range("bytes=90-999", 100, start, end) && end == 99);
    assert(!parse_byte_range("bytes=-10", 100, start, end));
    assert(!parse_byte_range("bytes=0-1,3-4", 100, start, end));
    assert(!parse_byte_range("bytes=0-", 0, start, end));
    assert(etag_matches("\"a\", W/\"b\"", "\"b\""));
    assert(etag_matches("*", "\"a\""));
    assert(!etag_matches("\"a\"", "\"b\""));
    assert(is_safe_path_component("N035E138.bin"));
    assert(is_safe_path_component("L4"));
    assert(!is_safe_path_component("../secret"));
    assert(!is_safe_path_component("a/b"));
    assert(!is_safe_path_component(""));
}
