#include <cstdint>
#include <iostream>

#include "ws_server.hpp"

int main(int argc, char** argv) {
    uint16_t port = 9001;
    if (argc > 1) {
        try {
            port = static_cast<uint16_t>(std::stoi(argv[1]));
        } catch (const std::exception&) {
            std::cerr << "invalid port argument: " << argv[1] << std::endl;
            return 1;
        }
    }

    std::cout << "Sim3dView sim_server starting" << std::endl;

    sim3dview::WsServer server(port);
    server.run();

    return 0;
}
