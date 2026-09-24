# 地形前処理CLI

ALOS DSMのGeoTIFFから、sim3dviewが読む原点非依存のタイルLODを生成する。
形式は[ライブラリ設計書](../../docs/DETAILED_DESIGN.md)2・9.1・9.5節を参照。

## ビルド

C++20とGDALが必要。リポジトリルートから実行する。
LinuxではGDAL開発パッケージ、WindowsではVisual Studioとvcpkgのツールチェーンを利用する。

```sh
cmake -S tools/geotiff_preprocess -B tools/geotiff_preprocess/build -DCMAKE_BUILD_TYPE=Release
cmake --build tools/geotiff_preprocess/build --config Release
ctest --test-dir tools/geotiff_preprocess/build -C Release --output-on-failure
```

Windowsではconfigure時に `-DCMAKE_TOOLCHAIN_FILE=<vcpkg>/scripts/buildsystems/vcpkg.cmake` を追加する。

## 実行

```text
geotiff_preprocess [入力ディレクトリ=map_data] [出力ディレクトリ=terrain]
```

パスは起動時の作業ディレクトリから解決する。出力先は任意で、特定のサーバー構成には依存しない。
