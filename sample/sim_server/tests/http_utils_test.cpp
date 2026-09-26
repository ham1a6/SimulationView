#include "http_utils.hpp"

#include <cassert>
#include <string>

namespace {

// テスト検証専用: gzip_compress()が作ったバイト列を元に戻す(zlibのinflate、
// windowBits=15+32でgzip/zlibどちらのヘッダも受け付ける)。
std::string gzip_decompress(const std::string& compressed) {
    z_stream stream{};
    const int init_result = inflateInit2(&stream, 15 + 32);
    assert(init_result == Z_OK);
    // テストで使う短い文字列を十分収められるだけの大きさ(圧縮率が高い入力だと
    // 元のバイト数の何倍にも展開されるため、圧縮後サイズからの見積もりでは足りない)。
    std::string out(4096, '\0');
    stream.next_in = reinterpret_cast<Bytef*>(const_cast<char*>(compressed.data()));
    stream.avail_in = static_cast<uInt>(compressed.size());
    stream.next_out = reinterpret_cast<Bytef*>(out.data());
    stream.avail_out = static_cast<uInt>(out.size());
    const int result = inflate(&stream, Z_FINISH);
    assert(result == Z_STREAM_END);
    out.resize(out.size() - stream.avail_out);
    inflateEnd(&stream);
    return out;
}

} // namespace

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

    assert(accepts_gzip("gzip"));
    assert(accepts_gzip("gzip, deflate, br"));
    assert(accepts_gzip("deflate, gzip;q=0.8"));
    assert(accepts_gzip("*"));
    assert(!accepts_gzip("deflate"));
    assert(!accepts_gzip(""));
    assert(!accepts_gzip("gzip;q=0"));
    assert(!accepts_gzip("gzip;q=0, deflate"));

    const std::string original(200, 'a'); // 圧縮率が分かりやすい単純な繰り返し
    const auto compressed = gzip_compress(original);
    assert(compressed.has_value());
    assert(compressed->size() < original.size());
    assert(gzip_decompress(*compressed) == original);
}
