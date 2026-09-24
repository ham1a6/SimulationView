#pragma once

#include <filesystem>
#include <fstream>
#include <random>
#include <sstream>
#include <stdexcept>
#include <string>
#include <string_view>

namespace sim3dview {
// 受信中は一時ファイルだけを公開し、正常終了時に確定する。
// クライアントの名前をパスに使わず、排他的に作ったディレクトリへ保存する。
class UploadFile {
public:
    static constexpr std::size_t max_bytes = 64 * 1024 * 1024;
    explicit UploadFile(const std::filesystem::path& root) {
        std::filesystem::create_directories(root);
        std::random_device random;
        for (int attempt = 0; attempt < 100; ++attempt) {
            std::ostringstream name;
            name << std::hex << random() << random() << random() << random();
            auto candidate = root / name.str();
            if (std::filesystem::create_directory(candidate)) {
                directory_ = candidate;
                id_ = name.str() + "/data.bin";
                break;
            }
        }
        if (directory_.empty()) throw std::runtime_error("保存IDを作成できません");
        stream_.open(directory_ / "data.part", std::ios::binary);
        if (!stream_) {
            discard();
            throw std::runtime_error("保存ファイルを作成できません");
        }
    }
    UploadFile(const UploadFile&) = delete;
    UploadFile& operator=(const UploadFile&) = delete;
    ~UploadFile() { if (!committed_) discard(); }
    bool append(std::string_view bytes) {
        if (bytes.size() > max_bytes - size_) return false;
        stream_.write(bytes.data(), static_cast<std::streamsize>(bytes.size()));
        if (!stream_) throw std::runtime_error("書き込みに失敗しました");
        size_ += bytes.size();
        return true;
    }
    std::string finish() {
        stream_.close();
        if (!stream_) throw std::runtime_error("保存に失敗しました");
        std::filesystem::rename(directory_ / "data.part", directory_ / "data.bin");
        committed_ = true;
        return id_;
    }
private:
    void discard() noexcept {
        stream_.close();
        std::error_code ec;
        std::filesystem::remove(directory_ / "data.part", ec);
        std::filesystem::remove(directory_, ec);
    }
    std::filesystem::path directory_;
    std::ofstream stream_;
    std::string id_;
    std::size_t size_ = 0;
    bool committed_ = false;
};
} // namespace sim3dview
