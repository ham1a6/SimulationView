# Sim3dView 設計書一式

セットアップ・起動手順は[ルートのREADME.md](../README.md)。ここは**設計書の索引**で、どの文書に何が書いてあるか、どの順に読むか、どう保守するかを示す。

## 文書体系

設計は3層で、上の層ほど「何を・なぜ」、下の層ほど「どう作るか(正確な数値・手順)」を持つ。同じ事実は1か所にだけ書き、他は参照する。

| 層 | 文書 | 内容 | 正となるもの |
|---|---|---|---|
| 基本設計 | [BASIC_DESIGN.md](BASIC_DESIGN.md) | 背景・システム構成・技術スタック・**確定した設計方針(17項目)**・ディレクトリ構成・実装フェーズ | 設計判断(なぜそうしたか)の要約 |
| 詳細設計(システム) | [DETAILED_DESIGN.md](DETAILED_DESIGN.md) | 地形データの実態・前処理の方針・座標系(ENU)・**通信プロトコル**・C++サーバー・ライブラリの設計方針(モジュール構成・図)・サンプルアプリのUI | プロトコル(4節)・C++(5節)・サンプルUI(7節)、および方針・理由・UML図 |
| 詳細設計(ライブラリ実装仕様) | [impl/](impl/)(第1〜5部) | 定数・アルゴリズム・バイト配置・WGSL全文・context・CSS契約・テスト。**実装コードから起こした**正確な仕様 | 数値・手順・バイト定義(食い違ったらこちらが正) |
| 実装ガイド | [IMPLEMENTATION_GUIDE.md](IMPLEMENTATION_GUIDE.md) | ドキュメントだけでライブラリを1から再実装するための入口(フェーズ・受け入れ基準・落とし穴チェックリスト) | フェーズ表・落とし穴 |
| 履歴 | [DEVELOPMENT_HISTORY.md](DEVELOPMENT_HISTORY.md) | 機能ごとの実装経緯(要望→調査→原因→修正→実機確認)・過去のハマりどころ | 経緯(当時の記述。現状の仕様ではない) |

ルート直下の[README.md](../README.md)(セットアップ・ビルド・トラブルシューティング)・[CLAUDE.md](../CLAUDE.md)(AIエージェント向けの作業方針)・
[sim3dview/README.md](../sim3dview/README.md)(ライブラリの使い方=公開API)・[THIRD_PARTY_NOTICE.md](../THIRD_PARTY_NOTICE.md)(ライセンス表記)は設計書ではなく、利用者・作業者向け。

## 読み順

| 知りたいこと | 読むもの |
|---|---|
| まず全体像・確定方針を知りたい | [BASIC_DESIGN.md](BASIC_DESIGN.md) 4節(確定17項目)・6節 |
| データ形式・座標変換・通信プロトコル・UML | [DETAILED_DESIGN.md](DETAILED_DESIGN.md) |
| ある機能の「なぜ」と全体像(LOD・水域・覆域・作図・航跡…) | [DETAILED_DESIGN.md](DETAILED_DESIGN.md) 6節 |
| ある機能の定数・式・バイト配置・シェーダー | [impl/](impl/)の該当部(対応表は下) |
| ライブラリを使うアプリを書きたい | [sim3dview/README.md](../sim3dview/README.md) |
| ドキュメントだけでライブラリを再実装したい | [IMPLEMENTATION_GUIDE.md](IMPLEMENTATION_GUIDE.md) → [impl/](impl/)をフェーズ順に |
| ある機能がなぜ今の形になったか(過去の失敗・却下した案) | [DEVELOPMENT_HISTORY.md](DEVELOPMENT_HISTORY.md)・git履歴 |

### 詳細設計と実装仕様の対応

| 実装仕様 | 範囲 | 対応する詳細設計の節 |
|---|---|---|
| [第1部 データ・測地・メッシュ・前処理](impl/1_data_and_geodesy.md) | ファイル契約、`fetch/loader/geodesy/heightmap/mesh`、`geotiff_preprocess` | 1〜3節・6.6〜6.7節 |
| [第2部 カメラ・LOD・ピッキング・見通し](impl/2_lod_camera_los.md) | `camera/lod/pick/profile/los`(純粋関数) | 6.6・6.9・6.10節 |
| [第3部 レンダラー・シェーダー](impl/3_renderer_and_shaders.md) | `renderer/*`、`vertex`、`terrain.wgsl`・`draw.wgsl` | 6.4・6.8・6.10(水域)・6.11(描画パス) |
| [第4部 重ね描き](impl/4_overlays.md) | `render_bias/markers/drawing/drawing_geometry/draw_tool/tracks` | 6.9・6.11・6.12節 |
| [第5部 UI・統合](impl/5_ui_and_integration.md) | `ui/*`、context、`TerrainView`のイベント・Effect・LOD適用ループ、CSS契約、crate構成 | 6.0〜6.1・6.10(適用)・7.6〜7.7節 |

通信プロトコル(DETAILED_DESIGN 4節)・C++サーバー(5節)・サンプルアプリのVAB/状況パネル/メニュー(7節)は`sample/`の設計で、実装仕様の対象外。

## 保守ルール

- **同じ事実を2か所に書かない**。数値・アルゴリズム・バイト配置は実装仕様(`impl/`)に書き、詳細設計書には方針と理由だけを書く。食い違いを見つけたら実装(コード)→`impl/`→詳細設計書の順に確かめて直す
- **定数・アルゴリズムを変えたら**、対応する`impl/`の記述(と[IMPLEMENTATION_GUIDE.md](IMPLEMENTATION_GUIDE.md)のテスト名表)を更新する。設計上の理由が変わるときは[DETAILED_DESIGN.md](DETAILED_DESIGN.md) 6節も直す
- **シェーダー(`terrain.wgsl`・`draw.wgsl`)を編集したら**、`python scripts/sync_impl_wgsl.py`で`impl/3`の埋め込みWGSLを同期する(`--check`はコミット前の確認用)
- **依存を変えたら**`python scripts/gen_third_party_notice.py`で[THIRD_PARTY_NOTICE.md](../THIRD_PARTY_NOTICE.md)を再生成する
- **詳細設計書の節番号はソースのコメントから参照されている**(例: `DETAILED_DESIGN.md 6.10節`)。節を足すときは枝番にし、番号を詰め直さない
- 機能の経緯(要望・原因・失敗した案)は[DEVELOPMENT_HISTORY.md](DEVELOPMENT_HISTORY.md)に足し、詳細設計書には結論だけを書く
- 機能を足したら、ルートの[CLAUDE.md](../CLAUDE.md)の要点・[sim3dview/README.md](../sim3dview/README.md)の使い方・技術解説ノート(Artifact「Sim3dViewのしくみ」)も更新する
- 設計書はコミットメッセージ・コメントと同じく日本語で書く
