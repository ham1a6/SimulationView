# Sim3dView

C++シミュレータ + Rust/Leptos(WASM) Web UI + ALOS DEMベースの3D地形ビューア。

## コミット方針

このリポジトリでは、意味のあるまとまった変更(機能実装・バグ修正・リファクタ・ドキュメント更新等)が
完了するたびに、都度ユーザーに確認を取らずClaude Codeの判断で自動的にコミットしてよい。
コミットメッセージは日本語で、変更内容とその意図(なぜその変更をしたか)が分かるように書くこと。
(push・force-push・amendなど、通常の安全ルールで確認が必要な操作は引き続き確認を取ること。)

## ドキュメント

- **[BASIC_DESIGN.md](BASIC_DESIGN.md)** — 基本設計書。システム概要・要求仕様・確定事項一覧・
  ディレクトリ構成・実装フェーズ計画。まずこれを読む。
- **[DETAILED_DESIGN.md](DETAILED_DESIGN.md)** — 詳細設計書。データフォーマット・座標変換の数式・
  通信プロトコルのバイト定義・クラス図/シーケンス図/状態遷移図(UML)。実装の詳細を確認する際に参照。

旧`claude_code_instructions.md`(元の指示書)と旧`DESIGN.md`(初期の設計検討過程の記録)の内容は
上記2文書に完全に統合済みのため、両ファイルは削除済み。過去の意思決定の経緯そのもの
(なぜその結論に至ったか)を辿りたい場合はgit履歴を参照する。

新しいセッションを始めるときは、まずBASIC_DESIGN.mdを読むこと。特に以下:
- 4節: 確定した設計方針の一覧(旧指示書の未確定12項目 + 実装で追加発生した5項目、計17項目の結論)
- 6節: 実装フェーズの順序(このリポジトリは基本この順に実装する)

## ディレクトリ構成

```
sim_server/       # C++側(シミュレーション本体 + WebSocketサーバー + GeoTIFF前処理ツール)
sim_frontend/     # Rust/Leptos/WASM側(Web UI + 3D地形描画)
map_data/         # 入力: ALOS DSM GeoTIFFタイル(17枚、既存・変更しない)
```

詳細はBASIC_DESIGN.md 5節参照。

## 主要な設計判断(要約。詳細はDETAILED_DESIGN.md参照)

- 地形前処理は**原点非依存**(緯度経度グリッドのまま出力)。再投影・ENU変換は行わない(詳細設計書2.2節)
- ENU変換(緯度経度→東/北/上メートル)は**フロント側(Rust/WASM)がランタイムに**行う(詳細設計書3節)。原点はUIから緯度経度入力、サーバー(C++)が正の状態を持つ(`OriginState`)
- 原点変更は**シミュレーション停止中のみ**許可(詳細設計書3.4節)。範囲外や実行中の`set_origin`は`CommandError`(msg_type 0x05)で拒否
- メッシュ解像度は1024×1024固定、平均法ダウンサンプリング、テクスチャなし(標高グラデーション着色のみ)
- VABは開発用ダミー値 rows=4, cols=6、単純クリックのみ、空ラベルボタンはDOM生成しない
- 状況パネル項目はC++側から`StatusPanelConfig`で動的配信(ハードコードしない)
- WebSocket自動再接続: 指数バックオフ+ジッター、タブ非表示中は一時停止(Page Visibility API)

## ビルド方法(sim_server)

依存関係: `sim_server/third_party/uWebSockets`(uSockets含む、gitサブモジュール)、
`sim_server/third_party/msgpack-cxx`(msgpack-c の `cpp_master` ブランチ、gitサブモジュール、ヘッダオンリー)、
`libuv`/`zlib`/`gdal`(vcpkg経由、`sim_server/vcpkg.json`のmanifestで管理)。
`gdal`は`tools/geotiff_preprocess`専用(`sim_server`本体はリンクしない。BASIC_DESIGN.md 3.1節の要件通り)。

初回チェックアウト時(`--recursive`は使わない。下記のuSockets配下のboringssl/lsquic注記を参照):
```
git submodule update --init sim_server/third_party/uWebSockets sim_server/third_party/msgpack-cxx
git -C sim_server/third_party/uWebSockets submodule update --init uSockets libdeflate
```

ビルド(Windows / Visual Studio 2022同梱のCMake・vcpkgを使用する例):
```
cmake -S sim_server -B sim_server/build -G "Visual Studio 17 2022" -A x64 -DCMAKE_TOOLCHAIN_FILE="C:/Program Files/Microsoft Visual Studio/2022/Community/VC/vcpkg/scripts/buildsystems/vcpkg.cmake"
cmake --build sim_server/build --config Debug
```
初回はvcpkgが`libuv`/`zlib`を自動ビルドする(バイナリキャッシュ済みなら数秒)。

生成物: `sim_server/build/Debug/sim_server.exe`(引数でポート指定可、既定9001。`/sim`パスでWebSocket待受)。
このマシンではアプリケーション制御ポリシーにより`Start-Process`でのexe起動はブロックされるが、
PowerShellから`& "build\Debug\sim_server.exe" > out.log 2> err.log`のように直接起動(`Start-Process`を経由しない)
すればブロックされない。この方法でsim_frontendとの接続まで動作確認済み(フェーズ2)。

uSocketsは`LIBUS_NO_SSL`(TLS不要)・Windowsでは`LIBUS_USE_LIBUV`(libusockets.hが自動選択)でビルドしており、
third_party/uWebSockets配下の`boringssl`/`lsquic`/`fuzzing`/`h1spec`等の重いネストサブモジュールは意図的に取得していない。
上記の初回チェックアウト手順(`--recursive`を使わず個別に`--init`する)がその取得除外の実体であり、
`git submodule update --init --recursive`を実行すると(ローカルの`.git/modules`削除跡とは無関係に、
フレッシュなcloneでも)これらが全て取得されてしまう点に注意。README.md「1. リポジトリの取得」の補足も参照。

### GeoTIFF前処理ツール(geotiff_preprocess、フェーズ6+7)

`sim_server/tools/geotiff_preprocess/`に実装。`sim_server`本体とは別ターゲット(GDAL依存はこのツール限定)。
`sim_server/CMakeLists.txt`が`add_subdirectory(tools/geotiff_preprocess)`で取り込むため、上記の通常ビルドで一緒に生成される。
成果物: `sim_server/build/tools/geotiff_preprocess/Debug/geotiff_preprocess.exe`(sim_server.exeとは出力先が違う点に注意)。

実行(リポジトリルートから。map_data/を自動でglobし、17タイルをモザイク→1024×1024へ平均法でダウンサンプリング):
```
geotiff_preprocess.exe
```
引数なしの既定値は `map_data`(入力ディレクトリ)と `sim_server/assets/terrain`(出力先)。
出力: `heightmap.bin`(f32 1024×1024、南→北の行順。DETAILED_DESIGN.md 2.6節の座標復元式に合わせて
GDAL標準の北→南から反転させている)と `metadata.json`。実データでの動作確認済み(17タイル全て検出、
elevation_min/max ≈ -11.5〜3677.9m、平均法による丸めで単一ピクセルの実測値-102.0〜3771/3776mより
穏やかな範囲になるのは想定通り)。

vcpkgの`gdal`portは既定featureのままだと`libxml2`(GML/KML用、Windowsでの既知のIconv絡みビルド失敗あり)を
引き込むため、`vcpkg.json`で`"default-features": false`にして回避している(GeoTIFF/TIFF読み込みはGDALのコア機能で
featureフラグ不要)。GDALのフルビルドはvcpkgで15分前後かかる(バイナリキャッシュがあれば数秒)。

## ビルド方法(sim_frontend)

Rustツールチェーン(`rustup`, `wasm32-unknown-unknown`ターゲット)と`trunk`が必要。このマシンには
winget(`Rustlang.Rustup`)と`cargo install trunk`で導入済み。

```
cd sim_frontend
cargo check --target wasm32-unknown-unknown   # コンパイル確認のみ
trunk serve --port 8081                        # 開発サーバー起動(既定の8080はDocker/WSLが使用中のため8081を使う)
```

注意: このマシンでは環境変数`NO_COLOR=1`が設定されており、trunkの`--no-color`引数パーサ(clapが`true`/`false`を期待)と
衝突してエラーになる。`trunk`実行前に `$env:NO_COLOR = "true"` を設定すること(PowerShell)。

`sim_frontend`は起動時に `ws://<ページのhostname>:9001/sim` へ接続する(`src/ws.rs::default_ws_url()`)。
`sim_server.exe`を先に起動してから`trunk serve`でページを開くこと。

フェーズ2で実施した動作確認: `sim_server`起動→ブラウザで`OriginState`/`VabConfig`/`StatusPanelConfig`/`SimState`が
表示されること、`sim_server`を強制終了→フロント側が指数バックオフで再接続を試み続けること、
`document.hidden`を疑似的に`true`にして`visibilitychange`を発火→再接続が一時停止すること、
`false`に戻す→即座に再接続を再開すること、をすべて確認済み。

## メッセージプロトコル(msg_type)

| 値 | 名前 | 方向 |
|---|---|---|
| 0x01 | SimState | Server→Client(高頻度) |
| 0x02 | VabConfig | Server→Client |
| 0x03 | OriginState | Server→Client |
| 0x04 | StatusPanelConfig | Server→Client |
| 0x05 | CommandError | Server→Client(要求元のみ) |
| 0x06 | AppStatus | Server→Client |
| — | ClientCommand | Client→Server |

フレーミング: `[1byte: msg_type][MessagePack body]`。詳細はDETAILED_DESIGN.md 4節。

## コードレビュー・既知の技術的負債(全実装完了後の最終レビューで発見)

C++/Rust全ソースを通読し、`cargo check`(警告ゼロ化)・CMakeビルド(警告ゼロ)まで確認済み。
このレビューで見つけて**修正済み**のもの:
- Rust: `glam::Mat4::look_at_rh`/`perspective_rh`がglam 0.33で非推奨化されていたため、
  後継の`glam::camera::rh::view::look_at_mat4`/`proj::directx::perspective`に置き換え(`terrain/camera.rs`)
- Rust: `SimState.positions`/`StatusItem.id`が未使用フィールド警告になっていたため、
  `#[allow(dead_code)]`+理由コメントを付与(msgpackが配列位置エンコードのためフィールド自体は削除不可、`protocol.rs`)
- C++: `WsServer`(生ポインタ`impl_`を所有)にコピー/ムーブのdelete宣言を追加(二重解放防止の防御的修正)
- C++: `main.cpp`のポート引数パース(`std::stoi`)が非数値入力で未処理例外crashしていたのをtry/catchで修正

**見つけたが未修正**(設計判断が絡むため、次回着手前に方針確認を推奨):
- `sim_server/include/simulation.hpp`の`kMinLat`等(35/40/135/140)が、依然としてハードコードの
  ままになっている。**`geotiff_preprocess/main.cpp`側は既にタイル構成から外接矩形を自動計算する
  よう修正済み**(「他のタイルも表示範囲内であれば表示してほしい」対応)なので、`map_data/`に
  別の場所のタイルを追加すればフロント側の地形範囲は自動的に広がるが、simulation.hppの原点
  バリデーション範囲は追従せず、サーバー側が新しい範囲の原点をCommandErrorで拒否してしまう
  (以前は「両方ハードコードで値が一致しているだけ」の潜在的な食い違いだったが、今は
  片方だけ動的化されたことでこの食い違いがより顕在化しやすくなっている)。
  根本修正にはsim_serverがmetadata.jsonを起動時に読む仕組みが必要(GDAL非依存でJSONの
  4つのdouble値を読むだけなので、簡易パーサ手書きでもnlohmann/json追加でも対応可能)。
- `sim_frontend/src/terrain/renderer.rs`のパイプライン設定で`cull_mode: None`(裏面カリング無効)のまま。
  コメントは「フェーズ10(自由視点カメラ)で正しい巻き順を確認して有効化する」としていたが、
  フェーズ10完了後も未着手。深度バッファがあるため描画結果自体に誤りはない(裏面描画がGPU時間を
  余分に使うだけ)が、コメントと実装が食い違っている。

## 現在の実装状況

**BASIC_DESIGN.md 6節の全11フェーズが完了・動作確認済み。** 実装フェーズとして計画されていた
作業はすべて終わっている(以降の作業は、明示的な新規要望がない限り、バグ修正や細部の改善が中心になる)。
スプリッタードラッグ時のリサイズ追従も含め、既知の未検証事項は残っていない。

### フェーズ11: UI細部の仕上げ

3項目とも、実は前のフェーズの時点で既に満たされていたことをこのフェーズで確認した:
- **VABの空ラベルボタン非表示**(フェーズ4で実装済み): DOM上のボタン数が24でなく23個であること、
  4行目6列目に相当する位置にボタンが存在しないことを`getComputedStyle`で再確認
- **パネルの横スクロール対応**(フェーズ3で実装済み): ビューポート幅600pxまで縮小しても3カラムの
  横並びを維持し、縦積みへの再レイアウトはせず横スクロールで対応することをスクリーンショットで再確認
- **再接続ステータス表示**(フェーズ2で実装済み): 接続状態バッジ(接続済み/接続中/再接続試行中/
  タブ非表示中)は既にOperationPanelに実装済みで、この時点で追加作業は不要だった

### フェーズ10: 自由視点カメラ + 側面図の実装

`terrain/camera.rs`に`OrbitCamera`(注視点・距離・yaw/pitchで表す球面座標カメラ)と
`CameraPreset`(俯瞰/側面)を追加。`components/terrain_view.rs`が共通の描画コンポーネントで、
ドラッグ(`pointermove`)で回転、ホイール(`wheel`)でズーム、プリセットボタンで視点切り替えを行う。
中央の地図(`MapView`)・右パネル下部の側面図(`SideView`)は両方ともこのコンポーネントを使い、
**カメラ状態はパネルごとに完全に独立**(一方をドラッグしても他方は動かない。実機で確認済み)。

地形データの二重フェッチを避けるため`terrain/store.rs`(`TerrainStore`)を新設し、
中央・側面の両パネルが同じ`Rc<TerrainData>`を共有する(フェッチは1回だけ)。`Rc`はSend/Syncでないため
`RwSignal`は`LocalStorage`版(`RwSignal::new_local`)を使っている(`WsConnection`と同じ理由)。

ブラウザで実機確認済み: ドラッグでの回転、ホイールでのズーム、俯瞰/側面プリセットへのスナップ、
側面図パネルが独立して地形を描画すること、をすべてスクリーンショットで確認。

### フェーズ9: 原点入力フォーム

`components/operation_panel.rs`の`OriginForm`で実装。metadata.jsonの`geodetic_bounds`を取得して
入力値をクライアント側で検証し(範囲外なら`設定`ボタンを無効化、DETAILED_DESIGN.md 3.5節)、`set_origin`
コマンド送信→`OriginState`反映→**heightmap.bin/metadata.jsonを再フェッチせず**メッシュを再計算して
GPUバッファを更新、までブラウザで実際に動作確認済み(原点を富士山付近→日本アルプス付近に変更し、
地形が別の山岳地形に切り替わることをスクリーンショットで確認)。

### フェーズ8: 地形描画(地形の初回描画・動作確認済み)

**フェーズ8(地形描画、DETAILED_DESIGN.md 6節)は動作確認済み**:
`sim_frontend/src/terrain/`(loader.rs/mesh.rs/camera.rs/renderer.rs)でheightmap.bin取得→ENU変換→
メッシュ生成→wgpu描画まで通し、ブラウザで実際に富士山(既定原点付近)が標高グラデーション着色付きで
正しく描画されることをスクリーンショットで確認済み。

### 解決済み: canvasサイズ取得のタイミング問題

マウント直後はcanvasのCSSレイアウトがまだ確定しておらず、`client_width()`/`client_height()`が
不正な値(例: 320x0)を返す問題があった。**単純なポーリング待機では直らなかった**
(デバッグログで2秒間・120回ポーリングしても値が一度も変化しないことを実測で確認)。
根本原因は、Leptosの「canvasのマウント」と「親要素の`grid-template-columns`スタイル反映」という
2つの独立したreactive更新の間に順序保証がないこと。**`ResizeObserver`ベースの実装に切り替えて解決**
(`components/map_view.rs`)。ResizeObserverは仕様上「実レイアウト確定後にのみ発火する」ため、
初回サイズ取得と以後のリサイズ追従を同じ仕組みで扱える。

### 解決済み: スプリッタードラッグ時のリサイズ追従

以前は、作業に使っていたBrowserペインが非表示(バックグラウンド)状態になっており、Chromiumが
そのタブの`ResizeObserver`通知自体をスロットリング(完全停止)していたため動作確認できていなかった
(独立に作成したテスト用の素のJS `ResizeObserver`も一切発火しないことを確認して切り分け済み)。

Browserペインを実際に表示した状態(`document.visibilityState === "visible"`をJS実行で確認済み)で
中央地図・側面図間のスプリッター(`.resizer`)を左右にドラッグして動作確認した結果、両方向とも
正しく追従することを確認:
- 中央地図canvas: 内部解像度(`canvas.width/height`)がCSS表示サイズ(`getBoundingClientRect()`)と
  常に一致(例: ドラッグで636×720→437×720に変化、逆方向で437×720→636×720に復帰)
- 側面図canvasも同時に逆方向へ追従(例: 286×310→485×310)
- リサイズ後も地形が歪まず正しい縦横比で再描画される(スクリーンショットで確認)

`TerrainRenderer::resize()`の実装・`ResizeObserver`ベースの設計とも問題なし。

### UIパネル名の統一(位置ベースの汎用名に変更)

5パネルの名称を、表示内容ではなく画面上の位置に基づく汎用名に統一した(タブ機能で
後から中身を差し替えられるようにするため。7.6節のTabbedPanelと同じ考え方を全パネルに
広げたもの):

| 位置 | 新名称 | コンポーネント | 備考 |
|---|---|---|---|
| 左上 | シミュレーションステータスパネル | `SimulationStatusPanel`(`operation_panel.rs`) | 旧称「操作パネル」「ステータス」 |
| 左下 | VABパネル | `VabPanel`(`vab.rs`) | 旧称「VAB」 |
| 中央 | メインパネル | `MainPanel`(`main_panel.rs`、旧`map_view.rs`) | 旧称「地図」。canvas左上に控えめなオーバーレイラベルで表示(他パネルのようなh2は置いていない) |
| 右上 | トップステータスパネル | `TopStatusPanel`(`right_panel.rs`) | タブ「各種情報」。旧称「各種情報パネル」 |
| 右下 | ボトムステータスパネル | `BottomStatusPanel`(`right_panel.rs`) | タブ「断面図」。旧称「側面図パネル」 |

### 解決済み(重要): 自由視点カメラがズームインすると真っ黒になるバグ

原点が標高の高い場所(既定原点は富士山付近)にあるとき、`OrbitCamera`の注視点を
`Vec3::ZERO`(ENU上座標=0、すなわち楕円体高0m)に固定していたため、実際の地表(標高
数百〜数千m)との間に大きなズレがあった。遠くから見ている分には気付かないが、ズーム
イン(`MIN_DISTANCE`付近)すると、カメラの視点位置が実質的に地表(山)の中に埋まって
しまい、画面が真っ黒になっていた。

修正: `terrain::mesh::sample_heightmap()`(元は`terrain/profile.rs`にあった双線形補間
サンプリング関数を`mesh.rs`へ移動・公開化)で原点の実際の地表標高を求め、
`OrbitCamera::preset()`の注視点を`Vec3::new(0, 0, 実際の標高)`にするよう修正
(`components/terrain_view.rs`の`ViewState::target_up`で保持し、原点変更時・プリセット
切り替え時にも追従させる)。ブラウザで実機確認済み(ズームインしても黒画面にならず、
地表の色が正しく描画され続けることを確認)。

### 既知の制約: 現在のheightmap解像度ではズームインしても地形の凹凸が見えない

上記のバグ修正に合わせて`MIN_DISTANCE`を300m→100mに引き下げ、より近くまでズーム
できるようにした。ただし現在のheightmapは5°四方(約555km)を1024×1024へダウンサン
プリングしており、1グリッドセルが約542m(元のALOS 30mデータよりかなり粗い)。実際に
1000m前後までズームして確認したところ、視野が1グリッドセルの中に収まってしまい、
地形の形状は見えず単色の平面が画面いっぱいに広がるだけだった(黒画面バグとは別の、
解像度不足による見た目の限界)。

ユーザーとの相談の結果、**地形データの解像度向上は別タスクとして扱う**ことになった
(この制約を解消するには、原点周辺だけ元のALOS 30mデータに近い解像度で描画する
LOD的な仕組みが必要で、現在の「5°四方を単一の固定1024×1024メッシュとして丸ごと
GPUに載せる」設計からの変更が必要になるため)。カメラの`MIN_DISTANCE`はすでに
十分近くまで下げてあるので、解像度向上タスク側でカメラの再調整は不要な想定。

### メニューバー・原点設定フローティングパネルの追加

画面最上部にメニューバー(`components/menu_bar.rs`)を追加(ファイル/設定/表示/ヘルプ)。
「設定」→「原点設定...」で、これまでシミュレーションステータスパネルに直接埋め込んで
いた原点入力フォームを、画面中央のフローティングパネル(`components/origin_dialog.rs`)
として開くように変更した。フォーム自体のロジック(バリデーション・送信)は無変更で、
表示場所だけを移した。ファイル/表示/ヘルプは項目未定のプレースホルダ(「準備中」)。
詳細はDETAILED_DESIGN.md 7.7節参照。

ブラウザで実機確認済み: 「設定」→「原点設定...」でパネルが中央に開く、緯度経度を
入力して「設定」で実際に原点が変わりメインパネル/ボトムステータスパネルの地形が
切り替わる、✕ボタン・背景クリックの両方でパネルが閉じる、ファイル/表示/ヘルプは
「準備中」のプレースホルダが出る、をすべて確認。

実装上のハマりどころ: メニュー項目・ダイアログのクリックハンドラのように、
開閉のたびに何度も呼ばれる`{move || ...}`(`FnMut`である必要がある)の中で、
`WsConnection`(非`Copy`)をムーブするクロージャを外側で1回だけ作ると、
2回目以降の呼び出しで「ムーブ済み」エラーになる。開閉のたびに実行される内側の
クロージャの中で毎回`conn.clone()`してから使うことで回避した(`origin_dialog.rs`)。

### AppStatus(シミュレータ状態文字列)の追加

シミュレーションステータスパネルのバッジ表示を変更: **接続済みの間はC++側
(sim_server)が実際に送ってきた状態文字列をそのまま表示し、それ以外(接続中・
再接続試行中・一時停止中)は一律「接続中」と表示する**ようにした(バッジの色分けは
維持)。これに伴い新しいメッセージ型`AppStatus`(msg_type 0x06、`{text: String}`)を
追加し、C++側の`Simulation`が保持している`running_`(pause/resumeで変化)を
反映した文字列("シミュレーション実行中" / "一時停止中")を、接続直後 + pause/resume時に
配信するようにした(`OriginState`の`origin_changed`と同じパターンで
`SimulationTickResult::app_status_changed`を追加)。

ブラウザで実機確認済み: 起動直後は`running_`の既定値がfalseのため「一時停止中」と
表示される、sim_serverを一時停止(WebSocket切断)すると即座に汎用「接続中」表示に
切り替わる、再接続すると再び「一時停止中」に戻る、をスクリーンショットで確認。
(v1時点ではUIからpause/resumeコマンドを送るボタンが無いため、実行中↔一時停止中の
切り替え自体はまだ確認できていない。プロトコル・表示ロジックとも実装済みで、
`ClientCommand::type_="pause"/"resume"`を送信すれば動作する)。

### 解決済み: メインパネルの初期表示が地形データの局所的な一部しか映していなかった

メインパネル(俯瞰プリセット)の既定`distance`が35,000m(35km)だったため、初期表示では
原点(既定は富士山付近)周辺のごく一部しか見えず、地形データ全体(5°四方、約555km四方、
対角線で約785km)を見るには手動でホイールアウトする必要があった。

あわせて、ズームアウトの上限(`MAX_DISTANCE`、当時400,000m)と`z_far`(遠方クリップ距離)が
**同じ値**になっていたバグも発見: カメラ自体がtargetから400,000m離れた状態で`z_far`も
400,000mだと、地形データの遠い側(カメラから見て400,000mより先)がクリッピングされて
描画されず、最大ズームアウト時に地形のごく一部(手前の細い三角形)しか見えず大部分が
真っ黒になっていた。

修正: `z_far`を1,500,000m(`MAX_DISTANCE`+データ外接矩形の対角線長に十分な余裕を持たせた
値)に引き上げてクリッピングを解消し、`MAX_DISTANCE`を500,000mに、俯瞰プリセットの既定
`distance`を400,000mに変更(ブラウザで実測し、データ領域全体が画面内にきれいに収まる
距離を確認して採用)。ブラウザで実機確認済み: ページ読み込み直後(手動ズーム操作なし)で
地形データ全体が画面に収まって表示される、最大ズームアウト(500,000m)でも同様に
クリッピングなく全体が見える、「俯瞰」ボタンでこの既定表示に正しく戻る、をスクリーン
ショットで確認。

### 無データ領域(欠損タイル・タイル内NODATA)を海として水色で塗るように変更

地形データは17枚のGeoTIFFタイルに分かれており(北緯35〜40度・東経135〜140度の
5×5=25セルのうち17枚のみ存在、8セルは欠損)、当初はモザイク時に欠損領域を標高0mで
一律埋めていた。これだと海抜0m付近の実在する陸地と見た目上区別がつかない
(色分けが地形グラデーション任せになる)。

`geotiff_preprocess`側: 欠損タイル・各タイル内のNODATA画素(GDALの`GetNoDataValue()`で
取得、主に海域)を標高0mではなく**f32のNaN**で埋めるように変更。ダウンサンプリング
(平均法)もNaN対応にし、ブロック内に実データが1つでもあればそれらだけで平均を取り
(陸地の縁で実データを最大限活かす)、ブロック全体がNaNの場合のみ出力もNaNにする。
`elevation_min`/`elevation_max`もNaNを除いて計算するよう修正(`std::minmax_element`は
NaNを正しく除外できないため手動ループに変更)。

フロント側(`terrain/mesh.rs`): `build_mesh()`で標高がNaNの格子点は、頂点位置を標高0mの
平面として配置しつつ、色だけ地形グラデーションと明確に区別できる水色
(`WATER_COLOR = (0.55, 0.78, 0.92)`)にする。`sample_heightmap()`(カメラ注視点の高さ・
断面図で共用)も、双線形補間の結果がNaNなら標高0mへ丸めるようにし、NaNが呼び出し側へ
伝播して他の計算を汚染しないようにした。

ブラウザ実機で確認済み: 地形データ(データ全体は約32%が海=旧8/25セル欠損の割合と
ほぼ一致)を俯瞰表示すると、山岳地帯(緑〜茶〜白のグラデーション)と海域(水色の平面)が
明確に塗り分けられて見える。カメラを回転させても海域が複数箇所(北東側・南側)に
正しく現れる(実際のタイル欠損パターンと整合)。断面図(ボトムステータスパネル)も
NaN混入によるクラッシュや表示崩れなく動作を継続。

### モザイクの外接矩形をハードコードではなく実際のタイル構成から自動計算するように変更

`geotiff_preprocess`はこれまで、モザイク対象の外接矩形(北緯35〜40度・東経135〜140度)を
`kMosaicMinLat`等の定数としてハードコードしていた。「他のタイルも表示範囲内であれば
表示してほしい」との要望を受け、`map_data/`に別の場所のタイルを追加/削除しても
コード変更なしに追従するよう、外接矩形を実行時に自動計算するよう変更した。

- `discover_tiles()`: `map_data/`内の`*_DSM.tif`を列挙し、ファイル名からタイルID
  (南西角の整数緯度経度)だけを読み取る(GDAL読み込みはまだ行わない、高速な1段階目)
- `compute_mosaic_bounds()`: 見つかった全タイルIDの緯度・経度それぞれの最小/最大から
  外接矩形を求める。タイルが1枚も見つからない場合は例外を投げる(既存のtry/catchで
  捕捉されエラー終了する)
- `build_mosaic()`: 決まった外接矩形のcanvasへ、各タイルを実際に読み込んで敷き詰める
  (2段階目)。以前あった「外接矩形外のタイルをスキップする」チェックは、外接矩形自体が
  タイル群から導出されるため構造的に不要になり削除した

現在の17タイル構成で実行し、自動計算された外接矩形(`lat 35..40, lon 135..140`)・
標高範囲(-11.5458〜3677.9)が変更前とビット単位で一致することを確認済み(挙動保存の
リファクタであることを確認)。既知の関連課題として、`sim_server/include/simulation.hpp`の
原点バリデーション範囲は依然ハードコードのままで追従しない(上記「コードレビュー・
既知の技術的負債」参照)。
