# Sim3dView デスクトップ版

既存のRust/WASM画面をElectronで表示し、専用のC++サーバーを自動起動するWindows x64アプリ。
ブラウザ版の`sim_server.exe` + `trunk serve`による起動も引き続き使える。
双方は別のシミュレーションとして動作し、同じ前処理済み地形を読み取り専用で共有する。

## 開発環境から起動

ルートREADMEの手順でC++サーバー(Debug)をビルドし、地形を生成しておく。
追加でNode.js 22.12以降(推奨24 LTS)が必要。

```powershell
cd sample/sim_desktop
npm ci
npm run build:ui
npm start
```

初回の`npm start`ではElectron本体のダウンロードが発生する。
`npm run build:ui`はReleaseのHTML/JS/WASMを`out/frontend`へ生成する。
Trunk開発サーバーの`sample/sim_frontend/dist`は使わない。UIを編集したら再ビルドして起動する。
ブラウザ版のサーバーが9001番で起動中でも、Electronは別の空きポートを使う。

既定の地形は`sample/sim_server/assets/terrain`。
配布版では実行ファイルの隣の`terrain`を探し、見つからなければフォルダー選択画面を開く。
選ぶのはGeoTIFFの`map_data`ではなく、`metadata.json`、`tile_index.json`、`base.bin`、`tiles/`のある前処理済みフォルダー。
選択はユーザーデータ内の`desktop-settings.json`に保存される。
「ファイル → 地形フォルダーを変更…」で変更するとアプリが再起動する。

環境変数で指定する場合(保存設定より優先):

`SIM3DVIEW_TERRAIN_DIR`指定中はメニューからの地形変更を無効にする。

```powershell
$env:SIM3DVIEW_TERRAIN_DIR = 'D:\Sim3dView\terrain'
$env:SIM3DVIEW_SERVER_EXE = 'C:\path\to\sim_server.exe'
npm start
```

ウィンドウを閉じると、このアプリが起動したC++プロセスとUI配信サーバーを終了する。
起動失敗やC++プロセスの異常終了はダイアログで通知する。
同じデスクトップアプリの二重起動時は既存ウィンドウを前面に出す。

## Windows配布フォルダーの作成

ルートREADMEで使うCMakeでC++サーバーをReleaseビルドしてから実行する。

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
地形は容量が大きいため自動コピーせず、初回起動時にフォルダーを選ぶ。
署名とインストーラー、自動更新はこの配布処理の対象外。

Electronの`LICENSE`、`LICENSES.chromium.html`と、プロジェクトの`THIRD_PARTY_NOTICE.md`を一緒に配布する。
`SIM3DVIEW_SERVER_EXE`を指定して別の実行ファイルを同梱することも可能だが、配布にはReleaseビルドを使う。

## 検証

```powershell
npm test
npm run test:ui
```

Nodeの組み込みテストで静的配信のMIME/パス境界、実サーバー2個の同時起動、
HTTP Range、WebSocket接続、子プロセス終了を検証する。
実サーバーテストにはビルド済みC++サーバーと既定の地形が必要。
`test:ui`はElectron本体を起動し、開始・一時停止、2D/3D切替、再読み込みを繰り返し、
スクリーンショットを`out/smoke`へ保存する。終了後のポート解放も確認する。
3D描画はスクリーンショットに加え、実際にカメラを操作して確認する。
描画の可否は実機GPUとドライバーに依存する。
