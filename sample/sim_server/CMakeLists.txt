cmake_minimum_required(VERSION 3.20)
project(sim3dview_sim_server CXX C)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_C_STANDARD 11)

set(USOCKETS_DIR ${CMAKE_CURRENT_SOURCE_DIR}/third_party/uWebSockets/uSockets)
set(UWEBSOCKETS_DIR ${CMAKE_CURRENT_SOURCE_DIR}/third_party/uWebSockets)
set(MSGPACK_CXX_DIR ${CMAKE_CURRENT_SOURCE_DIR}/third_party/msgpack-cxx)

# --- uSockets (uWebSocketsが依存するCネットワーキング層) ---------------------
# TLS/QUIC/io_uringは本プロジェクトでは不要なため、それらのソース(crypto/, quic.c,
# io_uring/)はビルド対象から除外する(third_party/uWebSocketsからもboringssl/lsquic/
# fuzzing等の重いサブモジュールは取得していない)。
add_library(usockets STATIC
    ${USOCKETS_DIR}/src/bsd.c
    ${USOCKETS_DIR}/src/context.c
    ${USOCKETS_DIR}/src/loop.c
    ${USOCKETS_DIR}/src/socket.c
    ${USOCKETS_DIR}/src/udp.c
)
target_include_directories(usockets PUBLIC ${USOCKETS_DIR}/src)
target_compile_definitions(usockets PUBLIC LIBUS_NO_SSL)

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
add_executable(sim_server
    src/main.cpp
    src/simulation.cpp
    src/ws_server.cpp
)
target_include_directories(sim_server PRIVATE include)
target_link_libraries(sim_server PRIVATE uwebsockets msgpack_cxx)

if (WIN32)
    target_compile_definitions(sim_server PRIVATE _WIN32_WINNT=0x0601 NOMINMAX)
    target_link_libraries(sim_server PRIVATE ws2_32)
endif()

if (MSVC)
    target_compile_options(sim_server PRIVATE /Zc:__cplusplus)
endif()

# --- GeoTIFF前処理ツール(単発CLI、GDAL依存はこのサブディレクトリに限定) -------------
add_subdirectory(tools/geotiff_preprocess)
