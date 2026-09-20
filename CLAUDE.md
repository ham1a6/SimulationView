# Sim3dView

C++シミュレータ + Rust/Leptos(WASM) Web UI + ALOS DEMベースの3D地形ビューア。
このファイルは作業方針と要点だけの短い索引。詳細は下記のドキュメントへ(長く書き足さないこと)。

## 応答言語

ユーザーへの応答(チャット上のテキスト)は常に日本語で行うこと。コミットメッセージ・
コード内コメント・ドキュメントも同様に日本語で書く(このリポジトリの既存の慣習通り)。

## コミット方針

意味のあるまとまった変更(機能実装・バグ修正・リファクタ・ドキュメント更新等)が完了するたびに、
都度ユーザーに確認を取らずClaude Codeの判断で自動的にコミットしてよい。コミットメッセージは日本語で、
変更内容とその意図(なぜその変更をしたか)が分かるように書く。
(push・force-push・amendなど、通常の安全ルールで確認が必要な操作は引き続き確認を取ること。)

## ドキュメント(どこに何があるか)

| 知りたいこと | 参照先 |
|---|---|
| 設計書の全体像(どの文書に何があるか・読み順・保守ルール) | [docs/README.md](docs/README.md) |
| システム概要・確定した設計方針(17項目)・実装フェーズ | [docs/BASIC_DESIGN.md](docs/BASIC_DESIGN.md)(新しいセッションはまずこれ。特に4節・6節) |
| データフォーマット・座標変換の数式・通信プロトコルのバイト定義・UML・各機能の設計方針(なぜ)、および9節=ライブラリの定数・アルゴリズム・バイト配置の要点(コードから起こした) | [docs/DETAILED_DESIGN.md](docs/DETAILED_DESIGN.md) |
| セットアップ・ビルド・起動手順・環境問題の対処(トラブルシューティング表) | [README.md](README.md) |
| `sim3dview`ライブラリの使い方(組み込み方・context・CSSテーマ契約) | [sim3dview/README.md](sim3dview/README.md) |
| 機能ごとの実装経緯(要望→調査→原因→修正→実機確認)・過去のハマりどころ | [docs/DEVELOPMENT_HISTORY.md](docs/DEVELOPMENT_HISTORY.md) |
| 利用しているサードパーティのライセンス・著作権表示(依存を変えたら`python scripts/gen_third_party_notice.py`で再生成) | [THIRD_PARTY_NOTICE.md](THIRD_PARTY_NOTICE.md) |
| ライブラリを1から再実装したい(エージェントへの実装依頼を含む) | [docs/IMPLEMENTATION_GUIDE.md](docs/IMPLEMENTATION_GUIDE.md)(フェーズ・受け入れ基準・落とし穴)と、詳細設計書9節(仕様の要点)。定数・アルゴリズムを変えたら9節の該当箇所を更新する(1〜8節に同じ数値を書き写さない。シェーダー全文はドキュメントに持たずソースが正) |
| C++経験者向けに、Rust・WASM・wgpu・各機能のしくみを土台から読みたい | 技術解説ノート(Artifact「Sim3dViewのしくみ」<https://claude.ai/artifact/1hpM7J9MELrr9K8PXsH8VR>。機能を足したら更新する)。HTMLのスナップショットは[docs/tech_note.html](docs/tech_note.html) |

設計判断の「なぜ」を辿りたいときは、上記に加えてgit履歴を参照する。

## ディレクトリ構成

```
sim3dview/            # ライブラリ本体(Rust/Leptos/WASM)。terrain/(データ取得・カメラ・wgpu描画・
                       # 覆域/見通し計算)+ ui/(TerrainView等のLeptosコンポーネント)
sample/
  sim_server/            # C++側(シミュレーション本体 + WebSocketサーバー)
  sim_frontend/          # sim3dviewを使うサンプルアプリ。VAB・状況パネル・メニュー・通信プロトコル等
tools/
  geotiff_preprocess/    # ライブラリの一部の前処理CLI(C++/GDAL)。GeoTIFF→1度タイルごとの多段解像度グリッド+metadata.json。独立CMakeプロジェクト
docs/                 # 設計書一式(基本設計・詳細設計(9節=ライブラリ実装仕様)・実装ガイド・開発履歴。索引はdocs/README.md)
scripts/              # ライセンス表記の生成
map_data/             # 入力: ALOS DSM GeoTIFFタイル(現在390枚、既存・変更しない。git管理外で履歴にも無い)
Cargo.toml             # ワークスペースルート(members: sim3dview, sample/sim_frontend)
```

ライブラリ/サンプルの責務分割の詳細は docs/BASIC_DESIGN.md 5節・docs/DETAILED_DESIGN.md 6節冒頭の対応関係表を参照。

## 主要な設計判断(要約)

- 地形前処理は**原点非依存**(緯度経度グリッドのまま出力)。ENU変換はフロント(Rust/WASM)がランタイムに行う(詳細設計書2.2・3節)
- **原点**(ENU座標系の基準、`OriginState`)はサーバー(C++)が正の状態を持ち、変更は**シミュレーション停止中のみ**
  (3.4節)。カメラの**中心点**(注視点`OrbitCamera::target`)は別物で、動かしても原点・メッシュ・観測点は変わらない
- 地形は**1度タイル単位のLOD+近いタイルは6x6チャンク**: 全タイルをチャンクで常駐し(最も粗くても
  レベル1=約620m/セル。起動直後だけレベル0=約1.85km/セルのタイル全体1枚)、近いチャンクほど細かいレベル
  (185m/62m/最細31m=元データの30m)をサーバーから取得して差し替える(`terrain/lod.rs`が計画、
  `ui/terrain_view/lod_driver.rs`が適用。チャンクの頂点は全タイルの下限を含めて合計2500万まで。画面外のメッシュは
  視錐台カリングで描かない)。大きいレベルはHTTP Rangeでチャンク1個分だけ取得する。標高サンプリングは各チャンクの
  「いま画面に出しているレベル」で引く。標高グラデーション着色+陰影(ヒルシェード、表示メニューでON/OFF、既定ON)。詳細はdocs/DETAILED_DESIGN.md 6.8節・6.10節
- 海域はheightmapのNaN(`*_MSK.tif`の海+欠損タイル)で表し、フロントはNaN頂点を含む三角形を描画しない。
  その「地形が無いところ」には**水域レイヤー**(WGS84楕円体の海抜0mの面)を水色で描く。メッシュではなく、
  画面いっぱいの三角形1枚で各画素の視線と楕円体の交点をシェーダーで直接求め(`terrain.wgsl`の`fs_water`)、
  地形メッシュより先に、楕円体との交点の深度(視線方向に1km奥へずらした値)を書いて描く。地球本体の向こう側や
  海面の下の地形は隠れ(水面越しに透けない)、標高0m以下の地形は余裕の範囲で水域より手前になり隠れない。
  視線が楕円体に当たらない画素(空)は黒(docs/DETAILED_DESIGN.md 6.10節)。以前(メッシュ方式)は撤去した経緯あり
- 地形は楕円体(WGS84相当)をENUへ変換した曲面で、遠方ほど丸みで下がる(原点から1,000kmで約80km)。
  遠方の地表の高さ・クリック判定は標高ではなく`heightmap.rs`の`ground_at_enu`(丸み込みのENU上座標)を使うこと。
  `EnuTransform::inverse`は原点近傍の接平面近似なので遠方には使わない。2Dモードの奥行き範囲
  (`camera.rs`の`ORTHO_DEPTH_RANGE_M`)も丸みを含めて決めてある
- サーバーの原点受理範囲は起動時に`assets/terrain/metadata.json`の`geodetic_bounds`から読む
  (`simulation.cpp`。読めなければチェック無効+警告)。`map_data/`を変えたらsim_serverの再起動が必要
- 描画は**反転Z**(Depth32Float、`depth_compare: Greater`、深度クリア値0.0、透視は有限far)+ MSAA 4x +
  2倍スーパーサンプリング。深度・射影を触るときは`camera.rs`の`screen_to_ray`(ピッキング)も整合させること
- **作図**(図形・線)は`terrain::drawing::DrawingState`(context)で出し入れする。位置の種類で固定先を選ぶ:
  `World`(緯度経度+高度、地形と同じ深度)/`View`(カメラからの相対m)/`Screen`(画面px)。カメラ固定は地形と別の2つ目のパスで描く。
  線の太さはシェーダーで画面pxへ展開(docs/DETAILED_DESIGN.md 6.11節、`sim3dview/README.md`に使い方)。ユーザーが地図をクリックして図形を作る機能は`terrain::draw_tool::DrawToolState`+`ui::drawing_editor::DrawingEditor`
  (作成中の図形は`DrawingState`に仮の要素として置く。作った図形はlocalStorageへ保存。サンプルは表示メニューの「作図...」で開く移動可能なウインドウ)
- **右クリックメニュー**は`ui::context_menu`(汎用。項目は使う側が渡す)。地図の右クリックは`ui::context_menu::MapMenuState`に項目を作るコールバックを渡す
  (サンプルは`components/map_menu.rs`。観測点の追加もここの1項目。`MapMenuState`/`ContextMenuState`が無ければ従来どおり右クリックで観測点を追加)
- **航跡**(航空機・艦船・車両等の現在位置)は`terrain::tracks::TracksState`(context)へアプリが`set`する(サンプルは`track_bridge.rs`が
  `TrackList`を変換)。シンボルは向きつきビルボード(画面サイズ固定、進行方向が画面上の実際の向きを指す)、ラベルはHTML要素の重ね合わせ
  (docs/DETAILED_DESIGN.md 6.12節)。シンボルのクリックで`TracksState::selected`が変わり、詳細はアプリ側で表示(サンプルはトップパネルの「航跡情報」タブ)
- VABは開発用ダミー値 rows=6, cols=4(先頭行=カテゴリ、中段4行、下段1行)・空ラベルはDOM生成しない。状況パネル項目はC++から`StatusPanelConfig`で動的配信
- WebSocket自動再接続: 指数バックオフ+ジッター、タブ非表示中は一時停止(Page Visibility API)
- `sim3dview`は通信プロトコル・サーバーのURLを一切知らない(URL組み立て・原点のミラーはアプリ側の責務)

## ビルド・起動(要点。手順の詳細と環境問題の対処はREADME.md)

```
cargo check -p sim3dview --target wasm32-unknown-unknown      # ライブラリ単体
cargo check -p sim_frontend --target wasm32-unknown-unknown   # サンプルアプリ統合
cargo test -p sim3dview                                        # 単体テスト(ネイティブで動く。合成地形`TerrainData::synthetic`を使う)
cd sample/sim_frontend && trunk serve                          # 開発サーバー(ポート8081、Trunk.toml参照)
```

- `sim_server.exe`(既定ポート9001)を先に起動。地形データ(`metadata.json`/`tile_index.json`/`base.bin`/`tiles/`、約12GB)は
  `geotiff_preprocess.exe`をリポジトリルートから実行して生成する(リポジトリには含まれない)
- HTTPS/WSSは任意(既定は平文)。`tools/gen_dev_cert.ps1`で自己署名証明書を作り、sim_serverに`--cert/--key`、trunkに`--tls-cert-path/--tls-key-path`を渡す。
  フロントはページのスキームに合わせて`wss/https`で接続する(両方TLSにすること。手順はREADME.md)
- `trunk`実行前に`$env:NO_COLOR = "true"`が必要。`Start-Process`でのexe起動はブロックされるので直接実行する
- **ライブラリ(`sim3dview`)側だけを編集した場合はtrunkを再起動する**(path依存先は自動watchされない。サンプル側のファイルを`touch`しても再ビルドされる)
- `cargo.exe`が「信頼されていないマウントポイント」で起動しない場合は
  `~/.rustup/toolchains/stable-x86_64-pc-windows-msvc/bin/cargo.exe`を直接実行する
- UIの動作確認はBrowserペインを**表示した状態**で行う(非表示だとResizeObserverが発火しない)。
  プライベートIP(192.168.x.x)宛はペイン側でブロックされるので`http://localhost:8081`で確認する
- ファイアウォール等のセキュリティ設定変更は自分では実行せず、ユーザーに手順を提示する

## メッセージプロトコル(msg_type)

| 値 | 名前 | 方向 |
|---|---|---|
| 0x01 | SimState | Server→Client(高頻度) |
| 0x02 | VabConfig | Server→Client |
| 0x03 | OriginState | Server→Client |
| 0x04 | StatusPanelConfig | Server→Client |
| 0x05 | CommandError | Server→Client(要求元のみ) |
| 0x06 | AppStatus | Server→Client |
| 0x07 | TrackList | Server→Client(航跡。進行中は約20Hz) |
| — | ClientCommand | Client→Server |

フレーミング: `[1byte: msg_type][MessagePack body]`。詳細はdocs/DETAILED_DESIGN.md 4節。

## 実装時の落とし穴(過去に踏んだもの。詳細はdocs/DEVELOPMENT_HISTORY.md)

- Leptosの`view!`属性値に演算子を含む式を直接書かない(`let`で受けてから渡す。隣の属性が誤認識される)
- `{move || ...}`等のchildren位置のクロージャは`Send`境界を要求する。`Rc<RefCell<..>>`を直接捕捉できない
  (`StoredValue::new_local`で包んだ`Copy`のハンドルを捕捉する。サンプルの`WsHandle`。`unsafe impl Send`は使わない)。
  公開コールバックは`Send`不要の`UnsyncCallback`(`Callback`ではない)
- 同じ詳細度のCSSクラスは宣言順が後ろの方が勝つ(VABの`.vab-button-active`は`.vab-button-dummy`より後ろに置く)
- 判断を1枚のスクリーンショットだけで下さない。同じ操作を複数回再現して確認する

## 既知の技術的負債(未修正。着手前に方針確認を推奨)

- `sim3dview/src/terrain/renderer/pipelines.rs`のパイプラインが`cull_mode: None`(裏面カリング無効)のまま。
  地形は高さ場で裏面はほとんど映らないため、有効化しても効果は小さい見込み(要計測)。有効化するなら、先にスカート
  (縁の壁、`mesh.rs`の`grid_indices`)の巻き順が4辺で揃っているか確認すること(揃っていないとクラックが出うる)
- VABの中段・下段はフロント側だけのダミーボタン(`vab_dummy_*`)で、C++は存在を知らない(`VabConfig`は先頭行のみ)。
  カテゴリごとに本当に別の操作をさせるには、サーバー側にカテゴリの概念を持たせ、別の`VabConfig`(または拡張プロトコル)を
  配信する設計変更が必要
- 極端に浅いカメラ角度では、2倍スーパーサンプリングでも弱い縞模様が残る(完全な解消にはLOD化か更に高倍率が必要)
