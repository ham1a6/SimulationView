# Sim3dView デスクトップ版

既存のRust/WASM画面をElectronで表示し、専用のC++サーバーを自動起動するWindows/Linux x64アプリ。
ブラウザ版の`sim_server.exe` + `trunk serve`による起動も引き続き使える。
双方は別のシミュレーションとして動作し、同じ前処理済み地形を読み取り専用で共有する。

## 開発環境から起動

[サンプルREADME](../README.md)の手順でC++サーバー(Debug)をビルドし、地形を生成しておく。
追加でNode.js 22.12以降(推奨24 LTS)が必要。

```powershell
cd sample/sim_desktop
npm ci
npm run build:ui
npm start
```

`npm ci`ではElectron本体のダウンロードが発生する。
`npm run build:ui`はReleaseのHTML/JS/WASMを`out/frontend`へ生成する。
Trunk開発サーバーの`sample/sim_frontend/dist`は使わない。UIを編集したら再ビルドして起動する。
ブラウザ版のサーバーが9001番で起動中でも、Electronは別の空きポートを使う。

開発時の既定の地形は`sample/sim_server/assets/terrain`。
配布版は`resources/terrain`の同梱データを使い、フォルダー選択や保存設定は不要。
`SIM3DVIEW_TERRAIN_DIR`は開発時の読み込み先と配布生成時のコピー元だけに使用する。

開発時に環境変数で指定する場合:

```powershell
$env:SIM3DVIEW_TERRAIN_DIR = 'D:\Sim3dView\terrain'
$env:SIM3DVIEW_SERVER_EXE = 'C:\path\to\sim_server.exe'
npm start
```

ウィンドウを閉じると、このアプリが起動したC++プロセスとUI配信サーバーを終了する。
起動失敗やC++プロセスの異常終了はダイアログで通知する。
同じデスクトップアプリの二重起動時は既存ウィンドウを前面に出す。

## Windows配布フォルダーの作成

[サンプルREADME](../README.md)で使うCMakeでC++サーバーをReleaseビルドしてから実行する。

```powershell
# リポジトリルートで実行
$cmake = 'C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'
& $cmake --build sample/sim_server/build --config Release
cd sample/sim_desktop
npm ci
npm run build:ui
npm run package
```

`out/Sim3dView-win32-x64-<生成時刻>/Sim3dView.exe`が起動入口。
**フォルダー全体**を渡す。利用者側にはNode.js・Rust・Trunkは不要。
Visual C++のランタイムが無いPCではMicrosoft Visual C++ Redistributable(x64)が必要。
地形全体を`resources/terrain`にコピーする。必須ファイルがない場合は配布生成を失敗させる。
容量と生成時間は入力地形に比例する(現在の全域データは約12GB)。
署名とインストーラー、自動更新はこの配布処理の対象外。

Electronの`LICENSE`、`LICENSES.chromium.html`と、プロジェクトの`THIRD_PARTY_NOTICE.md`を一緒に配布する。
`SIM3DVIEW_SERVER_EXE`を指定して別の実行ファイルを同梱することも可能だが、配布にはReleaseビルドを使う。

## Linuxでの起動と配布

[サンプルREADME](../README.md)のLinux手順でサーバーと地形を準備し、Node.js 22.12以降を導入する。
X11またはWaylandのデスクトップセッションと、WebGPU対応GPU・ドライバーが必要。
Ubuntu 24.04ではElectron用の共有ライブラリも導入する。

```bash
sudo apt-get install -y libgtk-3-0t64 libnss3 libasound2t64 libgbm1
cd sample/sim_desktop
npm ci
npm run build:ui
npm test
npm start
```

サーバーの既定パスは`sample/sim_server/build/sim_server`。
別のビルド場所や地形を使う場合は次のように指定する。

```bash
SIM3DVIEW_SERVER_EXE=/absolute/path/to/sim_server SIM3DVIEW_TERRAIN_DIR=/absolute/path/to/terrain npm start
npm run package
```

配布生成はLinux x64上で行う。Releaseビルドを使い、
`out/Sim3dView-linux-x64-<生成時刻>/Sim3dView`を起動する。
フォルダー全体を実行権限を保つ形式(tar等)で渡す。
Linuxのシステム共有ライブラリは同梱しないため、配布先にもElectronの依存と
OpenSSL・zlib・C++ランタイムが必要。同じUbuntuリリース・x64を配布先の基準とし、
`ldd resources/server/sim_server`で未解決の共有ライブラリがないことを確認する。
Electronのsandboxは有効のまま使用し、管理者(root)では起動しない。
環境側でsandboxが拒否される場合はそのエラーを管理者に確認する。

## 検証

### オフライン運用

ビルド済み配布フォルダー、前処理済み地形、OSの必要なランタイムとGPUドライバーを事前に用意すれば、インターネット接続なしで運用できる。
JS/WASM・CSS・3Dモデル・地形を配布フォルダー内から読み込む。
通信は同一PC内のHTTP/WebSocket(`127.0.0.1`)を使うため、ローカル通信とサーバー起動は必要。
地形は自動同梱されるため、配布フォルダー全体を持ち込めばよい。

初回の開発環境構築は別途準備が必要。Rust/WASMターゲット・Cargo依存・Trunkと補助ツール、C++ツールチェーン・サブモジュール・vcpkg依存、Node.js・Electronを接続環境で取得しておく。
`CARGO_NET_OFFLINE=true`で既存キャッシュによるUIビルドを検証できるが、新しいPCで依存を取得せずビルドできることは意味しない。

```powershell
$env:SIM3DVIEW_TEST_OFFLINE = '1'
$env:SIM3DVIEW_TEST_OUTPUT = Join-Path (Get-Location) 'out/offline-smoke'
npm run test:ui
Remove-Item Env:SIM3DVIEW_TEST_OFFLINE
Remove-Item Env:SIM3DVIEW_TEST_OUTPUT
```

このモードは新規テストプロファイルで、Electronセッションのループバック以外へのHTTP/WebSocket要求を拒否する。
起動・開始/一時停止・2D/3D切替・再読み込みを検証し、外部要求があれば失敗する。
取得URL一覧を`offline-requests.json`、画面を`screen-0.png`・`screen-1.png`へ保存する。
OSのネットワーク切断やファイアウォール変更は行わず、OS全体・Electron内部サービス・C++プロセスの全通信を遮断する試験ではない。

2026-09-23のWindows実機検証では、このモードで2回の操作と再読み込みが成功し、外部要求0件、ローカルURL 438件を確認した。
依存キャッシュを使ったオフライン指定のRelease UIビルドと、ローカル配信・実C++サーバーの4件のテストも成功した。

### 通常の検証

```powershell
npm test
npm run test:ui
```

Nodeの組み込みテストで静的配信のMIME/パス境界、実サーバー2個の同時起動、
HTTP Range、WebSocket接続、子プロセス終了を検証する。
実サーバーテストにはビルド済みC++サーバーが必要。通信専用の最小地形ファイルは一時生成する。
`test:ui`はElectron本体を起動し、開始・一時停止、2D/3D切替、再読み込みを繰り返し、
スクリーンショットを`out/smoke`へ保存する。終了後のポート解放も確認する。
3D描画はスクリーンショットに加え、実際にカメラを操作して確認する。
描画の可否は実機GPUとドライバーに依存する。
