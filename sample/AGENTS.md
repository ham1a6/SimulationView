# サンプル実装の作業ガイド

共通方針は[ルートAGENTS.md](../AGENTS.md)、設計は[サンプル設計書](docs/DETAILED_DESIGN.md)、ビルド・起動は[README](README.md)、過去の判断は[開発履歴](docs/DEVELOPMENT_HISTORY.md)を参照する。サンプル固有の文書は`sample/docs/`に置き、ルートの`docs/`へ書かない。

- `sim_frontend/` は通信・VAB・状況パネル・メニュー、`sim_server/` はC++参照サーバー、`sim_desktop/` はElectron起動・配布。
- Cargoワークスペース・ロック・ビルド出力はsample内で完結させる。ライブラリは `path = "../.."` で参照する。
- `map_data/` は入力GeoTIFF、`certs/` は開発用証明書。git管理外で、依頼なしに内容を変更しない。
- 原点の正はC++のOriginState。変更はシミュレーション停止中だけで、カメラの注視点とは別。
- VAB中段・下段はフロントだけのダミー。C++はカテゴリを知らず、本対応にはプロトコルの設計変更が必要。
- 同じ詳細度のCSSは後勝ち。`.vab-button-active` は `.vab-button-dummy` より後に置く。
- Electron配布版の地形は `resources/terrain` に同梱する。フォルダー選択や保存設定に依存させない。
- 依存変更時はルートで `python sample/scripts/gen_third_party_notice.py` を実行する。

## 検証

ルートで `cargo check --manifest-path sample/Cargo.toml -p sim_frontend --target wasm32-unknown-unknown`。
C++はCMake/CTest、Electronは `sim_desktop/` で `npm test` と `npm run build:ui`。Linux一括検証は `bash sample/ci/linux.sh`。
Trunk実行前は `$env:NO_COLOR = "true"` を設定し、`sim_frontend/` で起動する。ライブラリだけを編集してもTrunkを再起動する。
UI確認はBrowserペインを表示して `http://localhost:8081` で同じ操作を複数回確認する。
セキュリティやファイアウォール設定は変更しない。
