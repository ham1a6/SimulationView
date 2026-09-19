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

find_package(GDAL CONFIG REQUIRED)
target_link_libraries(geotiff_preprocess PRIVATE GDAL::GDAL)

if (MSVC)
    target_compile_options(geotiff_preprocess PRIVATE /Zc:__cplusplus)
    target_compile_definitions(geotiff_preprocess PRIVATE _CRT_SECURE_NO_WARNINGS)
endif()
