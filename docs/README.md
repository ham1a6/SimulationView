# Sim3dView ドキュメント

設計は[Sim3dView設計書](DETAILED_DESIGN.md)の1冊に統合している。

| 文書 | 内容 |
|---|---|
| [DETAILED_DESIGN.md](DETAILED_DESIGN.md) | システム概要、設計方針、詳細設計、実装仕様、再実装ガイド。設計の唯一の正 |
| [DEVELOPMENT_HISTORY.md](DEVELOPMENT_HISTORY.md) | 要望、調査、失敗した方式、修正、実機確認の履歴。現行仕様の正ではない |
| [../README.md](../README.md) | セットアップ、ビルド、起動、トラブルシューティング |
| [../sim3dview/README.md](../sim3dview/README.md) | ライブラリの公開APIと組み込み方 |
| [tech_note.html](tech_note.html) | 技術解説ノートのHTMLスナップショット。設計の正ではない |

## 設計書の読み方

| 目的 | 節 |
|---|---|
| 全体像と確定方針 | 0節 |
| 地形データ・前処理・座標系 | 1〜3節 |
| 通信・C++サーバー | 4〜5節 |
| Rustライブラリ・UI | 6〜7節 |
| 図の索引 | 8節 |
| 定数・アルゴリズム・バイト配置 | 9節 |
| 再実装の順序と受け入れ基準 | 10節 |

## 保守ルール

- 現行仕様は`DETAILED_DESIGN.md`だけに書き、履歴は`DEVELOPMENT_HISTORY.md`へ分ける。
- 数値・アルゴリズム・バイト配置を変えたら9節、設計理由を変えたら該当する1〜7節を更新する。
- 節番号はソースコメントから参照されているため、既存の1〜9節を振り直さない。
- シェーダー全文は持たず、エントリポイント、式、バイト契約だけを9.9節へ記録する。
- 依存を変えたら`python scripts/gen_third_party_notice.py`で`THIRD_PARTY_NOTICE.md`を再生成する。
