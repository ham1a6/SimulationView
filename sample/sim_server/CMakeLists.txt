cmake_minimum_required(VERSION 3.20)
project(sim3dview_sim_server CXX C)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_C_STANDARD 11)

set(USOCKETS_DIR ${CMAKE_CURRENT_SOURCE_DIR}/third_party/uWebSockets/uSockets)
set(UWEBSOCKETS_DIR ${CMAKE_CURRENT_SOURCE_DIR}/third_party/uWebSockets)
set(MSGPACK_CXX_DIR ${CMAKE_CURRENT_SOURCE_DIR}/third_party/msgpack-cxx)

# --- uSockets (uWebSocketsが依存するCネットワーキング層) ---------------------
# QUIC/io_uringは本プロジェクトでは不要なため、そのソース(quic.c, io_uring/)はビルド対象から
# 除外する(third_party/uWebSocketsからもboringssl/lsquic/fuzzing等の重いサブモジュールは
# 取得していない)。TLS(HTTPS/WSS)はvcpkgのOpenSSLで対応する(crypto/openssl.c, sni_tree.cpp)。
add_library(usockets STATIC
    ${USOCKETS_DIR}/src/bsd.c
    ${USOCKETS_DIR}/src/context.c
    ${USOCKETS_DIR}/src/loop.c
    ${USOCKETS_DIR}/src/socket.c
    ${USOCKETS_DIR}/src/udp.c
    ${USOCKETS_DIR}/src/crypto/openssl.c
    ${USOCKETS_DIR}/src/crypto/sni_tree.cpp
)
target_include_directories(usockets PUBLIC ${USOCKETS_DIR}/src)
find_package(OpenSSL REQUIRED)
find_package(Threads REQUIRED)
target_compile_definitions(usockets PUBLIC LIBUS_USE_OPENSSL)
target_link_libraries(usockets PUBLIC OpenSSL::SSL OpenSSL::Crypto Threads::Threads)

if (WIN32)
    # libusockets.h は _WIN32 かつ他のイベンティング指定が無い場合、自動的に
    # LIBUS_USE_LIBUV を選択する(epoll/kqueueはWindows非対応のため)。
    target_sources(usockets PRIVATE ${USOCKETS_DIR}/src/eventing/libuv.c)
    find_package(libuv CONFIG REQUIRED)
    target_link_libraries(usockets PUBLIC
        $<IF:$<TARGET_EXISTS:libuv::uv_a>,libuv::uv_a,libuv::uv>)
else()
    target_sources(usockets PRIVATE ${USOCKETS_DIR}/src/eventing/epoll_kqueue.c)
endif()

if (MSVC)
    target_compile_definitions(usockets PRIVATE _CRT_SECURE_NO_WARNINGS)
endif()

# --- uWebSockets (ヘッダオンリー) -----------------------------------------
# PerMessageDeflate.h が常に zlib.h を要求するため、圧縮を使わない(DISABLED)構成でも
# zlibのリンクは必要。
find_package(ZLIB REQUIRED)

add_library(uwebsockets INTERFACE)
target_include_directories(uwebsockets INTERFACE ${UWEBSOCKETS_DIR}/src)
target_link_libraries(uwebsockets INTERFACE usockets ZLIB::ZLIB)

# --- msgpack-cxx (ヘッダオンリー) -----------------------------------------
add_library(msgpack_cxx INTERFACE)
target_include_directories(msgpack_cxx INTERFACE ${MSGPACK_CXX_DIR}/include)
# boost非依存でビルドする(msgpack同梱のpredef/other/endian.hを使う)。
target_compile_definitions(msgpack_cxx INTERFACE MSGPACK_NO_BOOST)

# --- sim_server 本体 -------------------------------------------------------
add_library(simulation_core STATIC src/simulation.cpp)
target_include_directories(simulation_core PUBLIC include)
target_link_libraries(simulation_core PUBLIC msgpack_cxx)

add_executable(sim_server src/main.cpp src/ws_server.cpp)
target_include_directories(sim_server PRIVATE include)
target_link_libraries(sim_server PRIVATE simulation_core uwebsockets)

if (WIN32)
    target_compile_definitions(sim_server PRIVATE _WIN32_WINNT=0x0601 NOMINMAX)
    target_link_libraries(sim_server PRIVATE ws2_32)
endif()

if (MSVC)
    target_compile_options(sim_server PRIVATE /Zc:__cplusplus)
    target_compile_options(simulation_core PRIVATE /Zc:__cplusplus)
endif()

include(CTest)
if (BUILD_TESTING)
    add_executable(simulation_test tests/simulation_test.cpp)
    target_link_libraries(simulation_test PRIVATE simulation_core)
    add_test(NAME simulation_state COMMAND simulation_test)

    add_executable(http_utils_test tests/http_utils_test.cpp)
    target_include_directories(http_utils_test PRIVATE include)
    add_test(NAME http_utils COMMAND http_utils_test)
endif()
