#include <cstdint>
#include <iostream>
#include <stdexcept>
#include <string>

#include "ws_server.hpp"

namespace {

void print_usage(const char* exe) {
    std::cerr << "usage: " << exe << " [port] [--host <address>] [--terrain-dir <path>] [--upload-dir <path>]"
              << " [--cert <cert.pem> --key <key.pem>]\n"
              << "  port: 0〜65535。0は空きポートを自動割り当て\n"
              << "  --cert/--key: WebTransport用のTLS証明書・秘密鍵(両方必須)\n";
}

} // namespace

int main(int argc, char** argv) {
    uint16_t port = 9001;
    sim3dview::TlsConfig tls;
    std::string host = "0.0.0.0";
    std::string terrain_dir = "assets/terrain";
    std::string upload_dir = "uploads";

    for (int i = 1; i < argc; ++i) {
        const std::string arg = argv[i];
        if ((arg == "--cert" || arg == "--key") && i + 1 < argc) {
            (arg == "--cert" ? tls.cert_file : tls.key_file) = argv[++i];
        } else if (arg == "--host" && i + 1 < argc) {
            host = argv[++i];
        } else if (arg == "--upload-dir" && i + 1 < argc) {
            upload_dir = argv[++i];
        } else if (arg == "--terrain-dir" && i + 1 < argc) {
            terrain_dir = argv[++i];
        } else if (!arg.empty() && arg[0] != '-') {
            try {
                size_t consumed = 0;
                const int parsed = std::stoi(arg, &consumed);
                if (consumed != arg.size() || parsed < 0 || parsed > 65535) {
                    throw std::out_of_range("port");
                }
                port = static_cast<uint16_t>(parsed);
            } catch (const std::exception&) {
                std::cerr << "invalid port argument: " << arg << std::endl;
                return 1;
            }
        } else {
            print_usage(argv[0]);
            return 1;
        }
    }
    if (tls.cert_file.empty() != tls.key_file.empty()) {
        std::cerr << "--cert and --key must be specified together" << std::endl;
        return 1;
    }

    std::cout << "Sim3dView sim_server starting (HTTPS + WebTransport)"
              << std::endl;

    try {
        sim3dview::WebTransportServer server(port, std::move(tls), host, terrain_dir, upload_dir);
        server.run();
    } catch (const std::exception& e) {
        std::cerr << e.what() << std::endl;
        return 1;
    }

    return 0;
}
