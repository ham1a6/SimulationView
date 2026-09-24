#!/usr/bin/env bash
# Ubuntu 24.04でのサンプル全体のビルド・検証。依存の導入はsample/README.mdを参照。
set -euo pipefail
cd "$(dirname "$0")/.."
git submodule update --init sim_server/third_party/uWebSockets sim_server/third_party/msgpack-cxx
git -C sim_server/third_party/uWebSockets submodule update --init uSockets libdeflate
cmake -S sim_server -B sim_server/build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build sim_server/build --parallel 2
ctest --test-dir sim_server/build --output-on-failure
cargo check -p sim_frontend --target wasm32-unknown-unknown --locked
cd sim_desktop
npm ci
npm test
npm run build:ui
# 実地形を準備した場合だけ配布フォルダーも生成する。
if [[ -n "${SIM3DVIEW_TERRAIN_DIR:-}" || -f ../sim_server/assets/terrain/metadata.json ]]; then
  npm run package
  test -x out/Sim3dView-linux-x64-*/Sim3dView
  test -f out/Sim3dView-linux-x64-*/resources/terrain/base.bin
fi
