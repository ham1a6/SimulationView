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
| システム概要・確定した設計方針(17項目)・実装フェーズ | [BASIC_DESIGN.md](BASIC_DESIGN.md)(新しいセッションはまずこれ。特に4節・6節) |
| データフォーマット・座標変換の数式・通信プロトコルのバイト定義・UML・各機能の仕様 | [DETAILED_DESIGN.md](DETAILED_DESIGN.md) |
| セットアップ・ビルド・起動手順・環境問題の対処(トラブルシューティング表) | [README.md](README.md) |
| `sim3dview`ライブラリの使い方(組み込み方・context・CSSテーマ契約) | [sim3dview/README.md](sim3dview/README.md) |
| 機能ごとの実装経緯(要望→調査→原因→修正→実機確認)・過去のハマりどころ | [DEVELOPMENT_HISTORY.md](DEVELOPMENT_HISTORY.md) |

設計判断の「なぜ」を辿りたいときは、上記に加えてgit履歴を参照する。

## ディレクトリ構成

```
sim3dview/            # ライブラリ本体(Rust/Leptos/WASM)。terrain/(データ取得・カメラ・wgpu描画・
                       # 覆域/見通し計算)+ ui/(TerrainView等のLeptosコンポーネント)
sample/
  sim_server/            # C++側(シミュレーション本体 + WebSocketサーバー + GeoTIFF前処理ツール)
  sim_frontend/          # sim3dviewを使うサンプルアプリ。VAB・状況パネル・メニュー・通信プロトコル等
map_data/             # 入力: ALOS DSM GeoTIFFタイル(17枚、既存・変更しない)
Cargo.toml             # ワークスペースルート(members: sim3dview, sample/sim_frontend)
```

ライブラリ/サンプルの責務分割の詳細は BASIC_DESIGN.md 5節・DETAILED_DESIGN.md 6節冒頭の対応関係表を参照。

## 主要な設計判断(要約)

- 地形前処理は**原点非依存**(緯度経度グリッドのまま出力)。ENU変換はフロント(Rust/WASM)がランタイムに行う(詳細設計書2.2・3節)
- **原点**(ENU座標系の基準、`OriginState`)はサーバー(C++)が正の状態を持ち、変更は**シミュレーション停止中のみ**
  (3.4節)。カメラの**中心点**(注視点`OrbitCamera::target`)は別物で、動かしても原点・メッシュ・観測点は変わらない
- メッシュ解像度は2048×2048固定(単一メッシュを丸ごとGPUへ。これ以上はLOD化が必要)、標高グラデーション着色のみ
- 海域はheightmapのNaN(`*_MSK.tif`の海+欠損タイル)で表し、フロントで水色(`WATER_COLOR`)に塗る。
  低地の陸地色は水色と紛らわしくない緑にしてある(`mesh.rs`)
- 描画は**反転Z**(Depth32Float、`depth_compare: Greater`、深度クリア値0.0、透視は有限far)+ MSAA 4x +
  2倍スーパーサンプリング。深度・射影を触るときは`camera.rs`の`screen_to_ray`(ピッキング)も整合させること
- 背景の海は実データの外側だけを覆うリング状スカート+保険の全面板(`mesh.rs`の`append_background_skirt`)
- VABは開発用ダミー値 rows=4, cols=6・空ラベルはDOM生成しない。状況パネル項目はC++から`StatusPanelConfig`で動的配信
- WebSocket自動再接続: 指数バックオフ+ジッター、タブ非表示中は一時停止(Page Visibility API)
- `sim3dview`は通信プロトコル・サーバーのURLを一切知らない(URL組み立て・原点のミラーはアプリ側の責務)

## ビルド・起動(要点。手順の詳細と環境問題の対処はREADME.md)

```
cargo check -p sim3dview --target wasm32-unknown-unknown      # ライブラリ単体
cargo check -p sim_frontend --target wasm32-unknown-unknown   # サンプルアプリ統合
cd sample/sim_frontend && trunk serve                          # 開発サーバー(ポート8081、Trunk.toml参照)
```

- `sim_server.exe`(既定ポート9001)を先に起動。地形データ(`heightmap.bin`/`metadata.json`)は
  `geotiff_preprocess.exe`をリポジトリルートから実行して生成する(リポジトリには含まれない)
- `trunk`実行前に`$env:NO_COLOR = "true"`が必要。`Start-Process`でのexe起動はブロックされるので直接実行する
- **ライブラリ(`sim3dview`)側だけを編集した場合はtrunkを再起動する**(path依存先は自動watchされない)
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
| — | ClientCommand | Client→Server |

フレーミング: `[1byte: msg_type][MessagePack body]`。詳細はDETAILED_DESIGN.md 4節。

## 実装時の落とし穴(過去に踏んだもの。詳細はDEVELOPMENT_HISTORY.md)

- Leptosの`view!`属性値に演算子を含む式を直接書かない(`let`で受けてから渡す。隣の属性が誤認識される)
- `{move || ...}`等のchildren位置のクロージャは`Send`境界を要求する。`Rc<RefCell<..>>`を直接捕捉できない
  (`WsConnection`のように`unsafe impl Send`するか、条件付き描画自体を避ける)
- 同じ詳細度のCSSクラスは宣言順が後ろの方が勝つ(VABの`.vab-button-active`は`.vab-button-dummy`より後ろに置く)
- 判断を1枚のスクリーンショットだけで下さない。同じ操作を複数回再現して確認する

## 既知の技術的負債(未修正。着手前に方針確認を推奨)

- `sample/sim_server/include/simulation.hpp`の原点バリデーション範囲(`kMinLat`等 35/40/135/140)がハードコードのまま。
  `geotiff_preprocess`は外接矩形を自動計算するので、`map_data/`にタイルを追加するとサーバーが新範囲の原点を
  `CommandError`で拒否する。根本修正にはsim_serverが`metadata.json`を起動時に読む仕組みが必要
- `sim3dview/src/terrain/renderer.rs`のパイプラインが`cull_mode: None`(裏面カリング無効)のまま。
  描画結果は正しいがGPU時間を余分に使う。コメント(巻き順を確認して有効化する)と実装が食い違っている
- VABの中段・下段はフロント側だけのダミーボタン(`vab_dummy_*`)で、C++は存在を知らない(`VabConfig`は先頭行のみ)。
  カテゴリごとに本当に別の操作をさせるには、サーバー側にカテゴリの概念を持たせ、別の`VabConfig`(または拡張プロトコル)を
  配信する設計変更が必要
- 極端に浅いカメラ角度では、2倍スーパーサンプリングでも弱い縞模様が残る(完全な解消にはLOD化か更に高倍率が必要)
