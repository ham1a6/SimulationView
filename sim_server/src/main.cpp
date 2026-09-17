#include <cstdint>
#include <iostream>

#include "ws_server.hpp"

int main(int argc, char** argv) {
    uint16_t port = 9001;
    if (argc > 1) {
        port = static_cast<uint16_t>(std::stoi(argv[1]));
    }

    std::cout << "Sim3dView sim_server starting" << std::endl;

    sim3dview::WsServer server(port);
    server.run();

    return 0;
}
