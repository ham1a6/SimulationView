# Sim3dView

C++シミュレータ本体 + Rust(Leptos/WASM)製Web UI + ALOS全球数値地表モデル(DEM)による3D地形ビューア。

- C++プロセス(`sim_server`)がシミュレーションを実行し、WebSocketで状態を配信する
- ブラウザ上のRust/WASMアプリ(`sim_frontend`)がUIと3D地形描画を担当する
- 地形データは事前にGeoTIFF前処理ツール(`geotiff_preprocess`)で1回だけ生成する

詳しい設計は [BASIC_DESIGN.md](BASIC_DESIGN.md)(基本設計書)・[DETAILED_DESIGN.md](DETAILED_DESIGN.md)
(詳細設計書、UML図つき)を参照。開発環境固有の情報・既知の問題は [CLAUDE.md](CLAUDE.md) を参照。

```
sim_server/       C++側(シミュレーション本体 + WebSocketサーバー + GeoTIFF前処理ツール)
sim_frontend/     Rust/Leptos/WASM側(Web UI + 3D地形描画)
map_data/         入力: ALOS DSM GeoTIFFタイル(17枚。リポジトリに同梱済み)
```

---

## 動作環境

現時点で動作確認済みなのは **Windows + Visual Studio 2022** の組み合わせのみ。

| 種別 | 必須 | 備考 |
|---|---|---|
| OS | Windows 10/11 | |
| Visual Studio 2022(Community可) | ✅ | 「C++によるデスクトップ開発」ワークロードを入れること。**CMake・vcpkgが同梱**されており、別途インストール不要 |
| Git | ✅ | サブモジュール取得に使用 |
| Rust ツールチェーン | ✅ | 未導入なら下記手順でセットアップする |
| PowerShell | ✅ | 本READMEのコマンド例はPowerShell(pwsh)想定 |

---

## セットアップ手順(初回、ゼロから)

### 1. リポジトリの取得

```powershell
git clone <このリポジトリのURL> Sim3dView
cd Sim3dView
git submodule update --init sim_server/third_party/uWebSockets sim_server/third_party/msgpack-cxx
git -C sim_server/third_party/uWebSockets submodule update --init uSockets libdeflate
```

サブモジュールは以下の2つ(`sim_server/third_party/`配下):

- `uWebSockets`(uSockets含む) — WebSocket/HTTPサーバー
- `msgpack-cxx`(msgpack-cの`cpp_master`ブランチ) — MessagePackシリアライズ(ヘッダオンリー)

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

### 3. C++側(sim_server)のビルド

Visual Studio 2022同梱のCMake・vcpkgを使う。パスは環境によって多少変わるので、自分の環境の
Visual Studioインストール先に合わせて読み替えること。

```powershell
$cmake = "C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
$toolchain = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\vcpkg\scripts\buildsystems\vcpkg.cmake"

& $cmake -S sim_server -B sim_server/build -G "Visual Studio 17 2022" -A x64 -DCMAKE_TOOLCHAIN_FILE="$toolchain"
& $cmake --build sim_server/build --config Debug
```

初回のconfigure時にvcpkgが依存(`libuv`/`zlib`/`gdal`、`sim_server/vcpkg.json`で管理)を自動ビルドする。
**GDALのフルビルドだけで15分前後かかる**(2回目以降はバイナリキャッシュが効いて数秒〜数十秒)。

ビルドが成功すると以下が生成される(出力先が2箇所に分かれる点に注意):

- `sim_server/build/Debug/sim_server.exe` — シミュレーション本体+WebSocket/HTTPサーバー
- `sim_server/build/tools/geotiff_preprocess/Debug/geotiff_preprocess.exe` — 地形データ前処理ツール

### 4. 地形データの生成(初回のみ・1回だけ実行)

`sim_server.exe`はHTTPで`/terrain/heightmap.bin`・`/terrain/metadata.json`を配信するが、
これらのファイルは**リポジトリに含まれておらず**、初回起動前に前処理ツールで生成する必要がある。
**リポジトリのルートディレクトリから**実行すること(`map_data/`と`sim_server/assets/terrain/`を
相対パスで参照するため):

```powershell
sim_server\build\tools\geotiff_preprocess\Debug\geotiff_preprocess.exe
```

`map_data/`内のGeoTIFFタイル(現在17枚同梱済み)を自動検出し、モザイク→ダウンサンプリング
して`sim_server/assets/terrain/heightmap.bin`と`metadata.json`を書き出す(数秒〜数十秒)。
モザイクの対象範囲(外接矩形)はハードコードではなく、実際に見つかったタイルの緯度経度から
毎回自動計算されるため、`map_data/`に別の場所のタイルを追加/削除してもコード変更は不要。
成功すると以下のようなログが出る:

```
[geotiff_preprocess] found 17 tile(s), mosaic bounds: lat 35..40, lon 135..140 (18000x18000px)
[geotiff_preprocess] composited 17 tile(s) into 18000x18000 mosaic (missing cells left as NaN = ocean)
[geotiff_preprocess] wrote sim_server/assets/terrain\heightmap.bin and sim_server/assets/terrain\metadata.json
[geotiff_preprocess] elevation range: -22.0... .. 3710.8...
```

存在しないタイル・各タイル内のNODATA画素・マスクファイル(`*_MSK.tif`、同梱)が海と示す画素は
NaNとして出力され、Web UI側で地形グラデーションとは別の水色として塗り分けられる(標高0mとは
区別される)。

---

## アプリの起動方法

ビルドとデータ生成が済んだら、**2つのプロセスを同時に起動**する(別々のターミナルで)。

### ターミナル1: C++側(sim_server)を起動

```powershell
cd sim_server
.\build\Debug\sim_server.exe
```

既定でポート9001をWebSocket(`/sim`)とHTTP(`/terrain/*`)の両方で待ち受ける。
`sim_server/`ディレクトリから起動すること(`assets/terrain/...`を相対パスで開くため)。

> **Windowsの「アプリケーション制御ポリシー」に注意**: 環境によっては、`Start-Process`経由での
> exe起動がブロックされることがある。その場合は上記のように `.\build\Debug\sim_server.exe` を
> 直接呼び出す(`&`や`.\`で直接実行する)形にすれば問題ない。

### ターミナル2: Rust側(sim_frontend)の開発サーバーを起動

```powershell
cd sim_frontend
$env:NO_COLOR = "true"   # 下記「既知の環境問題」参照。設定していないとtrunkがエラー終了する
trunk serve
```

ポート番号(8081)・待受アドレス(`0.0.0.0`、LAN上の別端末からもアクセスできるように全
インターフェースで待ち受ける)は`sim_frontend/Trunk.toml`に設定済みなので、コマンドラインでの
指定は不要。初回はコンパイルに数十秒〜数分かかる(2回目以降は差分ビルドで数秒)。
`既定の8080番ポートはDocker Desktop/WSLが使用していることが多い`ため、本プロジェクトでは
8081番を使う運用にしている(競合する場合は空いている別のポート番号に変更してよい)。

### ブラウザで開く

```
http://localhost:8081
```

`sim_frontend`は起動時に `ws://<ページのホスト名>:9001/sim` へ自動接続する。**sim_server.exeを
先に起動してから**ブラウザを開くこと(先にブラウザを開いても、自動再接続機能により後からsim_serverを
起動すれば数秒以内につながる)。

---

## 動作確認のポイント

正しく起動できていれば、ブラウザで以下が確認できる:

- 左上のステータスが「接続済み」になる
- 「原点」に緯度経度(既定値: 35.355556, 138.859722)が表示される
- 左下のVAB(操作ボタン)が4行×6列(右下の1マスだけ空欄)で表示される
- 右上の「各種情報」に経過時間・高度・速度の数値がリアルタイムに変化する
- 中央の地図パネルに3D地形が表示される(**現在実装中の機能。詳細はCLAUDE.md参照**)

---

## 既知の環境問題・トラブルシューティング

| 症状 | 原因・対処 |
|---|---|
| `trunk`実行時に`--no-color`関連のエラーで即終了する | 環境変数`NO_COLOR=1`がセットされているとtrunkの引数パーサ(`true`/`false`を期待)と衝突する。`$env:NO_COLOR = "true"`に設定してから実行する |
| `trunk serve`が`address already in use`(os error 10048)で失敗する | 既定の8080番ポートはDocker Desktop/WSLが使用していることがある。`sim_frontend/Trunk.toml`で8081番に変更済み |
| `trunk serve`の起動ログが`server listening at:`の後、一部アドレス(`kubernetes.docker.internal`等)を数十秒おきに追加表示し続けて実際には繋がらない | 起動時のネットワークインターフェース・ホスト名列挙処理がDocker関連の仮想ネットワーク環境でハングすることがある。`Trunk.toml`で`disable_address_lookup = true`にして回避済み |
| `sim_server.exe`が起動しない/すぐ終了する | 別プロセスが既に9001番ポートを使っていないか確認(`netstat -ano \| findstr 9001`)。前のsim_serverプロセスが残っていないか確認する |
| vcpkgの`gdal`インストールが`libxml2`のビルドで失敗する | 本プロジェクトでは`sim_server/vcpkg.json`で`gdal`を`"default-features": false`にすることで、不要な`libxml2`(GML/KML用、Windowsで既知のIconv関連ビルド失敗がある)を回避済み。設定を変更していなければ発生しない |
| `geotiff_preprocess.exe`を実行しても`map_data`が見つからない | リポジトリの**ルートディレクトリ**から実行しているか確認する(既定のパスは`map_data`/`sim_server/assets/terrain`という相対パス) |
| GDALのビルドがとても遅い | 初回のみ発生(15分前後)。2回目以降はvcpkgのバイナリキャッシュが効くため数秒で終わる |

---

## プロジェクトの現在の状態

[BASIC_DESIGN.md](BASIC_DESIGN.md) 6節の実装フェーズのうち、フェーズ1〜7(通信基盤・UI骨組み・
VAB・C++側シミュレーションループ・地形前処理)は完了・動作確認済み。フェーズ8(Rust側での3D地形描画)は
実装中(詳細はCLAUDE.mdの「現在の実装状況」を参照)。

## ドキュメント一覧

- [README.md](README.md) — 本書。セットアップ・起動手順
- [BASIC_DESIGN.md](BASIC_DESIGN.md) — 基本設計書(要求仕様・確定した設計方針・全体構成)
- [DETAILED_DESIGN.md](DETAILED_DESIGN.md) — 詳細設計書(データフォーマット・プロトコル・UML図)
- [CLAUDE.md](CLAUDE.md) — 開発環境固有の申し送り事項(このリポジトリで作業するAIエージェント/開発者向け)
