# Sim3dView 作業ガイド

Rust/Leptos(WASM)向けの3D地形ライブラリと地形前処理CLIを管理する。
このファイルはCodex向けの短い作業索引とし、仕様を重複して書かない。

## 最初に読むもの

1. [docs/DETAILED_DESIGN.md](docs/DETAILED_DESIGN.md) 0節で全体像と確定方針を確認する。
2. 変更対象の設計を同書1〜7節、実装仕様を9節で確認する。
3. 再実装や大規模変更では同書10節の依存順と受け入れ基準に従う。
4. 過去の判断理由や失敗例が必要な場合だけ[sample/docs/DEVELOPMENT_HISTORY.md](sample/docs/DEVELOPMENT_HISTORY.md)とgit履歴を読む。

| 用途 | 参照先 |
|---|---|
| 設計の唯一の正 | [docs/DETAILED_DESIGN.md](docs/DETAILED_DESIGN.md) |
| サンプルの作業手順 | [sample/AGENTS.md](sample/AGENTS.md) |
| `sim3dview`の公開API・組み込み方・CSS契約 | [README.md](README.md) |
| 開発経緯・過去のハマりどころ | [sample/docs/DEVELOPMENT_HISTORY.md](sample/docs/DEVELOPMENT_HISTORY.md) |
| ライセンス・著作権表示 | [THIRD_PARTY_NOTICE.md](THIRD_PARTY_NOTICE.md) |
| C++経験者向け技術解説 | [docs/tech_note.html](docs/tech_note.html) |

## 応答・変更・コミット

- ユーザーへの応答、コミットメッセージ、コードコメント、ドキュメントは日本語で書く。
- 既存の未コミット変更と未追跡ファイルはユーザーのものとして扱い、依頼に必要な範囲以外は変更しない。
- 意味のある変更単位が完成し、必要な検証が通ったら確認なしでコミットしてよい。変更内容と意図が分かる日本語のメッセージにする。
- push、force-push、amend、履歴改変、広範な削除は通常の安全ルールどおり事前確認する。
- 仕様変更では実装だけでなく設計書の対応節も更新する。定数・アルゴリズム・バイト配置は9節、設計理由は1〜7節を更新する。
- 依存を変えたら`python scripts/gen_third_party_notice.py`で`THIRD_PARTY_NOTICE.md`を再生成する。
- `docs/DETAILED_DESIGN.md`の既存節番号はソースコメントから参照されるため振り直さない。

## 責務とディレクトリ

```text
src/                      地形、カメラ、描画、覆域、作図、航跡、汎用UI
style/                    ライブラリCSS
tests/fixtures/           ライブラリ単体の検証データ
tools/geotiff_preprocess/  原点非依存な地形LOD前処理CLI
docs/                     ライブラリ設計と技術解説記事
scripts/                  ライブラリのライセンス生成
sample/                   独立した利用例。固有の手順はsample/AGENTS.md
```

`sim3dview`は通信プロトコルとサーバーURLを知らない。URL構築、WebSocket、プロトコル、原点状態の橋渡しはアプリ側の責務である。詳細は設計書0.4節・6節を参照する。

## 変更時に守る設計契約

- 地形前処理は原点非依存。ENU変換はRust/WASMが実行時に行う。
- 原点は呼び出し側から受け取る。カメラの注視点とは別物である。
- 地形は1度タイルLOD、レベル1以上は6×6チャンク。標高サンプリングは表示中チャンクのレベルを使う。
- 遠方の地表には地球の丸みを含む`ground_at_enu`を使う。`EnuTransform::inverse`は原点近傍用である。
- 海・欠損はNaN/`NO_DATA`で表し、その三角形は描画しない。水域は`fs_water`がWGS84楕円体との交点を画素ごとに求める。
- 描画は反転Z(`Depth32Float`、`Greater`、クリア0.0、有限far)、MSAA 4x、2倍スーパーサンプリング。射影変更時は`screen_to_ray`も整合させる。
- 作図は`DrawingState`、対話作成は`DrawToolState`、航跡は`TracksState`、3Dモデルは`ModelsState`を境界にする。
- VAB・状況パネル・メニュー・MessagePackプロトコルはサンプルアプリ固有で、ライブラリへ持ち込まない。

詳細な数値、バイト配置、アルゴリズムは必ず設計書9節とコードを確認し、この要約から推測しない。

## 検証

変更範囲に応じて次を実行する。

```powershell
cargo check -p sim3dview --target wasm32-unknown-unknown
cargo test -p sim3dview
```

- `cargo.exe`が信頼されていないマウントポイントのエラーになる場合は、`~/.rustup/toolchains/stable-x86_64-pc-windows-msvc/bin/cargo.exe`を直接使う。
- セキュリティやファイアウォール設定は変更せず、必要な手順をユーザーへ提示する。

## 実装上の注意

- Leptosの`view!`属性へ演算子を含む式を直接書かず、先に`let`へ束縛する。
- `Rc`を含む状態はローカルシグナルまたは`StoredValue::new_local`で扱う。公開コールバックは`UnsyncCallback`を使い、`unsafe impl Send/Sync`で回避しない。
- `Effect`内の`RefCell`借用はシグナル更新前に解放し、同期再実行による`BorrowMutError`を避ける。
- シェーダー全文を文書へ複製しない。WGSL変更時はnaga検証とRust/WGSL間のレイアウトテストを行う。

## 既知の技術的負債

- 地形パイプラインは`cull_mode: None`。有効化前に`mesh.rs::grid_indices`のスカート4辺の巻き順を確認し、効果を計測する。
- 極端に浅いカメラ角度では弱い縞模様が残る。完全解消にはLODまたはスーパーサンプリング倍率の再設計が必要である。
