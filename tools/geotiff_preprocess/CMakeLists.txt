# GeoTIFF前処理ツール(単発実行CLI)。sim3dviewライブラリが読む地形データ
# (heightmap.bin/metadata.json)を、ALOS DSM GeoTIFFタイルから生成する。
#
# sample/sim_server(常駐サーバー)とは独立したCMakeプロジェクトにしてあり、GDAL依存は
# このツールに限定される(サーバー側はGDALをリンクしない)。ビルド手順はREADME.md参照。
cmake_minimum_required(VERSION 3.20)
project(geotiff_preprocess CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

add_executable(geotiff_preprocess main.cpp)
target_include_directories(geotiff_preprocess PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})

# vcpkgのConfigとLinuxディストリビューションのFindGDALの双方に対応する。
find_package(GDAL CONFIG QUIET)
if (NOT GDAL_FOUND)
    find_package(GDAL REQUIRED)
endif()
find_package(Threads REQUIRED)
target_link_libraries(geotiff_preprocess PRIVATE GDAL::GDAL Threads::Threads)

if (MSVC)
    target_compile_options(geotiff_preprocess PRIVATE /Zc:__cplusplus)
    target_compile_definitions(geotiff_preprocess PRIVATE _CRT_SECURE_NO_WARNINGS)
endif()

include(CTest)
if (BUILD_TESTING)
    add_executable(preprocess_core_test tests/preprocess_core_test.cpp)
    target_include_directories(preprocess_core_test PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})
    add_test(NAME preprocess_core COMMAND preprocess_core_test)
endif()
