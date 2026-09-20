#include <cstdint>
#include <iostream>
#include <string>

#include "ws_server.hpp"

namespace {

void print_usage(const char* exe) {
    std::cerr << "usage: " << exe << " [port] [--cert <cert.pem> --key <key.pem>]\n"
              << "  --cert/--key: 両方指定するとHTTPS/WSSで待ち受ける(省略時は平文のHTTP/WS)\n";
}

} // namespace

int main(int argc, char** argv) {
    uint16_t port = 9001;
    sim3dview::TlsConfig tls;

    for (int i = 1; i < argc; ++i) {
        const std::string arg = argv[i];
        if ((arg == "--cert" || arg == "--key") && i + 1 < argc) {
            (arg == "--cert" ? tls.cert_file : tls.key_file) = argv[++i];
        } else if (!arg.empty() && arg[0] != '-') {
            try {
                port = static_cast<uint16_t>(std::stoi(arg));
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

    std::cout << "Sim3dView sim_server starting (" << (tls.enabled() ? "TLS" : "no TLS") << ")"
              << std::endl;

    sim3dview::WsServer server(port, std::move(tls));
    server.run();

    return 0;
}
