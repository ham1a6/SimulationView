# Sim3dView

ALOS全球数値地表モデル(DEM)による3D地形ビューア・レーダー覆域/見通し計算ライブラリ
「`sim3dview`」と、それを使ったサンプルアプリ(C++シミュレータ本体 + Rust製Web UI)。

## 構成とビルドの境界

ライブラリはルートの `Cargo.toml`・`src/`・`style/`、サンプルはこの `sample/` 以下に置く。
サンプルの独立ワークスペースは `sample/Cargo.toml`。Cargo.lockとビルド出力 `sample/target/` も分離する。
`sim_frontend/` はWeb UI、`sim_server/` はC++サーバー、`sim_desktop/` はElectron起動・配布処理、
`scripts/` はアセットとライセンス生成、`tools/` は地形前処理を担当する。
前処理CLIはライブラリの地形形式を生成するため `../tools/geotiff_preprocess/` に置く。

[ライブラリの利用方法](../README.md)・[ライブラリ設計](../docs/DETAILED_DESIGN.md)・[サンプル設計](docs/DETAILED_DESIGN.md)・[開発履歴](docs/DEVELOPMENT_HISTORY.md)。
以下のコマンドは特記がなければ**リポジトリルート**から実行する。

## 動作環境

Windows + Visual Studio 2022、およびLinux x64向けのビルド・起動に対応する。
Linuxの手順は下の「Linuxでのセットアップ」を参照。GPU描画は利用する実機で確認する。
Ubuntu 24.04コンテナでC++サーバー・地形前処理のビルドと単体テストを確認済み。
Linux実機でのElectronウィンドウ・GPU描画は未確認。

| 種別 | 必須 | 備考 |
|---|---|---|
| OS | Windows 10/11 または Linux x64 | 以下の表・既存手順はWindows向け |
| Visual Studio 2022(Community可) | ✅ | 「C++によるデスクトップ開発」ワークロードを入れること。**CMake・vcpkgが同梱**されており、別途インストール不要 |
| Git | ✅ | サブモジュール取得に使用 |
| Rust ツールチェーン | ✅ | 未導入なら下記手順でセットアップする |
| PowerShell | ✅ | 本READMEのコマンド例はPowerShell(pwsh)想定 |

---

## Linuxでのセットアップ

Ubuntu 24.04 x64を基準に、リポジトリルートから実行する。
Rust/Cargoはrustupで導入済みであること。Windowsと同じ作業ツリーを共有する場合も、
CMakeの`build`ディレクトリはOSごとに別のチェックアウトで生成する。

```bash
sudo apt-get update
sudo apt-get install -y build-essential cmake ninja-build pkg-config git curl libssl-dev zlib1g-dev libgdal-dev
git submodule update --init sample/sim_server/third_party/uWebSockets
git -C sample/sim_server/third_party/uWebSockets submodule update --init uSockets libdeflate
rustup target add wasm32-unknown-unknown
cargo install trunk --locked

cmake -S sample/sim_server -B sample/sim_server/build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build sample/sim_server/build --parallel
ctest --test-dir sample/sim_server/build --output-on-failure
cmake -S tools/geotiff_preprocess -B tools/geotiff_preprocess/build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build tools/geotiff_preprocess/build --parallel
ctest --test-dir tools/geotiff_preprocess/build --output-on-failure
```

ALOSの入力GeoTIFFを`sample/map_data/`へ配置して前処理する。既に前処理済みなら省略できる。

```bash
./tools/geotiff_preprocess/build/geotiff_preprocess sample/map_data sample/sim_server/assets/terrain
./sample/sim_server/build/sim_server 9001 --host 127.0.0.1 --terrain-dir sample/sim_server/assets/terrain
```

別ターミナルでUIを起動し、WebGPU対応ブラウザで`http://localhost:8081`を開く。

```bash
cd sample/sim_frontend
NO_COLOR=true trunk serve
```

専用ウィンドウで起動する場合は[デスクトップ版のLinux手順](sim_desktop/README.md#linuxでの起動と配布)を使う。
LAN公開時はサーバーの`--host`を公開先に合わせる。公開ネットワークでの利用時は、リバースプロキシでTLSを終端する。
Linuxでも地形・通信・描画の仕様は共通で、vcpkgやPowerShellは不要。
`bash sample/ci/linux.sh`でC++テスト、Rust/WASM検証、実サーバー通信、UIビルドを実行する。地形が準備済みの場合は配布生成も行う。
ルートのGitHub Actionsはライブラリだけを検証する。
GPU描画・ウィンドウ操作の検証はCIの対象外。

## Windowsでのセットアップ手順(初回、ゼロから)

### 1. リポジトリの取得

```powershell
git clone <このリポジトリのURL> Sim3dView
cd Sim3dView
git submodule update --init sample/sim_server/third_party/uWebSockets
git -C sample/sim_server/third_party/uWebSockets submodule update --init uSockets libdeflate
```

サブモジュールは以下の2つ(`sample/sim_server/third_party/`配下):

- `uWebSockets`(uSockets含む) — HTTP/WebSocket静的配信・通信サーバー

> **`git submodule update --init --recursive`は使わないこと**: `uWebSockets`は`fuzzing/*`・
> `h1spec`(テスト用、不要)を、その子の`uSockets`はさらに`boringssl`・`lsquic`(TLS/QUIC用、
> 本プロジェクトはTLS/QUICを使わないため不要)をネストサブモジュールとして持っており、
> 素直に`--recursive`すると合計1GB近い不要なリポジトリを取得してしまう。上記のように
> **必要な範囲だけを個別に`--init`する**のが正しい手順(`uSockets`の`boringssl`/`lsquic`は
> 一切initしない=まったく取得しない)。

### 2. Rustツールチェーンの導入(未導入の場合のみ)

`rustc`/`cargo`があるか確認:

```powershell
cargo --version
```

無ければwingetで導入:

```powershell
winget install --id Rustlang.Rustup -e --silent --accept-package-agreements --accept-source-agreements
```

インストール後、新しいターミナルを開くか`$env:PATH`に`%USERPROFILE%\.cargo\bin`を通す。続けて
WASMターゲットと、Rustの開発用ビルドツール`trunk`を入れる:

```powershell
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
```

(`cargo install trunk`は初回のみ数分かかる。以後は再利用される)

このリポジトリはCargoワークスペース(ルートの`Cargo.toml`、メンバーは`sim3dview`・
`sample/sim_frontend`)になっているため、`cargo check -p sim3dview`のようにリポジトリ
ルートからどちらのcrateも操作できる。

```powershell
cargo fmt --check                                             # Rustコードの整形確認
cargo clippy --workspace --all-targets -- -D warnings         # ワークスペース全体の静的検査
cargo check -p sim3dview --target wasm32-unknown-unknown      # ライブラリ単体のコンパイル確認
cargo check --manifest-path sample/Cargo.toml -p sim_frontend --target wasm32-unknown-unknown   # サンプルアプリの統合コンパイル確認
cargo test --workspace                                         # 両crateの単体テスト(ネイティブで動く。ブラウザ不要)
```

単体テストは、合成した地形(`TerrainData::synthetic`)で標高サンプリング・LOD計画・カメラ(反転Z)・見通し計算・
視錐台カリングなどを検証し、WGSLシェーダーの構文とuniform/頂点のレイアウトも`naga`で確認する
(描画そのもの(GPU)は含まないので、画面の確認は下の「動作確認のポイント」の手順で行う)。

### 3. C++側のビルド(sim_server と 前処理ツール)

Visual Studio 2022同梱のCMake・vcpkgを使う。パスは環境によって多少変わるので、自分の環境の
Visual Studioインストール先に合わせて読み替えること。

```powershell
$cmake = "C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
$ctest = "C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\ctest.exe"
$toolchain = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\vcpkg\scripts\buildsystems\vcpkg.cmake"

& $cmake -S sample/sim_server -B sample/sim_server/build -G "Visual Studio 17 2022" -A x64 -DCMAKE_TOOLCHAIN_FILE="$toolchain"
& $cmake --build sample/sim_server/build --config Debug
& $ctest --test-dir sample/sim_server/build -C Debug --output-on-failure

# 地形データ前処理ツール(sim3dviewライブラリの一部。GDAL依存。sim_serverとは別のCMakeプロジェクト)
& $cmake -S tools/geotiff_preprocess -B tools/geotiff_preprocess/build -G "Visual Studio 17 2022" -A x64 -DCMAKE_TOOLCHAIN_FILE="$toolchain"
& $cmake --build tools/geotiff_preprocess/build --config Debug
& $ctest --test-dir tools/geotiff_preprocess/build -C Debug --output-on-failure
```

CTestは、シミュレーションの状態遷移、HTTP Range・ETag・安全なパス要素の判定、前処理の
タイル名・外接矩形・チャンク分割という、外部I/Oに依存しない中核ロジックを検証する。

初回のconfigure時にvcpkgが依存を自動ビルドする(sim_serverは`libuv`/`openssl`/`zlib`=`sample/sim_server/vcpkg.json`、
前処理ツールは`gdal`=`tools/geotiff_preprocess/vcpkg.json`)。**GDALのフルビルドだけで15分前後かかる**
(2回目以降はバイナリキャッシュが効いて数秒〜数十秒)。

ビルドが成功すると以下が生成される(出力先が2箇所に分かれる点に注意):

- `sample/sim_server/build/Debug/sim_server.exe` — シミュレーション本体+HTTP/WebSocketサーバー
- `tools/geotiff_preprocess/build/Debug/geotiff_preprocess.exe` — 地形データ前処理ツール

### 4. 地形データの生成(初回のみ・1回だけ実行)

`sim_server.exe`はHTTPで`/terrain/`以下の地形データ(`metadata.json`・`tile_index.json`・`base.bin`・
`tiles/L*/*.bin`)を配信するが、これらのファイルは**リポジトリに含まれておらず**、初回起動前に
前処理ツールで生成する必要がある。
**リポジトリのルートディレクトリから**実行すること(`sample/map_data/`と`sample/sim_server/assets/terrain/`を
相対パスで参照するため):

```powershell
tools\geotiff_preprocess\build\Debug\geotiff_preprocess.exe sample/map_data sample/sim_server/assets/terrain
```

`sample/map_data/`内のGeoTIFFタイルを自動検出し、**1度タイルごとに複数の解像度レベル**(約1.85km / 620m /
185m / 62m / 31m。最細は元データの30m)の標高グリッドを`sample/sim_server/assets/terrain/`へ書き出す
(`metadata.json`・`tile_index.json`・`base.bin`・`tiles/L1〜L4/*.bin`。レベル1以上は1度タイルを6x6の
チャンクに分けて連結した形式)。タイルを1枚ずつ並列に処理するので、メモリは数百MB/スレッド程度で、
390タイル(DSM約10GB)で数分〜十数分かかる(Release構成の実行を推奨)。出力は約12GB(`tiles/`が
ほぼ全部)で、`.gitignore`済み。対象範囲(外接矩形)はハードコードではなく、実際に見つかったタイルの
緯度経度から毎回自動計算されるため、`sample/map_data/`に別の場所のタイルを追加/削除してもコード変更は不要
(ただし`sim_server`は起動時に`metadata.json`を読むので再起動が必要)。
フロントは全タイルの最粗レベルだけを起動時に取得し、カメラに近いチャンクだけ細かいレベルをその都度
取得して描画する(地形LOD。[ライブラリ設計書](../docs/DETAILED_DESIGN.md) 6.10節)。
成功すると以下のようなログが出る:

```
[geotiff_preprocess] found 390 tile(s), bounds: lat 20..50, lon 120..150
[geotiff_preprocess] (1/390) N020E121
...
[geotiff_preprocess] wrote 390 tile(s) with 5 level(s) to sample/sim_server/assets/terrain
[geotiff_preprocess] elevation range: -330 .. 3937
```

存在しないタイル・各タイル内のNODATA画素・マスクファイル(`*_MSK.tif`、同梱)が海と示す画素は
「データなし」として出力され、Web UI側ではその部分の三角形を描画しない(背景の黒のまま見える。
標高0mとは区別される)。

---

## アプリの起動方法

従来のブラウザ版に加え、WindowsではElectronデスクトップ版も使える。
デスクトップ版は`sample/sim_desktop`で`npm ci`、`npm run build:ui`、`npm start`を実行すると、
C++サーバーと専用ウィンドウが一緒に起動する(C++のDebugビルドと前処理済み地形、Node.js 22.12以降が必要)。
配布用exeの生成や地形フォルダー指定は[デスクトップ版README](sim_desktop/README.md)を参照。
以下のブラウザ版の手順も従来どおり使え、デスクトップ版と同時起動できる(シミュレーション状態は別々)。

ビルドとデータ生成が済んだら、**2つのプロセスを同時に起動**する(別々のターミナルで)。

### ターミナル1: C++側(sim_server)を起動

```powershell
cd sample/sim_server
.\build\Debug\sim_server.exe
```

既定でポート9001のTCPでWebSocket (`ws://…/sim`) とHTTP地形配信(`/terrain/*`)を待ち受ける。
証明書の生成・指定は不要である。
`sample/sim_server/`ディレクトリから起動すること(`assets/terrain/...`を相対パスで開くため)。

> **Windowsの「アプリケーション制御ポリシー」に注意**: 環境によっては、`Start-Process`経由での
> exe起動がブロックされることがある。その場合は上記のように `.\build\Debug\sim_server.exe` を
> 直接呼び出す(`&`や`.\`で直接実行する)形にすれば問題ない。

### ターミナル2: Rust側(sim_frontend、sim3dviewライブラリを使うサンプルアプリ)の開発サーバーを起動

```powershell
cd sample/sim_frontend
$env:NO_COLOR = "true"   # 下記「既知の環境問題」参照。設定していないとtrunkがエラー終了する
trunk serve
```

ポート番号(8081)・待受アドレス(`0.0.0.0`、LAN上の別端末からもアクセスできるように全
インターフェースで待ち受ける)は`sample/sim_frontend/Trunk.toml`に設定済みなので、コマンドラインでの
指定は不要。初回はコンパイルに数十秒〜数分かかる(2回目以降は差分ビルドで数秒)。
`既定の8080番ポートはDocker Desktop/WSLが使用していることが多い`ため、本プロジェクトでは
8081番を使う運用にしている(競合する場合は空いている別のポート番号に変更してよい)。

### ブラウザで開く

```
http://localhost:8081
```

`sample/sim_frontend`は起動時に `ws://<ページのホスト名>:9001/sim` へWebSocket接続する。**sim_server.exeを
先に起動してから**ブラウザを開くこと(先にブラウザを開いても、自動再接続機能により後からsim_serverを
起動すれば数秒以内につながる)。

### LANの別端末から開くとき

`http://localhost`以外(`http://192.168.x.x:8081`等)は**セキュアコンテキストではない**ため、ブラウザは
WebGPUなど一部の機能を無効にする。LAN越しでWebGPUも使う場合は、ページ配信をTLS対応リバースプロキシへ置き、
そのプロキシで`/sim`をWebSocketとして中継する。

```powershell
# ターミナル1: sim_serverをHTTP/WebSocketで起動
cd sample/sim_server
./build/Debug/sim_server.exe --host 0.0.0.0

# ターミナル2: trunkを起動
cd sample/sim_frontend
$env:NO_COLOR = "true"
trunk serve
```

`http://<このPCのIP>:8081`で開く。HTTPとWebSocketを平文で公開する場合は、信頼できる閉域網だけで使うこと。

---

## 動作確認のポイント

正しく起動できていれば、ブラウザで以下が確認できる:

- 左上のステータスが「接続済み」になる
- 「原点」に緯度経度(既定値: 35.355556, 138.859722)が表示される
- 左下のVABでカテゴリ、ページ切り替え、スクリーンショット、画面録画、シミュレーションの開始・一時停止を操作できる
- 中央の地図パネルに3D地形が表示され、原点(富士山付近)のまわりに、デモの航跡(航空機・ヘリ・艦船・車両など7つのシンボルとラベル)が止まって見える
- VAB中段1ページ目の「開始」を押すとシミュレーションが動き出し(状態が「シミュレーション実行中」になる)、右上の「各種情報」の経過時間・高度・速度が変化し、航跡のシンボルが動いて軌跡が伸びる(VABの「一時停止」で止まる。実行中は原点を変更できない)
- 地図上の航跡のシンボルをクリックすると、シンボルに白い輪が付き、右上のパネルが「航跡情報」タブへ切り替わって詳細(種別・所属・位置・高度・針路・速度・原点からの距離)が出る。何もない所をクリックすると選択が外れる
- 地図を右クリックすると、その地点(や航跡のシンボル)に対するメニューが出る(レーダー観測点の追加・原点の設定・中心点の移動・図形の作成・緯度経度のコピー等)。「ここにレーダー観測点を追加」で観測点のピンと覆域(3Dは半透明のドーム、2D表示は塗り+輪郭線)が出る。「表示」メニューで、航跡のラベル・軌跡・高度線の表示切り替えや「作図デモ」ができる

---

## 既知の環境問題・トラブルシューティング

| 症状 | 原因・対処 |
|---|---|
| `trunk`実行時に`--no-color`関連のエラーで即終了する | 環境変数`NO_COLOR=1`がセットされているとtrunkの引数パーサ(`true`/`false`を期待)と衝突する。`$env:NO_COLOR = "true"`に設定してから実行する |
| `trunk serve`が`address already in use`(os error 10048)で失敗する | 既定の8080番ポートはDocker Desktop/WSLが使用していることがある。`sample/sim_frontend/Trunk.toml`で8081番に変更済み |
| `trunk serve`の起動ログが`server listening at:`の後、一部アドレス(`kubernetes.docker.internal`等)を数十秒おきに追加表示し続けて実際には繋がらない | 起動時のネットワークインターフェース・ホスト名列挙処理がDocker関連の仮想ネットワーク環境でハングすることがある。`Trunk.toml`で`disable_address_lookup = true`にして回避済み |
| `sim_server.exe`が起動しない/すぐ終了する | 別プロセスが既に9001番ポートを使っていないか確認(`netstat -ano \| findstr 9001`)。前のsim_serverプロセスが残っていないか確認する |
| vcpkgの`gdal`インストールが`libxml2`のビルドで失敗する | 本プロジェクトでは`tools/geotiff_preprocess/vcpkg.json`で`gdal`を`"default-features": false`にすることで、不要な`libxml2`(GML/KML用、Windowsで既知のIconv関連ビルド失敗がある)を回避済み。設定を変更していなければ発生しない |
| `geotiff_preprocess.exe`を実行しても`map_data`が見つからない | リポジトリの**ルートディレクトリ**から実行しているか確認する(引数に`sample/map_data`と`sample/sim_server/assets/terrain`を指定する) |
| GDALのビルドがとても遅い | 初回のみ発生(15分前後)。2回目以降はvcpkgのバイナリキャッシュが効くため数秒で終わる |
| `sim3dview`ライブラリ側だけを編集したのに、`trunk serve`が再ビルドせず古い表示のまま | Trunkはpath依存先(`sim3dview`)のソース変更を自動ではwatchしない。`trunk serve`を再起動する(サンプルアプリ側のファイルも一緒に変更していれば自動検知される) |
| `cargo`が「信頼されていないマウントポイントが含まれているため、パスをスキャンできません」で起動しない | `~/.cargo/bin/cargo.exe`がシンボリックリンクのため、環境によっては起動できない。`~/.rustup/toolchains/stable-x86_64-pc-windows-msvc/bin/cargo.exe`を直接実行する |
| 他端末からLAN経由でアクセスすると「接続中」のまま地図も出ない(このマシンのlocalhostでは正常) | Windows Firewallの受信許可ルールが`sim_server.exe`の旧パスを指したままの可能性が高い(exeを移動・再作成した後に起きる)。管理者権限のPowerShellで`Get-NetFirewallRule -DisplayName "sim_server.exe" \| Set-NetFirewallApplicationFilter -Program "<sim_server.exeの現在のフルパス>"`を実行してルールのパスを更新する。localhostはループバック通信のためFirewallの影響を受けず、この不一致に気付きにくい |
| ページは出るが「未接続」のまま/地形が出ない | `sim_server.exe`が起動済みで、ページと同じホストの9001番へHTTP/WS接続できるか確認する。HTTPSページから平文`ws://`へは接続できないため、公開時はTLS対応リバースプロキシでWebSocketを中継する |
| 開発中のBrowserペイン(Claude Codeの組み込みブラウザ)から`http://192.168.x.x:8081`(プライベートIP)へ接続すると`ERR_BLOCKED_BY_CLIENT` | ペイン側の制限でネットワーク疎通とは無関係。実疎通は`Test-NetConnection -ComputerName <IP> -Port 8081`/`-Port 9001`で確認する。ペインでの動作確認は`http://localhost:8081`で行う |
| ブラウザペインを非表示のままページを開くと、canvasが300×150のまま引き伸ばされて地形が歪む/欠ける | 非表示タブではResizeObserverが発火しないことがある(タブが可視になった時点で`TerrainView`が取り直す実装済み)。動作確認は実際にペインを表示した状態で行う |

---

## プロジェクトの現在の状態

C++シミュレータ本体・Web UI(サンプルアプリ)とも実装・動作確認済み。`sim3dview`ライブラリへの
分離(VAB・状況パネル・メニュー・通信プロトコルをアプリ固有部分として`sample/sim_frontend`側に
残し、3D地形描画・レーダー覆域計算・汎用UI部品をライブラリ化)も完了している。

## ドキュメント一覧

- [README.md](README.md) — 本書。セットアップ・起動手順
- [../README.md](../README.md) — `sim3dview`ライブラリの使い方(開発者向け)
- [docs/README.md](docs/README.md) — サンプルのドキュメントの索引
- [docs/DETAILED_DESIGN.md](docs/DETAILED_DESIGN.md) — サンプル設計書(地形の実データ、原点の運用、通信プロトコル、C++サーバー、業務UI、Electron、Mermaid図)
- [docs/tech_note.html](docs/tech_note.html) — 技術解説ノート sample編(通信・C++/Rustのコード・画面の部品・全処理の解説スナップショット)
- [docs/DEVELOPMENT_HISTORY.md](docs/DEVELOPMENT_HISTORY.md) — 実装の経緯・ハマりどころの記録(機能ごとの「要望→原因→修正→確認」)
- [../docs/DETAILED_DESIGN.md](../docs/DETAILED_DESIGN.md) — ライブラリ設計書(地形形式・描画・実装仕様・再実装ガイド)
- [THIRD_PARTY_NOTICE.md](THIRD_PARTY_NOTICE.md) — 利用しているサードパーティ(Rustクレート・C++ライブラリ・ALOS地形データ)の一覧・著作権表示・ライセンス。依存を変えたら`python sample/scripts/gen_third_party_notice.py`で再生成する
- [AGENTS.md](AGENTS.md) — AIエージェント向けの作業方針・要点(短い索引)

## デモモデルの再生成

リポジトリルートで `python sample/scripts/gen_sample_models.py` を実行する。
5種類のGLBを `sample/sim_frontend/assets/models/` に生成する。

## ファイル転送のサンプル

「ファイル」→「サーバーへファイル転送...」でファイルを選び、「送信」を押す。
1ファイル64 MiBまで。送信完了後に表示する`<ID>/data.bin`が保存ルートからの相対パス。
元ファイル名は保存せず、バイト列をそのまま保存する。同じファイルも再送ごとに別IDになる。
保存先はサーバーの作業ディレクトリにある`uploads/`（`--upload-dir <path>`で変更可能）。
受信物は自動実行・HTTP配信されない。サンプルは認証なしの開発用サーバー。

HTTP受信の統合テスト（実サーバーを一時ポートで起動し、専用一時ディレクトリに保存）:

```powershell
python sample/sim_server/tests/upload_integration.py sample/sim_server/build/Release/sim_server.exe
```
