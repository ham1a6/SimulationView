# サードパーティ・ソフトウェア等の表示(THIRD_PARTY_NOTICE)

Sim3dView(`sim3dview`ライブラリ・`sample/sim_frontend`・`sample/sim_server`・`tools/geotiff_preprocess`)が、利用・同梱・リンクしているサードパーティのソフトウェアとデータの一覧、その著作権表示とライセンス条件です。

> **この文書について**: 依存関係とライセンスの情報を機械的に集めた**参考資料**で、法的な助言ではありません。製品として配布する前に、最新の依存関係(`Cargo.lock`・`vcpkg.json`)で再生成し、必要に応じて法務の確認を受けてください。本プロジェクト自身のライセンスは、このファイルの対象外です(リポジトリにLICENSEファイルはまだありません)。

- 生成日: 2026-09-24(`Cargo.lock`の内容とvcpkg・サブモジュールの版に基づく)
- 再生成の手順は末尾の「この文書の更新方法」を参照

## 目次

1. [地形データ: ALOS World 3D-30m (AW3D30)](#1-地形データ-alos-world-3d-30m-aw3d30)
2. [ブラウザに配布されるもの(Rustクレート)](#2-ブラウザに配布されるものrustクレート)
3. [サーバー(`sim_server`)に含まれるもの(C++)](#3-サーバーsim_serverに含まれるものc)
4. [前処理ツール(`geotiff_preprocess`)が使うもの](#4-前処理ツールgeotiff_preprocessが使うもの)
5. [著作権表示(Rustクレートごと)](#5-著作権表示rustクレートごと)
6. [ライセンス全文](#6-ライセンス全文)
7. [この文書に含めないもの・更新方法](#7-この文書に含めないもの更新方法)

## 1. 地形データ: ALOS World 3D-30m (AW3D30)

`sample/map_data/`の標高データ(`ALPSMLC30_*`。リポジトリには含まれず、ローカルに置く入力データ)と、それを`geotiff_preprocess`で変換した`assets/terrain/`の地形タイル(`metadata.json`・`base.bin`・`tiles/`)は、JAXA(宇宙航空研究開発機構)の**ALOS World 3D-30m(AW3D30)** から作られた二次的なデータです。

| 項目 | 内容 |
|---|---|
| データ名 | ALOS World 3D - 30m (AW3D30)(ファイル名の接頭辞は`ALPSMLC30`) |
| 提供元 | JAXA / EORC(地球観測研究センター)。データの取得元は<https://www.eorc.jaxa.jp/ALOS/en/dataset/aw3d30/aw3d30_e.htm> |
| 利用条件 | JAXAの利用条件(<https://earth.jaxa.jp/en/data/policy/>)に従う |
| クレジット | **JAXAがデータの提供元であることを明示する**こと。この条件のもとでの例示は「Credit: XXXX (JAXA)」の形 |
| 商用利用 | 利用条件のページでは、商用利用の場合は**事前にJAXAへ通知が必要**とされている(AW3D30のデータ提供ページは「無償で、商用・非商用を問わず利用できる」と書いており、表現が一致していない) |
| 再配布・二次的データ | 利用条件に従って可能。二次的なデータを配布するときは、JAXAとその他の関与した組織の両方をクレジットする |
| 保証 | JAXAは、データの利用またはその品質による結果に責任を負わない |
| 問い合わせ | earth@ml.jaxa.jp(JAXAの利用条件ページに記載) |

**推奨するクレジット表記の例**(アプリの「ヘルプ」や画面の隅などに置く):

```text
Elevation data: ALOS World 3D - 30m (AW3D30), provided by the Japan Aerospace Exploration Agency (JAXA).
標高データ: ALOS World 3D - 30m (AW3D30)(提供: 宇宙航空研究開発機構 JAXA)。地形タイルは、このデータをSim3dViewの前処理ツールで変換したものです。
```

> **要確認**: 上の内容は2026年9月時点でJAXAのWebページに書かれていた条件の要約です。**商用で配布・提供する場合は、JAXAの現行の利用条件を直接確認し、必要ならJAXAへ事前に通知してください**(特に、事前通知の要否と、地形タイルを配布物・サービスに含めることの扱い)。

## 2. ブラウザに配布されるもの(Rustクレート)

`sim3dview`と`sample/sim_frontend`をWebAssemblyにビルドしたときに、実行時にリンクされるクレートです(`Cargo.lock`から`wasm32-unknown-unknown`向けに解決した**194個**。単体テスト専用の依存(`naga`)、ビルド時だけ動くもの(proc-macro・ビルドスクリプトの依存)、開発用ツール(`trunk`・`wasm-bindgen`のCLI)は含めない)。

ライセンスが「A OR B」の形のクレートは、AとBのどちらの条件でも利用できる二重ライセンスです。

**ライセンスの内訳**:

- 99個: `MIT OR Apache-2.0`
- 39個: `MIT`
- 15個: `Unicode-3.0`
- 13個: `Apache-2.0 OR MIT`
- 5個: `MIT/Apache-2.0`
- 5個: `Zlib`
- 4個: `Unlicense OR MIT`
- 2個: `Apache-2.0`
- 2個: `Apache-2.0/MIT`
- 2個: `MIT OR Apache-2.0 OR Zlib`
- 2個: `Unlicense/MIT`
- 1個: `(MIT OR Apache-2.0) AND Unicode-3.0`
- 1個: `BSD-2-Clause OR Apache-2.0 OR MIT`
- 1個: `BSL-1.0`
- 1個: `CC0-1.0`
- 1個: `ISC`
- 1個: `Zlib OR Apache-2.0 OR MIT`

主要なクレートの役割: `leptos`(UIフレームワーク)、`wgpu`(WebGPU描画)、`glam`(ベクトル・行列)、`earcutr`(多角形の三角形分割)、`serde`/`serde_json`/`rmp-serde`(シリアライズ)、`gloo-net`/`gloo-timers`/`web-sys`/`wasm-bindgen`/`js-sys`(ブラウザAPI)、`bytemuck`(GPUバッファへのコピー)。

| クレート | 版 | ライセンス | 配布元 |
|---|---|---|---|
| aho-corasick | 1.1.5 | Unlicense OR MIT | <https://github.com/BurntSushi/aho-corasick> |
| any_spawner | 0.3.0 | MIT | <https://github.com/leptos-rs/leptos> |
| anyhow | 1.0.104 | MIT OR Apache-2.0 | <https://github.com/dtolnay/anyhow> |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 | <https://github.com/bluss/arrayvec> |
| async-lock | 3.4.2 | Apache-2.0 OR MIT | <https://github.com/smol-rs/async-lock> |
| async-once-cell | 0.5.4 | MIT OR Apache-2.0 | <https://github.com/danieldg/async-once-cell> |
| attribute-derive | 0.10.5 | MIT OR Apache-2.0 | <https://github.com/ModProg/attribute-derive> |
| base16 | 0.2.1 | CC0-1.0 | <https://github.com/thomcc/rust-base16> |
| base64 | 0.22.1 | MIT OR Apache-2.0 | <https://github.com/marshallpierce/rust-base64> |
| bit-set | 0.10.0 | Apache-2.0 OR MIT | <https://github.com/contain-rs/bit-set> |
| bit-vec | 0.9.1 | Apache-2.0 OR MIT | <https://github.com/contain-rs/bit-vec> |
| bitflags | 2.13.2 | MIT OR Apache-2.0 | <https://github.com/bitflags/bitflags> |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 | <https://github.com/RustCrypto/utils> |
| bumpalo | 3.20.3 | MIT OR Apache-2.0 | <https://github.com/fitzgen/bumpalo> |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT | <https://github.com/Lokathor/bytemuck> |
| byteorder | 1.5.0 | Unlicense OR MIT | <https://github.com/BurntSushi/byteorder> |
| bytes | 1.12.1 | MIT | <https://github.com/tokio-rs/bytes> |
| camino | 1.2.6 | MIT OR Apache-2.0 | <https://github.com/camino-rs/camino> |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 | <https://github.com/rust-lang/cfg-if> |
| codee | 0.3.5 | MIT OR Apache-2.0 | <https://github.com/Synphonyte/codee> |
| codespan-reporting | 0.13.1 | Apache-2.0 | <https://github.com/brendanzab/codespan> |
| collection_literals | 1.0.3 | MIT | <https://github.com/staedoix/collection_literals> |
| config | 0.15.25 | MIT OR Apache-2.0 | <https://github.com/rust-cli/config-rs> |
| console_error_panic_hook | 0.1.7 | Apache-2.0/MIT | <https://github.com/rustwasm/console_error_panic_hook> |
| console_log | 1.1.0 | MIT/Apache-2.0 | <https://github.com/iamcodemaker/console_log> |
| const-str | 1.1.0 | MIT | <https://github.com/Nugine/const-str> |
| const_format | 0.2.36 | Zlib | <https://github.com/rodrimati1992/const_format_crates/> |
| const_str_slice_concat | 0.1.0 | MIT | <https://github.com/leptos-rs/leptos> |
| convert_case | 0.11.0 | MIT | <https://github.com/rutrum/convert-case> |
| convert_case | 0.6.0 | MIT | <https://github.com/rutrum/convert-case> |
| convert_case_extras | 0.2.0 | MIT | <https://github.com/rutrum/convert-case-extras> |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 | <https://github.com/RustCrypto/traits> |
| digest | 0.10.7 | MIT OR Apache-2.0 | <https://github.com/RustCrypto/traits> |
| drain_filter_polyfill | 0.1.3 | MIT OR Apache-2.0 | <https://github.com/whentze/drain_filter_polyfill.git> |
| earcutr | 0.5.0 | ISC | <https://github.com/frewsxcv/earcutr/> |
| either | 1.18.0 | MIT OR Apache-2.0 | <https://github.com/rayon-rs/either> |
| either_of | 0.1.9 | MIT | <https://github.com/leptos-rs/leptos> |
| equivalent | 1.0.2 | Apache-2.0 OR MIT | <https://github.com/indexmap-rs/equivalent> |
| erased | 0.1.2 | MIT | <https://github.com/JonathanBrouwer/erased> |
| event-listener | 5.4.2 | Apache-2.0 OR MIT | <https://github.com/smol-rs/event-listener> |
| event-listener-strategy | 0.5.4 | Apache-2.0 OR MIT | <https://github.com/smol-rs/event-listener-strategy> |
| foldhash | 0.2.0 | Zlib | <https://github.com/orlp/foldhash> |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 | <https://github.com/servo/rust-url> |
| futures | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-channel | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-core | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-executor | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-io | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-sink | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-task | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| futures-util | 0.3.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/futures-rs> |
| generic-array | 0.14.7 | MIT | <https://github.com/fizyk20/generic-array.git> |
| geographiclib-rs | 0.2.7 | MIT | <https://github.com/georust/geographiclib-rs> |
| getrandom | 0.4.3 | MIT OR Apache-2.0 | <https://github.com/rust-random/getrandom> |
| glam | 0.33.7 | MIT OR Apache-2.0 | <https://github.com/bitshifter/glam-rs> |
| gloo-net | 0.6.0 | MIT OR Apache-2.0 | <https://github.com/rustwasm/gloo> |
| gloo-net | 0.7.0 | MIT OR Apache-2.0 | <https://github.com/rustwasm/gloo> |
| gloo-timers | 0.4.0 | MIT OR Apache-2.0 | <https://github.com/rustwasm/gloo/tree/master/crates/timers> |
| gloo-utils | 0.2.0 | MIT OR Apache-2.0 | <https://github.com/rustwasm/gloo/tree/master/crates/utils> |
| gloo-utils | 0.3.0 | MIT OR Apache-2.0 | <https://github.com/rustwasm/gloo/tree/master/crates/utils> |
| glow | 0.17.0 | MIT OR Apache-2.0 OR Zlib | <https://github.com/grovesNL/glow> |
| gltf | 1.4.1 | MIT OR Apache-2.0 | <https://github.com/gltf-rs/gltf> |
| gltf-json | 1.4.1 | MIT OR Apache-2.0 | <https://github.com/gltf-rs/gltf> |
| guardian | 1.3.0 | MIT OR Apache-2.0 | <https://github.com/jonhoo/guardian.git> |
| half | 2.7.1 | MIT OR Apache-2.0 | <https://github.com/VoidStarKat/half-rs> |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | <https://github.com/rust-lang/hashbrown> |
| html-escape | 0.2.15 | MIT | <https://github.com/magiclen/html-escape> |
| http | 1.5.0 | MIT OR Apache-2.0 | <https://github.com/hyperium/http> |
| hydration_context | 0.3.1 | MIT | <https://github.com/leptos-rs/leptos> |
| icu_collections | 2.3.0 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| icu_locale_core | 2.3.0 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| icu_normalizer | 2.3.0 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| icu_normalizer_data | 2.3.0 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| icu_properties | 2.3.0 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| icu_properties_data | 2.3.0 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| icu_provider | 2.3.1 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| idna | 1.1.0 | MIT OR Apache-2.0 | <https://github.com/servo/rust-url/> |
| idna_adapter | 1.2.2 | Apache-2.0 OR MIT | <https://github.com/hsivonen/idna_adapter> |
| indexmap | 2.14.2 | Apache-2.0 OR MIT | <https://github.com/indexmap-rs/indexmap> |
| inflections | 1.1.1 | MIT | <https://docs.rs/inflections> |
| interpolator | 0.5.0 | MIT OR Apache-2.0 | <https://github.com/ModProg/interpolator> |
| itertools | 0.14.0 | MIT OR Apache-2.0 | <https://github.com/rust-itertools/itertools> |
| itoa | 1.0.18 | MIT OR Apache-2.0 | <https://github.com/dtolnay/itoa> |
| js-sys | 0.3.105 | MIT OR Apache-2.0 | <https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys> |
| konst | 0.2.20 | Zlib | <https://github.com/rodrimati1992/konst/> |
| konst_macro_rules | 0.2.19 | Zlib | <https://github.com/rodrimati1992/konst/> |
| lazy_static | 1.5.0 | MIT OR Apache-2.0 | <https://github.com/rust-lang-nursery/lazy-static.rs> |
| leptos | 0.8.20 | MIT | <https://github.com/leptos-rs/leptos> |
| leptos_config | 0.8.10 | MIT | <https://github.com/leptos-rs/leptos> |
| leptos_dom | 0.8.8 | MIT | <https://github.com/leptos-rs/leptos> |
| leptos_hot_reload | 0.8.6 | MIT | <https://github.com/leptos-rs/leptos> |
| leptos_server | 0.8.7 | MIT | <https://github.com/leptos-rs/leptos> |
| libm | 0.2.16 | MIT | <https://github.com/rust-lang/compiler-builtins> |
| litemap | 0.8.3 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| litrs | 1.0.0 | MIT OR Apache-2.0 | <https://github.com/LukasKalbertodt/litrs> |
| lock_api | 0.4.14 | MIT OR Apache-2.0 | <https://github.com/Amanieu/parking_lot> |
| log | 0.4.34 | MIT OR Apache-2.0 | <https://github.com/rust-lang/log> |
| manyhow | 0.11.4 | MIT OR Apache-2.0 | <https://github.com/ModProg/manyhow> |
| memchr | 2.8.3 | Unlicense OR MIT | <https://github.com/BurntSushi/memchr> |
| naga | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| naga-types | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| next_tuple | 0.1.0 | MIT | <https://github.com/leptos-rs/leptos> |
| num-traits | 0.2.19 | MIT OR Apache-2.0 | <https://github.com/rust-num/num-traits> |
| oco_ref | 0.2.1 | MIT | <https://github.com/leptos-rs/leptos> |
| once_cell | 1.21.4 | MIT OR Apache-2.0 | <https://github.com/matklad/once_cell> |
| or_poisoned | 0.1.0 | MIT | <https://github.com/leptos-rs/leptos> |
| ordered-float | 5.5.0 | MIT | <https://github.com/reem/rust-ordered-float> |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 | <https://github.com/Amanieu/parking_lot> |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 | <https://github.com/Amanieu/parking_lot> |
| pathdiff | 0.2.3 | MIT/Apache-2.0 | <https://github.com/Manishearth/pathdiff> |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 | <https://github.com/servo/rust-url/> |
| pin-project | 1.1.13 | Apache-2.0 OR MIT | <https://github.com/taiki-e/pin-project> |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT | <https://github.com/taiki-e/pin-project-lite> |
| potential_utf | 0.1.6 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| prettyplease | 0.2.37 | MIT OR Apache-2.0 | <https://github.com/dtolnay/prettyplease> |
| proc-macro-error2 | 2.0.1 | MIT OR Apache-2.0 | <https://github.com/GnomedDev/proc-macro-error-2> |
| proc-macro-utils | 0.10.0 | MIT OR Apache-2.0 | <https://github.com/ModProg/proc-macro-utils> |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | <https://github.com/dtolnay/proc-macro2> |
| proc-macro2-diagnostics | 0.10.1 | MIT/Apache-2.0 | <https://github.com/SergioBenitez/proc-macro2-diagnostics> |
| profiling | 1.0.18 | MIT OR Apache-2.0 | <https://github.com/aclysma/profiling> |
| quote | 1.0.47 | MIT OR Apache-2.0 | <https://github.com/dtolnay/quote> |
| quote-use | 0.8.4 | MIT | <https://github.com/ModProg/quote-use> |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib | <https://github.com/rust-windowing/raw-window-handle> |
| reactive_graph | 0.2.14 | MIT | <https://github.com/leptos-rs/leptos> |
| reactive_stores | 0.4.3 | MIT | <https://github.com/leptos-rs/leptos> |
| regex | 1.13.1 | MIT OR Apache-2.0 | <https://github.com/rust-lang/regex> |
| regex-automata | 0.4.18 | MIT OR Apache-2.0 | <https://github.com/rust-lang/regex> |
| regex-syntax | 0.8.11 | MIT OR Apache-2.0 | <https://github.com/rust-lang/regex> |
| rmp | 0.8.15 | MIT | <https://github.com/3Hren/msgpack-rust> |
| rmp-serde | 1.3.1 | MIT | <https://github.com/3Hren/msgpack-rust> |
| rstml | 0.12.1 | MIT | <https://github.com/rs-tml/rstml> |
| rustc-hash | 1.1.0 | Apache-2.0/MIT | <https://github.com/rust-lang-nursery/rustc-hash> |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT | <https://github.com/rust-lang/rustc-hash> |
| same-file | 1.0.6 | Unlicense/MIT | <https://github.com/BurntSushi/same-file> |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 | <https://github.com/bluss/scopeguard> |
| send_wrapper | 0.6.0 | MIT/Apache-2.0 | <https://github.com/thk1/send_wrapper> |
| serde | 1.0.229 | MIT OR Apache-2.0 | <https://github.com/serde-rs/serde> |
| serde_core | 1.0.229 | MIT OR Apache-2.0 | <https://github.com/serde-rs/serde> |
| serde_json | 1.0.151 | MIT OR Apache-2.0 | <https://github.com/serde-rs/json> |
| serde_qs | 0.15.0 | MIT/Apache-2.0 | <https://github.com/samscott89/serde_qs> |
| serde_spanned | 1.1.1 | MIT OR Apache-2.0 | <https://github.com/toml-rs/toml> |
| server_fn | 0.8.13 | MIT | <https://github.com/leptos-rs/leptos> |
| server_fn_macro | 0.8.10 | MIT | <https://github.com/leptos-rs/leptos> |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | <https://github.com/RustCrypto/hashes> |
| slab | 0.4.12 | MIT | <https://github.com/tokio-rs/slab> |
| slotmap | 1.1.1 | Zlib | <https://github.com/orlp/slotmap> |
| smallvec | 1.16.1 | MIT OR Apache-2.0 | <https://github.com/servo/rust-smallvec> |
| spirv | 0.4.0+sdk-1.4.341.0 | Apache-2.0 | <https://github.com/gfx-rs/rspirv> |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 | <https://github.com/storyyeller/stable_deref_trait> |
| static_assertions | 1.1.0 | MIT OR Apache-2.0 | <https://github.com/nvzqz/static-assertions-rs> |
| syn | 2.0.119 | MIT OR Apache-2.0 | <https://github.com/dtolnay/syn> |
| syn | 3.0.6 | MIT OR Apache-2.0 | <https://github.com/dtolnay/syn> |
| synstructure | 0.14.0 | MIT | <https://github.com/mystor/synstructure> |
| tachys | 0.2.18 | MIT | <https://github.com/leptos-rs/leptos> |
| termcolor | 1.4.1 | Unlicense OR MIT | <https://github.com/BurntSushi/termcolor> |
| thiserror | 1.0.69 | MIT OR Apache-2.0 | <https://github.com/dtolnay/thiserror> |
| thiserror | 2.0.20 | MIT OR Apache-2.0 | <https://github.com/dtolnay/thiserror> |
| throw_error | 0.3.1 | MIT | <https://github.com/leptos-rs/leptos> |
| tinystr | 0.8.4 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| toml | 1.1.6+spec-1.1.0 | MIT OR Apache-2.0 | <https://github.com/toml-rs/toml> |
| toml_datetime | 1.1.1+spec-1.1.0 | MIT OR Apache-2.0 | <https://github.com/toml-rs/toml> |
| toml_parser | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 | <https://github.com/toml-rs/toml> |
| typed-builder | 0.23.2 | MIT OR Apache-2.0 | <https://github.com/idanarye/rust-typed-builder> |
| typenum | 1.20.1 | MIT OR Apache-2.0 | <https://github.com/paholg/typenum> |
| unicode-ident | 1.0.26 | (MIT OR Apache-2.0) AND Unicode-3.0 | <https://github.com/dtolnay/unicode-ident> |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 | <https://github.com/unicode-rs/unicode-segmentation> |
| unicode-width | 0.2.2 | MIT OR Apache-2.0 | <https://github.com/unicode-rs/unicode-width> |
| unicode-xid | 0.2.6 | MIT OR Apache-2.0 | <https://github.com/unicode-rs/unicode-xid> |
| url | 2.5.8 | MIT OR Apache-2.0 | <https://github.com/servo/rust-url> |
| utf8_iter | 1.0.4 | Apache-2.0 OR MIT | <https://github.com/hsivonen/utf8_iter> |
| uuid | 1.26.1 | Apache-2.0 OR MIT | <https://github.com/uuid-rs/uuid> |
| walkdir | 2.5.0 | Unlicense/MIT | <https://github.com/BurntSushi/walkdir> |
| wasm-bindgen | 0.2.128 | MIT OR Apache-2.0 | <https://github.com/wasm-bindgen/wasm-bindgen> |
| wasm-bindgen-futures | 0.4.78 | MIT OR Apache-2.0 | <https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/futures> |
| wasm-bindgen-macro-support | 0.2.128 | MIT OR Apache-2.0 | <https://github.com/wasm-bindgen/wasm-bindgen/tree/main/crates/macro-support> |
| wasm-bindgen-shared | 0.2.128 | MIT OR Apache-2.0 | <https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared> |
| wasm-streams | 0.5.0 | MIT OR Apache-2.0 | <https://github.com/MattiasBuelens/wasm-streams/> |
| wasm_split_helpers | 0.2.3 | MIT OR Apache-2.0 | <https://github.com/WorldSEnder/wasm-split-prototype> |
| web-sys | 0.3.105 | MIT OR Apache-2.0 | <https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/web-sys> |
| wgpu | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| wgpu-core | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| wgpu-hal | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| wgpu-naga-bridge | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| wgpu-types | 30.0.1 | MIT OR Apache-2.0 | <https://github.com/gfx-rs/wgpu> |
| winnow | 1.0.4 | MIT | <https://github.com/winnow-rs/winnow> |
| writeable | 0.6.4 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| xxhash-rust | 0.8.18 | BSL-1.0 | <https://github.com/DoumanAsh/xxhash-rust> |
| yansi | 1.0.1 | MIT OR Apache-2.0 | <https://github.com/SergioBenitez/yansi> |
| yoke | 0.8.3 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| zerocopy | 0.8.57 | BSD-2-Clause OR Apache-2.0 OR MIT | <https://github.com/google/zerocopy> |
| zerofrom | 0.1.8 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| zerotrie | 0.2.5 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| zerovec | 0.11.8 | Unicode-3.0 | <https://github.com/unicode-org/icu4x> |
| zmij | 1.0.23 | MIT | <https://github.com/dtolnay/zmij> |

WebAssemblyにはRustの標準ライブラリ(`std`・`core`・`alloc`、`compiler_builtins`など。`MIT OR Apache-2.0`)のコードも含まれます。配布元: <https://github.com/rust-lang/rust>

## 3. サーバー(`sim_server`)に含まれるもの(C++)

`sample/sim_server`をビルドした`sim_server.exe`に、ソースの取り込み(サブモジュール)またはvcpkgのライブラリとしてリンクされます。

| ライブラリ | 版 | ライセンス | 配布元・ライセンスファイル |
|---|---|---|---|
| uWebSockets | v20.80.0系(サブモジュール) | Apache-2.0 | <https://github.com/uNetworking/uWebSockets> (`sample/sim_server/third_party/uWebSockets/LICENSE`) |
| uSockets | v0.8.8系(uWebSocketsに同梱) | Apache-2.0 | <https://github.com/uNetworking/uSockets> (`.../uWebSockets/uSockets/LICENSE`) |
| msgpack-c(C++版、msgpack-cxx) | cpp-9.0.0(サブモジュール) | BSL-1.0(Boost Software License 1.0)。著作権表示: Copyright (C) 2008-2015 FURUHASHI Sadayuki | <https://github.com/msgpack/msgpack-c> (`.../msgpack-cxx/LICENSE_1_0.txt`・`COPYING`・`NOTICE`) |
| libuv | 1.52.1(vcpkg) | MIT | <https://github.com/libuv/libuv> |
| OpenSSL | 3.6.4(vcpkg) | Apache-2.0 | <https://www.openssl.org/> (TLS用。ビルドには常に必要) |
| zlib | 1.3.2(vcpkg) | Zlib | <https://zlib.net/> |

- msgpack-cxxは、Boost PredefとBoost Preprocessor(いずれもBoost Software License 1.0)を同梱しています(`msgpack-cxx/NOTICE`)。
- uSocketsのソースには、BoringSSL・lsquicのディレクトリがありますが、`sample/sim_server/CMakeLists.txt`はこれらをビルドに使いません(TLSはvcpkgのOpenSSL)。
- サーバー本体(`sample/sim_server/src`・`include`)はこのプロジェクトのコードです。
- 配布物にサブモジュールのソースを含める場合は、各サブモジュールの`LICENSE`ファイルを一緒に配布してください。

### 3.1 msgpack-cxxのライセンス全文(Boost Software License 1.0)

```text
Boost Software License - Version 1.0 - August 17th, 2003

Permission is hereby granted, free of charge, to any person or organization
obtaining a copy of the software and accompanying documentation covered by
this license (the "Software") to use, reproduce, display, distribute,
execute, and transmit the Software, and to prepare derivative works of the
Software, and to permit third-parties to whom the Software is furnished to
do so, all subject to the following:

The copyright notices in the Software and this entire statement, including
the above license grant, this restriction and the following disclaimer,
must be included in all copies of the Software, in whole or in part, and
all derivative works of the Software, unless such copies or derivative
works are solely in the form of machine-executable object code generated by
a source language processor.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE, TITLE AND NON-INFRINGEMENT. IN NO EVENT
SHALL THE COPYRIGHT HOLDERS OR ANYONE DISTRIBUTING THE SOFTWARE BE LIABLE
FOR ANY DAMAGES OR OTHER LIABILITY, WHETHER IN CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

uWebSockets・uSockets・OpenSSLのApache-2.0、libuvのMIT、zlibのZlibライセンスの全文は、[6. ライセンス全文](#6-ライセンス全文)にあります(libuvは`Copyright (c) 2015-present libuv project contributors.`、zlibは`(C) 1995-2026 Jean-loup Gailly and Mark Adler`)。uWebSockets・uSocketsのソースには、著作権者名の記載も`NOTICE`ファイルもありません(`LICENSE`はApache-2.0の全文のみ)。配布するときは、上流のリポジトリの表示を確認してください。

## 4. 前処理ツール(`geotiff_preprocess`)が使うもの

`tools/geotiff_preprocess`(GeoTIFF→地形タイルの変換CLI)がvcpkg経由でリンクするライブラリです。このツールは開発・データ生成用で、ブラウザやサーバーには含まれませんが、**ツールのバイナリを配布する場合は**下記の表示が必要です。

| ライブラリ | 版 | ライセンス(vcpkgの`copyright`による) |
|---|---|---|
| GDAL | 3.12.4 | MIT(X11)。GDAL/OGRのソースツリー内のその他のライセンスは、GDALの`LICENSE.TXT`を参照 |
| PROJ | 9.8.1 | MIT |
| libtiff | 4.7.2 | libtiffライセンス(BSD風) |
| libgeotiff | 1.7.4 | MITまたはパブリックドメイン |
| curl | 8.22.0 | curlライセンス(MIT風) |
| SQLite | 3.53.4 | パブリックドメイン |
| json-c | 0.19-20260627 | MIT |
| libjpeg-turbo | 3.2.0 | BSD-3-Clause・IJGライセンス・Zlib |
| liblzma(XZ Utils) | 5.8.4 | 0BSD(ライブラリ本体) |
| nlohmann-json | 3.12.0 | MIT |
| zlib | 1.3.2 | Zlib |

GDALの推移的な依存はvcpkgの版によって変わります。配布するときは、そのビルドの`vcpkg_installed/<triplet>/share/<port>/copyright`を、この表と照合してください。

## 5. 著作権表示(Rustクレートごと)

各クレートのライセンスファイル(`LICENSE*`・`COPYING*`・`NOTICE*`)から抜き出した著作権表示です。ファイルに著作権表示が無いクレートは、`Cargo.toml`の`authors`を記載しています。

### aho-corasick 1.1.5
`Unlicense OR MIT`

- Copyright (c) 2015 Andrew Gallant
- In jurisdictions that recognize copyright laws, the author or authors

### any_spawner 0.3.0
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### anyhow 1.0.104
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) David Tolnay <dtolnay@gmail.com>

### arrayvec 0.7.8
`MIT OR Apache-2.0`

- Copyright (c) Ulrik Sverdrup "bluss" 2015-2023

### async-lock 3.4.2
`Apache-2.0 OR MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Stjepan Glavina <stjepang@gmail.com>

### async-once-cell 0.5.4
`MIT OR Apache-2.0`

- Copyright 2023 Daniel De Graaf
- Copyright (c) 2023 Daniel De Graaf

### attribute-derive 0.10.5
`MIT OR Apache-2.0`

- Copyright (c) 2024 Roland Fredenhagen

### base16 0.2.1
`CC0-1.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Thom Chiovoloni <tchiovoloni@mozilla.com>

### base64 0.22.1
`MIT OR Apache-2.0`

- Copyright (c) 2015 Alice Maz

### bit-set 0.10.0
`Apache-2.0 OR MIT`

- Copyright (c) 2026 The Rust Project Developers

### bit-vec 0.9.1
`Apache-2.0 OR MIT`

- Copyright (c) 2023 The Rust Project Developers

### bitflags 2.13.2
`MIT OR Apache-2.0`

- Copyright (c) 2014 The Rust Project Developers

### block-buffer 0.10.4
`MIT OR Apache-2.0`

- Copyright (c) 2018-2019 The RustCrypto Project Developers

### bumpalo 3.20.3
`MIT OR Apache-2.0`

- Copyright (c) 2019 Nick Fitzgerald

### bytemuck 1.25.2
`Zlib OR Apache-2.0 OR MIT`

- Copyright (c) 2019 Daniel "Lokathor" Gee.

### byteorder 1.5.0
`Unlicense OR MIT`

- Copyright (c) 2015 Andrew Gallant
- In jurisdictions that recognize copyright laws, the author or authors

### bytes 1.12.1
`MIT`

- Copyright (c) 2018 Carl Lerche

### camino 1.2.6
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Without Boats <saoirse@without.boats>, Ashley Williams <ashley666ashley@gmail.com>, Steve Klabnik <steve@steveklabnik.com>, Rain <rain@sunshowers.io>

### cfg-if 1.0.5
`MIT OR Apache-2.0`

- Copyright (c) 2014 Alex Crichton

### codee 0.3.5
`MIT OR Apache-2.0`

- Copyright (c) 2023 Synphonyte

### codespan-reporting 0.13.1
`Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Brendan Zabarauskas <bjzaba@yahoo.com.au>

### collection_literals 1.0.3
`MIT`

- Copyright (c) The collection_literals Contributors

### config 0.15.25
`MIT OR Apache-2.0`

- Copyright (c) Individual contributors

### console_error_panic_hook 0.1.7
`Apache-2.0/MIT`

- Copyright (c) 2018 Nick Fitzgerald

### console_log 1.1.0
`MIT/Apache-2.0`

- Copyright (c) 2018 Matthew Nicholson

### const-str 1.1.0
`MIT`

- Copyright (c) 2020-2026 Nugine

### const_format 0.2.36
`Zlib`

- Copyright (c) 2020 Matias Rodriguez.

### const_str_slice_concat 0.1.0
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### convert_case 0.11.0
`MIT`

- Copyright (c) 2025 rutrum

### convert_case 0.6.0
`MIT`

- Copyright (c) 2020 David Purdum

### convert_case_extras 0.2.0
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) rutrum <dave@rutrum.net>

### crypto-common 0.1.7
`MIT OR Apache-2.0`

- Copyright (c) 2021 RustCrypto Developers

### digest 0.10.7
`MIT OR Apache-2.0`

- Copyright (c) 2017 Artyom Pavlov

### drain_filter_polyfill 0.1.3
`MIT OR Apache-2.0`

- (著作権表示の記載なし。配布元を参照)

### earcutr 0.5.0
`ISC`

- Copyright (c) 2016, Mapbox
- Copyright (c) 2018, Tree Cricket

### either 1.18.0
`MIT OR Apache-2.0`

- Copyright (c) 2015

### either_of 0.1.9
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### equivalent 1.0.2
`Apache-2.0 OR MIT`

- Copyright (c) 2016--2023

### erased 0.1.2
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Jonathan Brouwer <jonathantbrouwer@gmail.com>, Jonathan Dönszelmann <jonabent@gmail.com>

### event-listener 5.4.2
`Apache-2.0 OR MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Stjepan Glavina <stjepang@gmail.com>, John Nunley <dev@notgull.net>

### event-listener-strategy 0.5.4
`Apache-2.0 OR MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) John Nunley <dev@notgull.net>

### foldhash 0.2.0
`Zlib`

- Copyright (c) 2024 Orson Peters

### form_urlencoded 1.2.2
`MIT OR Apache-2.0`

- Copyright (c) 2013-2016 The rust-url developers

### futures 0.3.34
`MIT OR Apache-2.0`

- Copyright (c) 2016 Alex Crichton
- Copyright (c) 2017 The Tokio Authors

### futures-channel 0.3.34
`MIT OR Apache-2.0`

- Copyright (c) 2016 Alex Crichton
- Copyright (c) 2017 The Tokio Authors

### futures-core 0.3.34
`MIT OR Apache-2.0`

- Copyright (c) 2016 Alex Crichton
- Copyright (c) 2017 The Tokio Authors

### futures-executor 0.3.34
`MIT OR Apache-2.0`

- Copyright (c) 2016 Alex Crichton
- Copyright (c) 2017 The Tokio Authors

### futures-io 0.3.34
`MIT OR Apache-2.0`

- Copyright (c) 2016 Alex Crichton
- Copyright (c) 2017 The Tokio Authors

### futures-sink 0.3.34
`MIT OR Apache-2.0`

- Copyright (c) 2016 Alex Crichton
- Copyright (c) 2017 The Tokio Authors

### futures-task 0.3.34
`MIT OR Apache-2.0`

- Copyright (c) 2016 Alex Crichton
- Copyright (c) 2017 The Tokio Authors

### futures-util 0.3.34
`MIT OR Apache-2.0`

- Copyright (c) 2016 Alex Crichton
- Copyright (c) 2017 The Tokio Authors

### generic-array 0.14.7
`MIT`

- Copyright (c) 2015 Bartłomiej Kamiński

### geographiclib-rs 0.2.7
`MIT`

- Copyright (c) 2019

### getrandom 0.4.3
`MIT OR Apache-2.0`

- Copyright (c) 2018-2026 The rust-random Project Developers
- Copyright (c) 2014 The Rust Project Developers

### glam 0.33.7
`MIT OR Apache-2.0`

- Copyright 2020 Cameron Hart

### gloo-net 0.6.0
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Rust and WebAssembly Working Group, Elina <imelina@elina.website>

### gloo-net 0.7.0
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Rust and WebAssembly Working Group, Elina <imelina@elina.website>

### gloo-timers 0.4.0
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Rust and WebAssembly Working Group

### gloo-utils 0.2.0
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Rust and WebAssembly Working Group

### gloo-utils 0.3.0
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Rust and WebAssembly Working Group

### glow 0.17.0
`MIT OR Apache-2.0 OR Zlib`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Joshua Groves <josh@joshgroves.com>, Dzmitry Malyshau <kvarkus@gmail.com>

### gltf 1.4.1
`MIT OR Apache-2.0`

- Copyright (c) 2017 The gltf Library Developers

### gltf-json 1.4.1
`MIT OR Apache-2.0`

- Copyright (c) 2017 The gltf Library Developers

### guardian 1.3.0
`MIT OR Apache-2.0`

- Copyright (c) 2016 Jon Gjengset

### half 2.7.1
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Kathryn Long <squeeself@gmail.com>

### hashbrown 0.17.1
`MIT OR Apache-2.0`

- Copyright (c) 2016 Amanieu d'Antras

### html-escape 0.2.15
`MIT`

- Copyright (c) 2020 magiclen.org (Ron Li)

### http 1.5.0
`MIT OR Apache-2.0`

- Copyright 2017 http-rs authors
- Copyright (c) 2017 http-rs authors

### hydration_context 0.3.1
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### icu_collections 2.3.0
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### icu_locale_core 2.3.0
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### icu_normalizer 2.3.0
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### icu_normalizer_data 2.3.0
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### icu_properties 2.3.0
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### icu_properties_data 2.3.0
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### icu_provider 2.3.1
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### idna 1.1.0
`MIT OR Apache-2.0`

- Copyright (c) 2013-2025 The rust-url developers

### idna_adapter 1.2.2
`Apache-2.0 OR MIT`

- Copyright (c) The rust-url developers

### indexmap 2.14.2
`Apache-2.0 OR MIT`

- Copyright (c) 2016--2017

### inflections 1.1.1
`MIT`

- Copyright (c) 2016 Caleb Meredith

### interpolator 0.5.0
`MIT OR Apache-2.0`

- Copyright (c) 2023 Roland Fredenhagen

### itertools 0.14.0
`MIT OR Apache-2.0`

- Copyright (c) 2015

### itoa 1.0.18
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) David Tolnay <dtolnay@gmail.com>

### js-sys 0.3.105
`MIT OR Apache-2.0`

- Copyright (c) 2014 Alex Crichton

### konst 0.2.20
`Zlib`

- Copyright (c) 2021 Matias Rodriguez.

### konst_macro_rules 0.2.19
`Zlib`

- Copyright (c) 2021 Matias Rodriguez.

### lazy_static 1.5.0
`MIT OR Apache-2.0`

- Copyright (c) 2010 The Rust Project Developers

### leptos 0.8.20
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### leptos_config 0.8.10
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### leptos_dom 0.8.8
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### leptos_hot_reload 0.8.6
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### leptos_server 0.8.7
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### libm 0.2.16
`MIT`

- Copyright (c) 2018 Jorge Aparicio
- Copyright © 2005-2020 Rich Felker, et al.
- Copyright © 1993,2004 Sun Microsystems or
- Copyright © 2003-2011 David Schultz or
- Copyright © 2003-2009 Steven G. Kargl or
- Copyright © 2003-2009 Bruce D. Evans or
- Copyright © 2008 Stephen L. Moshier or
- Copyright © 2017-2018 Arm Limited

### litemap 0.8.3
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### litrs 1.0.0
`MIT OR Apache-2.0`

- Copyright (c) 2020 Project Developers

### lock_api 0.4.14
`MIT OR Apache-2.0`

- Copyright (c) 2016 The Rust Project Developers

### log 0.4.34
`MIT OR Apache-2.0`

- Copyright (c) 2014 The Rust Project Developers

### manyhow 0.11.4
`MIT OR Apache-2.0`

- Copyright (c) 2023 Roland Fredenhagen

### memchr 2.8.3
`Unlicense OR MIT`

- Copyright (c) 2015 Andrew Gallant
- In jurisdictions that recognize copyright laws, the author or authors

### naga 30.0.1
`MIT OR Apache-2.0`

- Copyright (c) 2025 The gfx-rs developers

### naga-types 30.0.1
`MIT OR Apache-2.0`

- Copyright (c) 2025 The gfx-rs developers

### next_tuple 0.1.0
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### num-traits 0.2.19
`MIT OR Apache-2.0`

- Copyright (c) 2014 The Rust Project Developers

### oco_ref 0.2.1
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Danik Vitek, Greg Johnston

### once_cell 1.21.4
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Aleksey Kladov <aleksey.kladov@gmail.com>

### or_poisoned 0.1.0
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### ordered-float 5.5.0
`MIT`

- Copyright (c) 2015 Jonathan Reem

### parking_lot 0.12.5
`MIT OR Apache-2.0`

- Copyright (c) 2016 The Rust Project Developers

### parking_lot_core 0.9.12
`MIT OR Apache-2.0`

- Copyright (c) 2016 The Rust Project Developers

### pathdiff 0.2.3
`MIT/Apache-2.0`

- Copyright 2017 The Rust Project Developers

### percent-encoding 2.3.2
`MIT OR Apache-2.0`

- Copyright (c) 2013-2025 The rust-url developers

### pin-project 1.1.13
`Apache-2.0 OR MIT`

- (著作権表示の記載なし。配布元を参照)

### pin-project-lite 0.2.17
`Apache-2.0 OR MIT`

- (著作権表示の記載なし。配布元を参照)

### potential_utf 0.1.6
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### prettyplease 0.2.37
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) David Tolnay <dtolnay@gmail.com>

### proc-macro-error2 2.0.1
`MIT OR Apache-2.0`

- Copyright 2019-2020 CreepySkeleton <creepy-skeleton@yandex.ru>
- Copyright (c) 2019-2020 CreepySkeleton

### proc-macro-utils 0.10.0
`MIT OR Apache-2.0`

- Copyright (c) 2023 Roland Fredenhagen

### proc-macro2 1.0.107
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) David Tolnay <dtolnay@gmail.com>, Alex Crichton <alex@alexcrichton.com>

### proc-macro2-diagnostics 0.10.1
`MIT/Apache-2.0`

- Copyright (c) 2016-2020 Sergio Benitez

### profiling 1.0.18
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Philip Degarmo <aclysma@gmail.com>

### quote 1.0.47
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) David Tolnay <dtolnay@gmail.com>

### quote-use 0.8.4
`MIT`

- (著作権表示の記載なし。配布元を参照)

### raw-window-handle 0.6.2
`MIT OR Apache-2.0 OR Zlib`

- Copyright (c) 2019 Osspial
- Copyright (c) 2020 Osspial

### reactive_graph 0.2.14
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### reactive_stores 0.4.3
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### regex 1.13.1
`MIT OR Apache-2.0`

- Copyright (c) 2014 The Rust Project Developers

### regex-automata 0.4.18
`MIT OR Apache-2.0`

- Copyright (c) 2014 The Rust Project Developers

### regex-syntax 0.8.11
`MIT OR Apache-2.0`

- Copyright (c) 2014 The Rust Project Developers

### rmp 0.8.15
`MIT`

- Copyright (c) 2017 Evgeny Safronov

### rmp-serde 1.3.1
`MIT`

- Copyright (c) 2017 Evgeny Safronov

### rstml 0.12.1
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) vldm <me@vldm.cc>, stoically <stoically@protonmail.com>

### rustc-hash 1.1.0
`Apache-2.0/MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) The Rust Project Developers

### rustc-hash 2.1.3
`Apache-2.0 OR MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) The Rust Project Developers

### same-file 1.0.6
`Unlicense/MIT`

- Copyright (c) 2017 Andrew Gallant
- In jurisdictions that recognize copyright laws, the author or authors

### scopeguard 1.2.0
`MIT OR Apache-2.0`

- Copyright (c) 2016-2019 Ulrik Sverdrup "bluss" and scopeguard developers

### send_wrapper 0.6.0
`MIT/Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Thomas Keh

### serde 1.0.229
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>

### serde_core 1.0.229
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>

### serde_json 1.0.151
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Erick Tryzelaar <erick.tryzelaar@gmail.com>, David Tolnay <dtolnay@gmail.com>

### serde_qs 0.15.0
`MIT/Apache-2.0`

- Copyright (c) 2016 Anthony Ramine
- Copyright (c) 2017 Sam Scott
- Copyright (c) 2017 Google Inc.

### serde_spanned 1.1.1
`MIT OR Apache-2.0`

- Copyright (c) Individual contributors

### server_fn 0.8.13
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston, Ben Wishovich

### server_fn_macro 0.8.10
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### sha2 0.10.9
`MIT OR Apache-2.0`

- Copyright (c) 2006-2009 Graydon Hoare
- Copyright (c) 2009-2013 Mozilla Foundation
- Copyright (c) 2016 Artyom Pavlov

### slab 0.4.12
`MIT`

- Copyright (c) 2019 Carl Lerche

### slotmap 1.1.1
`Zlib`

- Copyright (c) 2021 Orson Peters <orsonpeters@gmail.com>

### smallvec 1.16.1
`MIT OR Apache-2.0`

- Copyright (c) 2018 The Servo Project Developers

### spirv 0.4.0+sdk-1.4.341.0
`Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Lei Zhang <antiagainst@gmail.com>

### stable_deref_trait 1.2.1
`MIT OR Apache-2.0`

- Copyright (c) 2017 Robert Grosse

### static_assertions 1.1.0
`MIT OR Apache-2.0`

- Copyright (c) 2017 Nikolai Vazquez

### syn 2.0.119
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) David Tolnay <dtolnay@gmail.com>

### syn 3.0.6
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) David Tolnay <dtolnay@gmail.com>

### synstructure 0.14.0
`MIT`

- Copyright 2016 Nika Layzell

### tachys 0.2.18
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### termcolor 1.4.1
`Unlicense OR MIT`

- Copyright (c) 2015 Andrew Gallant
- In jurisdictions that recognize copyright laws, the author or authors

### thiserror 1.0.69
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) David Tolnay <dtolnay@gmail.com>

### thiserror 2.0.20
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) David Tolnay <dtolnay@gmail.com>

### throw_error 0.3.1
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston

### tinystr 0.8.4
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### toml 1.1.6+spec-1.1.0
`MIT OR Apache-2.0`

- Copyright (c) Individual contributors

### toml_datetime 1.1.1+spec-1.1.0
`MIT OR Apache-2.0`

- Copyright (c) Individual contributors

### toml_parser 1.1.3+spec-1.1.0
`MIT OR Apache-2.0`

- Copyright (c) Individual contributors

### typed-builder 0.23.2
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) IdanArye <idanarye@gmail.com>, Chris Morgan <me@chrismorgan.info>

### typenum 1.20.1
`MIT OR Apache-2.0`

- Copyright 2014 Paho Lurie-Gregg
- Copyright (c) 2014 Paho Lurie-Gregg

### unicode-ident 1.0.26
`(MIT OR Apache-2.0) AND Unicode-3.0`

- Copyright © 1991-2023 Unicode, Inc.

### unicode-segmentation 1.13.3
`MIT OR Apache-2.0`

- Copyright (c) 2015 The Rust Project Developers

### unicode-width 0.2.2
`MIT OR Apache-2.0`

- Copyright (c) 2015 The Rust Project Developers

### unicode-xid 0.2.6
`MIT OR Apache-2.0`

- Copyright (c) 2015 The Rust Project Developers

### url 2.5.8
`MIT OR Apache-2.0`

- Copyright (c) 2013-2025 The rust-url developers

### utf8_iter 1.0.4
`Apache-2.0 OR MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Henri Sivonen <hsivonen@hsivonen.fi>

### uuid 1.26.1
`Apache-2.0 OR MIT`

- Copyright (c) 2014 The Rust Project Developers
- Copyright (c) 2018 Ashley Mannix, Christopher Armstrong, Dylan DPC, Hunar Roop Kahlon

### walkdir 2.5.0
`Unlicense/MIT`

- Copyright (c) 2015 Andrew Gallant
- In jurisdictions that recognize copyright laws, the author or authors

### wasm-bindgen 0.2.128
`MIT OR Apache-2.0`

- Copyright (c) 2014 Alex Crichton

### wasm-bindgen-futures 0.4.78
`MIT OR Apache-2.0`

- Copyright (c) 2014 Alex Crichton

### wasm-bindgen-macro-support 0.2.128
`MIT OR Apache-2.0`

- Copyright (c) 2014 Alex Crichton

### wasm-bindgen-shared 0.2.128
`MIT OR Apache-2.0`

- Copyright (c) 2014 Alex Crichton

### wasm-streams 0.5.0
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Mattias Buelens <mattias@buelens.com>

### wasm_split_helpers 0.2.3
`MIT OR Apache-2.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Greg Johnston <greg.johnston@gmail.com>, Jeremy Maitin-Shepard <jbms@google.com>, Martin Molzer <worldsbegin@gmx.de>

### web-sys 0.3.105
`MIT OR Apache-2.0`

- Copyright (c) 2014 Alex Crichton

### wgpu 30.0.1
`MIT OR Apache-2.0`

- Copyright (c) 2025 The gfx-rs developers

### wgpu-core 30.0.1
`MIT OR Apache-2.0`

- Copyright (c) 2025 The gfx-rs developers

### wgpu-hal 30.0.1
`MIT OR Apache-2.0`

- Copyright (c) 2025 The gfx-rs developers

### wgpu-naga-bridge 30.0.1
`MIT OR Apache-2.0`

- Copyright (c) 2025 The gfx-rs developers

### wgpu-types 30.0.1
`MIT OR Apache-2.0`

- Copyright (c) 2025 The gfx-rs developers

### winnow 1.0.4
`MIT`

- (著作権表示の記載なし。配布元を参照)

### writeable 0.6.4
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### xxhash-rust 0.8.18
`BSL-1.0`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) Douman <douman@gmx.se>

### yansi 1.0.1
`MIT OR Apache-2.0`

- Copyright 2017 Sergio Benitez
- Copyright (c) 2017 Sergio Benitez

### yoke 0.8.3
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### zerocopy 0.8.57
`BSD-2-Clause OR Apache-2.0 OR MIT`

- Copyright 2023 The Fuchsia Authors
- Copyright 2019 The Fuchsia Authors.

### zerofrom 0.1.8
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### zerotrie 0.2.5
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### zerovec 0.11.8
`Unicode-3.0`

- Copyright © 2020-2024 Unicode, Inc.

### zmij 1.0.23
`MIT`

- (著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) David Tolnay <dtolnay@gmail.com>

## 6. ライセンス全文

上の各項目で使われているライセンスの全文です。MITライセンスは、著作権者ごとに`Copyright (c) <year> <copyright holders>`の部分だけが異なるので、雛形を1つだけ載せ、実際の著作権表示は[5.](#5-著作権表示rustクレートごと)(Rustクレート)と、[3.](#3-サーバーsim_serverに含まれるものc)のC++ライブラリの記載を参照してください。

### MIT

(雛形)

```text
MIT License

Copyright (c) <year> <copyright holders>

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### Apache-2.0

(`arrayvec`クレート同梱のファイルから)

```text
                              Apache License
                        Version 2.0, January 2004
                     http://www.apache.org/licenses/

TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

1. Definitions.

   "License" shall mean the terms and conditions for use, reproduction,
   and distribution as defined by Sections 1 through 9 of this document.

   "Licensor" shall mean the copyright owner or entity authorized by
   the copyright owner that is granting the License.

   "Legal Entity" shall mean the union of the acting entity and all
   other entities that control, are controlled by, or are under common
   control with that entity. For the purposes of this definition,
   "control" means (i) the power, direct or indirect, to cause the
   direction or management of such entity, whether by contract or
   otherwise, or (ii) ownership of fifty percent (50%) or more of the
   outstanding shares, or (iii) beneficial ownership of such entity.

   "You" (or "Your") shall mean an individual or Legal Entity
   exercising permissions granted by this License.

   "Source" form shall mean the preferred form for making modifications,
   including but not limited to software source code, documentation
   source, and configuration files.

   "Object" form shall mean any form resulting from mechanical
   transformation or translation of a Source form, including but
   not limited to compiled object code, generated documentation,
   and conversions to other media types.

   "Work" shall mean the work of authorship, whether in Source or
   Object form, made available under the License, as indicated by a
   copyright notice that is included in or attached to the work
   (an example is provided in the Appendix below).

   "Derivative Works" shall mean any work, whether in Source or Object
   form, that is based on (or derived from) the Work and for which the
   editorial revisions, annotations, elaborations, or other modifications
   represent, as a whole, an original work of authorship. For the purposes
   of this License, Derivative Works shall not include works that remain
   separable from, or merely link (or bind by name) to the interfaces of,
   the Work and Derivative Works thereof.

   "Contribution" shall mean any work of authorship, including
   the original version of the Work and any modifications or additions
   to that Work or Derivative Works thereof, that is intentionally
   submitted to Licensor for inclusion in the Work by the copyright owner
   or by an individual or Legal Entity authorized to submit on behalf of
   the copyright owner. For the purposes of this definition, "submitted"
   means any form of electronic, verbal, or written communication sent
   to the Licensor or its representatives, including but not limited to
   communication on electronic mailing lists, source code control systems,
   and issue tracking systems that are managed by, or on behalf of, the
   Licensor for the purpose of discussing and improving the Work, but
   excluding communication that is conspicuously marked or otherwise
   designated in writing by the copyright owner as "Not a Contribution."

   "Contributor" shall mean Licensor and any individual or Legal Entity
   on behalf of whom a Contribution has been received by Licensor and
   subsequently incorporated within the Work.

2. Grant of Copyright License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   copyright license to reproduce, prepare Derivative Works of,
   publicly display, publicly perform, sublicense, and distribute the
   Work and such Derivative Works in Source or Object form.

3. Grant of Patent License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   (except as stated in this section) patent license to make, have made,
   use, offer to sell, sell, import, and otherwise transfer the Work,
   where such license applies only to those patent claims licensable
   by such Contributor that are necessarily infringed by their
   Contribution(s) alone or by combination of their Contribution(s)
   with the Work to which such Contribution(s) was submitted. If You
   institute patent litigation against any entity (including a
   cross-claim or counterclaim in a lawsuit) alleging that the Work
   or a Contribution incorporated within the Work constitutes direct
   or contributory patent infringement, then any patent licenses
   granted to You under this License for that Work shall terminate
   as of the date such litigation is filed.

4. Redistribution. You may reproduce and distribute copies of the
   Work or Derivative Works thereof in any medium, with or without
   modifications, and in Source or Object form, provided that You
   meet the following conditions:

   (a) You must give any other recipients of the Work or
       Derivative Works a copy of this License; and

   (b) You must cause any modified files to carry prominent notices
       stating that You changed the files; and

   (c) You must retain, in the Source form of any Derivative Works
       that You distribute, all copyright, patent, trademark, and
       attribution notices from the Source form of the Work,
       excluding those notices that do not pertain to any part of
       the Derivative Works; and

   (d) If the Work includes a "NOTICE" text file as part of its
       distribution, then any Derivative Works that You distribute must
       include a readable copy of the attribution notices contained
       within such NOTICE file, excluding those notices that do not
       pertain to any part of the Derivative Works, in at least one
       of the following places: within a NOTICE text file distributed
       as part of the Derivative Works; within the Source form or
       documentation, if provided along with the Derivative Works; or,
       within a display generated by the Derivative Works, if and
       wherever such third-party notices normally appear. The contents
       of the NOTICE file are for informational purposes only and
       do not modify the License. You may add Your own attribution
       notices within Derivative Works that You distribute, alongside
       or as an addendum to the NOTICE text from the Work, provided
       that such additional attribution notices cannot be construed
       as modifying the License.

   You may add Your own copyright statement to Your modifications and
   may provide additional or different license terms and conditions
   for use, reproduction, or distribution of Your modifications, or
   for any such Derivative Works as a whole, provided Your use,
   reproduction, and distribution of the Work otherwise complies with
   the conditions stated in this License.

5. Submission of Contributions. Unless You explicitly state otherwise,
   any Contribution intentionally submitted for inclusion in the Work
   by You to the Licensor shall be under the terms and conditions of
   this License, without any additional terms or conditions.
   Notwithstanding the above, nothing herein shall supersede or modify
   the terms of any separate license agreement you may have executed
   with Licensor regarding such Contributions.

6. Trademarks. This License does not grant permission to use the trade
   names, trademarks, service marks, or product names of the Licensor,
   except as required for reasonable and customary use in describing the
   origin of the Work and reproducing the content of the NOTICE file.

7. Disclaimer of Warranty. Unless required by applicable law or
   agreed to in writing, Licensor provides the Work (and each
   Contributor provides its Contributions) on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
   implied, including, without limitation, any warranties or conditions
   of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
   PARTICULAR PURPOSE. You are solely responsible for determining the
   appropriateness of using or redistributing the Work and assume any
   risks associated with Your exercise of permissions under this License.

8. Limitation of Liability. In no event and under no legal theory,
   whether in tort (including negligence), contract, or otherwise,
   unless required by applicable law (such as deliberate and grossly
   negligent acts) or agreed to in writing, shall any Contributor be
   liable to You for damages, including any direct, indirect, special,
   incidental, or consequential damages of any character arising as a
   result of this License or out of the use or inability to use the
   Work (including but not limited to damages for loss of goodwill,
   work stoppage, computer failure or malfunction, or any and all
   other commercial damages or losses), even if such Contributor
   has been advised of the possibility of such damages.

9. Accepting Warranty or Additional Liability. While redistributing
   the Work or Derivative Works thereof, You may choose to offer,
   and charge a fee for, acceptance of support, warranty, indemnity,
   or other liability obligations and/or rights consistent with this
   License. However, in accepting such obligations, You may act only
   on Your own behalf and on Your sole responsibility, not on behalf
   of any other Contributor, and only if You agree to indemnify,
   defend, and hold each Contributor harmless for any liability
   incurred by, or claims asserted against, such Contributor by reason
   of your accepting any such warranty or additional liability.

END OF TERMS AND CONDITIONS

APPENDIX: How to apply the Apache License to your work.

   To apply the Apache License to your work, attach the following
   boilerplate notice, with the fields enclosed by brackets "[]"
   replaced with your own identifying information. (Don't include
   the brackets!)  The text should be enclosed in the appropriate
   comment syntax for the file format. We also recommend that a
   file or class name and description of purpose be included on the
   same "printed page" as the copyright notice for easier
   identification within third-party archives.

Copyright [yyyy] [name of copyright owner]

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

	http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

### Zlib

(`bytemuck`クレート同梱のファイルから)

```text
Copyright (c) 2019 Daniel "Lokathor" Gee.

This software is provided 'as-is', without any express or implied warranty. In no event will the authors be held liable for any damages arising from the use of this software.

Permission is granted to anyone to use this software for any purpose, including commercial applications, and to alter it and redistribute it freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not claim that you wrote the original software. If you use this software in a product, an acknowledgment in the product documentation would be appreciated but is not required.

2. Altered source versions must be plainly marked as such, and must not be misrepresented as being the original software.

3. This notice may not be removed or altered from any source distribution.
```

### Unicode-3.0

(`icu_collections`クレート同梱のファイルから)

```text
UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 2020-2024 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.

SPDX-License-Identifier: Unicode-3.0

—

Portions of ICU4X may have been adapted from ICU4C and/or ICU4J.
ICU 1.8.1 to ICU 57.1 © 1995-2016 International Business Machines Corporation and others.
```

### BSD-2-Clause

(`zerocopy`クレート同梱のファイルから)

```text
Copyright 2019 The Fuchsia Authors.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

   * Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.
   * Redistributions in binary form must reproduce the above
copyright notice, this list of conditions and the following disclaimer
in the documentation and/or other materials provided with the
distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### ISC

(`earcutr`クレート同梱のファイルから)

```text
ISC License

Copyright (c) 2016, Mapbox
Copyright (c) 2018, Tree Cricket

Permission to use, copy, modify, and/or distribute this software for any purpose
with or without fee is hereby granted, provided that the above copyright notice
and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH
REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND
FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT,
INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS
OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER
TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF
THIS SOFTWARE.
```

### BSL-1.0

```text
Boost Software License - Version 1.0 - August 17th, 2003

Permission is hereby granted, free of charge, to any person or organization
obtaining a copy of the software and accompanying documentation covered by
this license (the "Software") to use, reproduce, display, distribute,
execute, and transmit the Software, and to prepare derivative works of the
Software, and to permit third-parties to whom the Software is furnished to
do so, all subject to the following:

The copyright notices in the Software and this entire statement, including
the above license grant, this restriction and the following disclaimer,
must be included in all copies of the Software, in whole or in part, and
all derivative works of the Software, unless such copies or derivative
works are solely in the form of machine-executable object code generated by
a source language processor.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE, TITLE AND NON-INFRINGEMENT. IN NO EVENT
SHALL THE COPYRIGHT HOLDERS OR ANYONE DISTRIBUTING THE SOFTWARE BE LIABLE
FOR ANY DAMAGES OR OTHER LIABILITY, WHETHER IN CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

### CC0-1.0

(`base16`クレート同梱のファイルから)

```text
Creative Commons Legal Code

CC0 1.0 Universal

    CREATIVE COMMONS CORPORATION IS NOT A LAW FIRM AND DOES NOT PROVIDE
    LEGAL SERVICES. DISTRIBUTION OF THIS DOCUMENT DOES NOT CREATE AN
    ATTORNEY-CLIENT RELATIONSHIP. CREATIVE COMMONS PROVIDES THIS
    INFORMATION ON AN "AS-IS" BASIS. CREATIVE COMMONS MAKES NO WARRANTIES
    REGARDING THE USE OF THIS DOCUMENT OR THE INFORMATION OR WORKS
    PROVIDED HEREUNDER, AND DISCLAIMS LIABILITY FOR DAMAGES RESULTING FROM
    THE USE OF THIS DOCUMENT OR THE INFORMATION OR WORKS PROVIDED
    HEREUNDER.

Statement of Purpose

The laws of most jurisdictions throughout the world automatically confer
exclusive Copyright and Related Rights (defined below) upon the creator
and subsequent owner(s) (each and all, an "owner") of an original work of
authorship and/or a database (each, a "Work").

Certain owners wish to permanently relinquish those rights to a Work for
the purpose of contributing to a commons of creative, cultural and
scientific works ("Commons") that the public can reliably and without fear
of later claims of infringement build upon, modify, incorporate in other
works, reuse and redistribute as freely as possible in any form whatsoever
and for any purposes, including without limitation commercial purposes.
These owners may contribute to the Commons to promote the ideal of a free
culture and the further production of creative, cultural and scientific
works, or to gain reputation or greater distribution for their Work in
part through the use and efforts of others.

For these and/or other purposes and motivations, and without any
expectation of additional consideration or compensation, the person
associating CC0 with a Work (the "Affirmer"), to the extent that he or she
is an owner of Copyright and Related Rights in the Work, voluntarily
elects to apply CC0 to the Work and publicly distribute the Work under its
terms, with knowledge of his or her Copyright and Related Rights in the
Work and the meaning and intended legal effect of CC0 on those rights.

1. Copyright and Related Rights. A Work made available under CC0 may be
protected by copyright and related or neighboring rights ("Copyright and
Related Rights"). Copyright and Related Rights include, but are not
limited to, the following:

  i. the right to reproduce, adapt, distribute, perform, display,
     communicate, and translate a Work;
 ii. moral rights retained by the original author(s) and/or performer(s);
iii. publicity and privacy rights pertaining to a person's image or
     likeness depicted in a Work;
 iv. rights protecting against unfair competition in regards to a Work,
     subject to the limitations in paragraph 4(a), below;
  v. rights protecting the extraction, dissemination, use and reuse of data
     in a Work;
 vi. database rights (such as those arising under Directive 96/9/EC of the
     European Parliament and of the Council of 11 March 1996 on the legal
     protection of databases, and under any national implementation
     thereof, including any amended or successor version of such
     directive); and
vii. other similar, equivalent or corresponding rights throughout the
     world based on applicable law or treaty, and any national
     implementations thereof.

2. Waiver. To the greatest extent permitted by, but not in contravention
of, applicable law, Affirmer hereby overtly, fully, permanently,
irrevocably and unconditionally waives, abandons, and surrenders all of
Affirmer's Copyright and Related Rights and associated claims and causes
of action, whether now known or unknown (including existing as well as
future claims and causes of action), in the Work (i) in all territories
worldwide, (ii) for the maximum duration provided by applicable law or
treaty (including future time extensions), (iii) in any current or future
medium and for any number of copies, and (iv) for any purpose whatsoever,
including without limitation commercial, advertising or promotional
purposes (the "Waiver"). Affirmer makes the Waiver for the benefit of each
member of the public at large and to the detriment of Affirmer's heirs and
successors, fully intending that such Waiver shall not be subject to
revocation, rescission, cancellation, termination, or any other legal or
equitable action to disrupt the quiet enjoyment of the Work by the public
as contemplated by Affirmer's express Statement of Purpose.

3. Public License Fallback. Should any part of the Waiver for any reason
be judged legally invalid or ineffective under applicable law, then the
Waiver shall be preserved to the maximum extent permitted taking into
account Affirmer's express Statement of Purpose. In addition, to the
extent the Waiver is so judged Affirmer hereby grants to each affected
person a royalty-free, non transferable, non sublicensable, non exclusive,
irrevocable and unconditional license to exercise Affirmer's Copyright and
Related Rights in the Work (i) in all territories worldwide, (ii) for the
maximum duration provided by applicable law or treaty (including future
time extensions), (iii) in any current or future medium and for any number
of copies, and (iv) for any purpose whatsoever, including without
limitation commercial, advertising or promotional purposes (the
"License"). The License shall be deemed effective as of the date CC0 was
applied by Affirmer to the Work. Should any part of the License for any
reason be judged legally invalid or ineffective under applicable law, such
partial invalidity or ineffectiveness shall not invalidate the remainder
of the License, and in such case Affirmer hereby affirms that he or she
will not (i) exercise any of his or her remaining Copyright and Related
Rights in the Work or (ii) assert any associated claims and causes of
action with respect to the Work, in either case contrary to Affirmer's
express Statement of Purpose.

4. Limitations and Disclaimers.

 a. No trademark or patent rights held by Affirmer are waived, abandoned,
    surrendered, licensed or otherwise affected by this document.
 b. Affirmer offers the Work as-is and makes no representations or
    warranties of any kind concerning the Work, express, implied,
    statutory or otherwise, including without limitation warranties of
    title, merchantability, fitness for a particular purpose, non
    infringement, or the absence of latent or other defects, accuracy, or
    the present or absence of errors, whether or not discoverable, all to
    the greatest extent permissible under applicable law.
 c. Affirmer disclaims responsibility for clearing rights of other persons
    that may apply to the Work or any use thereof, including without
    limitation any person's Copyright and Related Rights in the Work.
    Further, Affirmer disclaims responsibility for obtaining any necessary
    consents, permissions or other rights required for any use of the
    Work.
 d. Affirmer understands and acknowledges that Creative Commons is not a
    party to this document and has no duty or obligation with respect to
    this CC0 or use of the Work.
```

### Unlicense

(`aho-corasick`クレート同梱のファイルから)

```text
This is free and unencumbered software released into the public domain.

Anyone is free to copy, modify, publish, use, compile, sell, or
distribute this software, either in source code form or as a compiled
binary, for any purpose, commercial or non-commercial, and by any
means.

In jurisdictions that recognize copyright laws, the author or authors
of this software dedicate any and all copyright interest in the
software to the public domain. We make this dedication for the benefit
of the public at large and to the detriment of our heirs and
successors. We intend this dedication to be an overt act of
relinquishment in perpetuity of all present and future rights to this
software under copyright law.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
IN NO EVENT SHALL THE AUTHORS BE LIABLE FOR ANY CLAIM, DAMAGES OR
OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
OTHER DEALINGS IN THE SOFTWARE.

For more information, please refer to <http://unlicense.org/>
```

### Electronデスクトップ版の追加依存

`sample/sim_desktop`はElectron 44.4.4(MIT)を使用します。配布元: <https://github.com/electron/electron>。
Electronに同梱されるChromium・Node.js等のライセンス/著作権表示は、配布フォルダーの`LICENSE`と`LICENSES.chromium.html`を参照してください。`package.cjs`はElectronの配布ファイルをそのままコピーし、これらの表示を保持します。

npmの取得・展開用依存は開発時だけ使用し、アプリの`node_modules`は配布しません。ブラウザ版にはElectronは含まれません。

## 7. この文書に含めないもの・更新方法

**含めないもの**
- 単体テスト専用の依存(`naga`)。テストのときだけビルドされ、配布物には入らない
- ビルド時だけ動くツール(`trunk`・`wasm-bindgen`のCLI・`cargo`・vcpkg本体・CMake)。生成物には入らない(ただし`wasm-bindgen`が生成するJSグルーコードは、`wasm-bindgen`クレートとして2.に含めている)
- proc-macroクレート(`serde_derive`・`leptos_macro`など)。コンパイル時に動き、生成物には含まれない
- ブラウザ・OSが提供する機能(WebGPU・WebSocket・システムフォントなど)。フォントは、CSSで`system-ui, sans-serif`を指定しているだけで、フォントファイルは同梱していない
- 解説ノート(Artifact「Sim3dViewのしくみ」)。Google Fontsを外部から読み込んでいるが、このリポジトリの配布物には含まれない

**更新方法**(依存を追加・更新したときに再実行する)
```powershell
# リポジトリのルートで実行する(cargo metadataは、スクリプトが自分で実行する)
python sample/scripts/gen_third_party_notice.py
```
C++側(3.・4.)の版とライセンスは、`sample/sim_server/vcpkg.json`・`tools/geotiff_preprocess/vcpkg.json`とサブモジュールの版(`git submodule status`)を見て、手で更新してください。
