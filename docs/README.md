# ライブラリのドキュメント

`docs/`には`sim3dview`ライブラリと地形前処理CLIの文書だけを置く。サンプルアプリ(`sample/`)に固有の文書は[sample/docs/](../sample/docs/README.md)に置く。

- [設計書](DETAILED_DESIGN.md): 地形形式、状態、描画、実装仕様。設計の正。節番号は維持し、サンプルへ移した節は欠番として残す。
- [利用方法](../README.md)
- [APIリファレンス](../API_REFERENCE.md)
- [実装ガイドライン](../IMPLEMENTATION_GUIDELINES.md)
- [技術解説ノート(ライブラリ編)](tech_note.html): C++経験者向けの解説スナップショット。現行仕様の正ではない。

## 置き場所の決め方

| 内容 | 置き場所 |
|---|---|
| ライブラリの公開API・地形データ契約・描画・汎用UI部品 | `docs/` |
| 通信プロトコル・C++参照サーバー・業務UI(VAB・状況パネル・メニュー・ログ)・Electron | `sample/docs/` |
| 開発の経緯・過去のハマりどころ | `sample/docs/DEVELOPMENT_HISTORY.md` |

ライブラリの文書からサンプルを参照するのは、利用例へのリンクに留める。サンプル固有の仕様・手順は`docs/`へ書かない。
