# GeoTIFF前処理ツール(単発実行CLI)。sim_server本体とは別ビルドターゲットにし、
# GDAL依存をこのツールに限定する(常駐サーバーであるsim_serverはGDALをリンクしない)。

add_executable(geotiff_preprocess main.cpp)

find_package(GDAL CONFIG REQUIRED)
target_link_libraries(geotiff_preprocess PRIVATE GDAL::GDAL)

if (MSVC)
    target_compile_options(geotiff_preprocess PRIVATE /Zc:__cplusplus)
    target_compile_definitions(geotiff_preprocess PRIVATE _CRT_SECURE_NO_WARNINGS)
endif()
