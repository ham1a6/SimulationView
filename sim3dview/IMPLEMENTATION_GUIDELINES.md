# sim3dview 実装ガイドライン

ライブラリへの機能追加・修正と、アプリへの組み込みを行う開発者向けの作業指針。
公開APIの入口は[APIリファレンス](API_REFERENCE.md)、導入手順・CSSは[README](README.md)を参照する。
仕様を決めるときは[設計書](../docs/DETAILED_DESIGN.md)0節、変更対象の1〜7節、対応する9節を読む。
大規模変更・再実装は10節の依存順と受け入れ基準に従う。

## 責務に沿って変更箇所を選ぶ

| 変更内容 | 配置先 | 設計書 |
|---|---|---|
| 公開状態の初期化・context登録 | `src/viewer.rs` | 9.13.1 |
| 作図データ / 対話作成 / 航跡 / モデル設定 | `src/terrain/{drawing,draw_tool,tracks,models}` | 6.11〜6.13、9.11〜9.12、9.15 |
| 測地・地形サンプリング・距離測定 | `src/terrain/{geodesy,heightmap,measurement}` | 3、9.3 |
| メッシュ・LOD計画・GPU描画 | `src/terrain/{mesh,lod,renderer}` | 6.4〜6.10、9.4、9.7、9.9 |
| DOMイベント・状態と描画の接続 | `src/ui/terrain_view/` | 9.13 |
| 汎用パネル・ダイアログ・入力部品 | `src/ui/`と`style/sim3dview.css` | 7.6〜7.7、9.14 |
| URL構築・通信・受信データ変換・業務UI | `sample/sim_frontend/` | 4、6.2〜6.3、7 |
| 地形ファイル生成 | `../tools/geotiff_preprocess/` | 2、9.1、9.5 |

ライブラリはアプリが指定したURLから地形やモデルを取得するが、特定サーバーのURL構築や通信プロトコルを持たない。
VAB・状況パネル・業務メニューをライブラリへ移さない。
UIからの業務処理要求はコールバックで返し、アプリ側が送信とエラー表示を担当する。

## 公開APIを追加する

1. 呼び出し側が行いたい操作を状態型または関数として定義する。GPUや内部キャッシュの実体を公開する前に、操作・要求だけを渡せないか検討する。
2. 外部から使うものを`pub`、内部の連携を`pub(crate)`、モジュール内だけなら非公開にする。既存の呼び出し側とサンプルを検索して影響を確認する。
3. rustdocに入力の単位、座標系、前提、戻り値、エラー、更新時の副作用を書く。`None`や空配列、範囲外入力の意味を明示する。
4. contextが必須か任意かを決める。追加する任意機能はcontextなしの動作も定義する。`ViewerState`に追加する場合は生成と登録を揃える。
5. [APIリファレンス](API_REFERENCE.md)の索引・context表と、必要なREADMEの利用例を更新する。仕様変更は設計書の対応節にも反映する。

実装・コメント・文書は日本語で書く。API名、単位付きフィールド名、既存の命名規則は維持する。
既存公開APIの削除・引数変更では呼び出し側の移行方法を文書に残す。

## Leptosの状態・借用・寿命

共有状態はOwner内で生成し、子コンポーネントより前に登録する。
`ViewerState::provide()`と同じ型の個別登録を重ねない。`DrawingState`と`DrawToolState`は同じ一覧を共有させる。
通信で届いた原点や航跡はアプリ側の橋渡し処理から更新し、ライブラリ内で通信状態を参照しない。

- `Rc`を含む値は`RwSignal::new_local`または`StoredValue::new_local`で保持する。公開コールバックは`UnsyncCallback`を使い、`unsafe impl Send/Sync`で制約を回避しない。
- `Effect`内で`RefCell`を借用したままシグナルを更新しない。更新先が別Effectを同期実行し、同じ状態を借用する場合がある。
- 追従が必要な読み取りは`get`/`with`、一時的な参照は`get_untracked`/`with_untracked`を使い分ける。重い処理が位置更新ごとに実行されないよう、必要な値だけを`Memo`へ射影する。
- `view!`の属性に演算子を含む式を直接書かず、先に`let`へ束縛する。
- DOMリスナー・`ResizeObserver`・JSクロージャを作ったら`on_cleanup`で解放する。JS値はローカルな保存領域へ置く。`Closure::forget`でGPU状態を保持し続けない。

借用の終了位置を明示する例:

```rust,ignore
// stateはRc<RefCell<...>>、statusはシグナルとする。
{
    let mut state = state.borrow_mut();
    state.update_geometry();
}
status.set(String::new());
```

上記は借用の構造を示す擬似コード。実装例は`ui/terrain_view/mod.rs`のEffectと`frame.rs`を参照する。

## 非同期処理と描画の更新

地形取得は`TerrainStore`で共有する。パネルごとにmetadataやbaseを取り直さない。
GPU生成には地形データと有効なcanvasサイズの両方が必要なので、完了順を仮定しない。

覆域計算やモデル取得など、完了までに入力が変わる処理では、古い結果を適用しないよう世代や要求キーを確認する。
ビュー破棄後のコールバックは状態の生存を確認する。重いループは分割し、UIへ制御を戻す。
具体的な実装は`ui/terrain_view/{coverage,models,lod_driver}.rs`と設計書9.13節を参照する。

航跡などの高頻度更新は`render_frame`でフレーム要求をまとめる。
カメラ変更などLOD更新も必要な操作は`render_now`の経路を使う。
描画要求のたびに地形取得・全メッシュ構築・DOM全再生成を起こさない。
キャッシュのキーには結果に影響する原点・入力・表示中の地形レベルを必要に応じて含める。

## 座標・地形・GPUの契約を保つ

| 変更時に保つ性質 | 確認先 |
|---|---|
| 前処理は原点非依存。ENUへの変換は実行時。原点とカメラ注視点を分離 | 設計書2〜3、9.3、9.13 |
| 遠方地表は地球の丸みを含む`ground_at_enu`を使う。`inverse`は原点近傍用 | 9.3 |
| 標高は表示中チャンクのレベルからサンプリング。海・欠損と標高0の陸を区別 | 9.1、9.3〜9.4、9.7 |
| 欠損ノードを含む三角形を描かず、水面は画素ごとの楕円体交点で描く | 6.10、9.9 |
| 反転Z、有限far、MSAA、スーパーサンプリングの整合性を保つ | 9.6、9.9 |
| 射影の変更に`screen_to_ray`と水面のレイ基底を追従させる | 9.6、9.9 |
| Rust構造体とWGSLのuniform・頂点レイアウトを揃える | 9.9、9.15 |

定数・バイト配置・アルゴリズムはコードと設計書9節で確認し、この文書へ複製しない。
シェーダー全文も文書へ転記しない。地形のカリング有効化など既知の技術的負債に触れる場合は、
[AGENTS.md](../AGENTS.md)の注意点と既存の受け入れ基準を先に確認する。

## 検証と完了条件

変更範囲に合わせて、リポジトリのルートで実行する。

| 変更範囲 | 必要な確認 |
|---|---|
| API文書・rustdoc | 下記`cargo doc`、相対リンクと型名、サンプルとの一致 |
| ライブラリのRust実装 | 両crateのWASM checkと`cargo test -p sim3dview` |
| 公開API・context | 上記に加え、状態共有・任意contextなし・サンプルの利用箇所 |
| WGSL・GPU構造体 | `cargo test -p sim3dview`内のnaga検証とRust/WGSLレイアウトテスト |
| カメラ・描画・対話UI・CSS | 関連する単体テストとブラウザでの反復操作 |

```powershell
cargo doc -p sim3dview --no-deps --target wasm32-unknown-unknown
cargo check -p sim3dview --target wasm32-unknown-unknown
cargo check -p sim_frontend --target wasm32-unknown-unknown
cargo test -p sim3dview
```

Rustコード変更時は設計書9.13節の整形・lint・ワークスペース検証も行う。

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`cargo.exe`が信頼されていないマウントポイントのエラーになる場合は、
`~/.rustup/toolchains/stable-x86_64-pc-windows-msvc/bin/cargo.exe`を直接使用する。
セットアップの詳細は[ルートREADME](../README.md)を参照する。

ブラウザ確認では`$env:NO_COLOR = "true"`を設定して`sample/sim_frontend`でTrunkを起動する。
ライブラリだけを編集した場合もTrunkを再起動する。Browserペインを表示して`http://localhost:8081`を使い、
同じ操作を複数回確認する。変更に応じて2D/3D切替、パン・ズーム、原点変更、地形境界、
作図の開始・取消・再開、航跡の選択・消滅、タブ再表示を確認する。
セキュリティやファイアウォール設定は変更しない。

完了時は変更理由、検証結果、未確認の範囲を記録する。既存の未コミット変更は依頼に必要な範囲以外で触らない。
依存を変えたら`python scripts/gen_third_party_notice.py`を実行する。
仕様変更は設計書1〜7節の理由と9節の実装仕様を更新し、既存の節番号を振り直さない。
