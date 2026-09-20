"""THIRD_PARTY_NOTICE.md を、Cargo.lock(cargo metadata)とC++側のライセンスファイルから生成する。

使い方(リポジトリのルートで): python scripts/gen_third_party_notice.py
C++側(vcpkg・サブモジュール)の版とライセンスは、下の文面を手で更新する。
"""
import glob
import json
import os
import re
import sys
from datetime import date

import subprocess

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__))).replace(chr(92), '/') + '/'
OUT = REPO + 'THIRD_PARTY_NOTICE.md'

# wasm32向けに解決した依存(Cargo.lock)を、cargo metadataから取る
meta = subprocess.run(
    ['cargo', 'metadata', '--format-version', '1', '--filter-platform', 'wasm32-unknown-unknown', '--locked'],
    cwd=REPO, capture_output=True, check=True,
).stdout
m = json.loads(meta.decode('utf-8'))
pk = {p['id']: p for p in m['packages']}
nodes = {n['id']: n for n in m['resolve']['nodes']}
roots = [i for i, p in pk.items() if p['name'] in ('sim3dview', 'sim_frontend')]
seen, stack = set(), list(roots)
while stack:
    i = stack.pop()
    if i in seen:
        continue
    seen.add(i)
    for d in nodes[i]['deps']:
        if None in [k['kind'] for k in d['dep_kinds']]:
            stack.append(d['pkg'])


def is_pm(p):
    return any('proc-macro' in t['kind'] for t in p['targets'])


crates = sorted(
    (pk[i] for i in seen if pk[i]['name'] not in ('sim3dview', 'sim_frontend') and not is_pm(pk[i])),
    key=lambda p: (p['name'], p['version']),
)


def read(path):
    for enc in ('utf-8', 'utf-16', 'latin-1'):
        try:
            return open(path, encoding=enc).read()
        except Exception:
            continue
    return ''


LICENSE_GLOBS = ['LICENSE*', 'LICENCE*', 'COPYING*', 'UNLICENSE*', 'NOTICE*', 'license*', 'COPYRIGHT*']


def license_files(p):
    d = os.path.dirname(p['manifest_path'])
    files = []
    for g in LICENSE_GLOBS:
        files += glob.glob(os.path.join(d, g))
    return sorted(set(f for f in files if os.path.isfile(f)))


YEAR = re.compile(r'(?i)copyright.*(19|20)\d\d')
WHO = re.compile(r'(?i)copyright.*(contributors|authors|developers|project)')
TEMPLATE = re.compile(r'\[yyyy\]|\{yyyy\}|<year>|owner|holder|notice|license', re.I)


def copyrights(p):
    found = []
    for f in license_files(p):
        for line in read(f).splitlines():
            line = line.strip().strip('*#/ ').strip()
            if not line or len(line) > 220:
                continue
            if (YEAR.search(line) or WHO.search(line)) and not re.search(r'\[yyyy\]|\{yyyy\}|<year>', line, re.I):
                if re.search(r'(?i)copyright (owner|holder)s?( |$)', line) and not YEAR.search(line):
                    continue
                if line not in found:
                    found.append(line)
    if not found:
        authors = [a for a in (p.get('authors') or []) if a]
        if authors:
            found = ['(著作権表示のファイルが無いため、Cargo.tomlのauthorsを記載) ' + ', '.join(authors)]
    return found


# ---------------- ライセンス全文を、実際のファイルから拾う ----------------
MARKERS = {
    'Apache-2.0': lambda t: re.search(r'Apache License\s+Version 2\.0', t) and 'APPENDIX' in t,
    'Zlib': lambda t: "This software is provided 'as-is'" in t or 'provided \'as-is\'' in t,
    'Unicode-3.0': lambda t: 'UNICODE LICENSE V3' in t,
    'BSD-2-Clause': lambda t: 'Redistribution and use in source and binary forms' in t and 'Redistributions in binary form' in t and 'Neither the name' not in t,
    'ISC': lambda t: 'Permission to use, copy, modify, and/or distribute this software for any purpose' in t,
    'CC0-1.0': lambda t: 'CC0 1.0 Universal' in t and 'Statement of Purpose' in t,
    'Unlicense': lambda t: 'This is free and unencumbered software' in t,
}
texts = {}
for p in crates:
    for f in license_files(p):
        t = read(f)
        for k, fn in MARKERS.items():
            if k not in texts and fn(t):
                texts[k] = (p['name'], t.strip('\n'))
# Boost Software License 1.0 は msgpack-cxx のものを使う
bsl = read(REPO + 'sample/sim_server/third_party/msgpack-cxx/LICENSE_1_0.txt').strip('\n')

MIT_TEXT = '''MIT License

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
SOFTWARE.'''

# ---------------- 文書の組み立て ----------------
out = []
w = out.append


def md_escape(s):
    return s.replace('|', '\\|')


w('# サードパーティ・ソフトウェア等の表示(THIRD_PARTY_NOTICE)\n')
w('Sim3dView(`sim3dview`ライブラリ・`sample/sim_frontend`・`sample/sim_server`・`tools/geotiff_preprocess`)が、'
  '利用・同梱・リンクしているサードパーティのソフトウェアとデータの一覧、その著作権表示とライセンス条件です。\n')
w('> **この文書について**: 依存関係とライセンスの情報を機械的に集めた**参考資料**で、法的な助言ではありません。'
  '製品として配布する前に、最新の依存関係(`Cargo.lock`・`vcpkg.json`)で再生成し、必要に応じて法務の確認を受けてください。'
  '本プロジェクト自身のライセンスは、このファイルの対象外です(リポジトリにLICENSEファイルはまだありません)。\n')
w(f'- 生成日: {date.today().isoformat()}(`Cargo.lock`の内容とvcpkg・サブモジュールの版に基づく)')
w('- 再生成の手順は末尾の「この文書の更新方法」を参照\n')

w('## 目次\n')
w('1. [地形データ: ALOS World 3D-30m (AW3D30)](#1-地形データ-alos-world-3d-30m-aw3d30)')
w('2. [ブラウザに配布されるもの(Rustクレート)](#2-ブラウザに配布されるものrustクレート)')
w('3. [サーバー(`sim_server`)に含まれるもの(C++)](#3-サーバーsim_serverに含まれるものc)')
w('4. [前処理ツール(`geotiff_preprocess`)が使うもの](#4-前処理ツールgeotiff_preprocessが使うもの)')
w('5. [著作権表示(Rustクレートごと)](#5-著作権表示rustクレートごと)')
w('6. [ライセンス全文](#6-ライセンス全文)')
w('7. [この文書に含めないもの・更新方法](#7-この文書に含めないもの更新方法)\n')

# ---- 1. データ
w('## 1. 地形データ: ALOS World 3D-30m (AW3D30)\n')
w('`map_data/`の標高データ(`ALPSMLC30_*`。リポジトリには含まれず、ローカルに置く入力データ)と、それを`geotiff_preprocess`で変換した'
  '`assets/terrain/`の地形タイル(`metadata.json`・`base.bin`・`tiles/`)は、JAXA(宇宙航空研究開発機構)の'
  '**ALOS World 3D-30m(AW3D30)** から作られた二次的なデータです。\n')
w('| 項目 | 内容 |')
w('|---|---|')
w('| データ名 | ALOS World 3D - 30m (AW3D30)(ファイル名の接頭辞は`ALPSMLC30`) |')
w('| 提供元 | JAXA / EORC(地球観測研究センター)。データの取得元は<https://www.eorc.jaxa.jp/ALOS/en/dataset/aw3d30/aw3d30_e.htm> |')
w('| 利用条件 | JAXAの利用条件(<https://earth.jaxa.jp/en/data/policy/>)に従う |')
w('| クレジット | **JAXAがデータの提供元であることを明示する**こと。この条件のもとでの例示は「Credit: XXXX (JAXA)」の形 |')
w('| 商用利用 | 利用条件のページでは、商用利用の場合は**事前にJAXAへ通知が必要**とされている(AW3D30のデータ提供ページは「無償で、商用・非商用を問わず利用できる」と書いており、表現が一致していない) |')
w('| 再配布・二次的データ | 利用条件に従って可能。二次的なデータを配布するときは、JAXAとその他の関与した組織の両方をクレジットする |')
w('| 保証 | JAXAは、データの利用またはその品質による結果に責任を負わない |')
w('| 問い合わせ | earth@ml.jaxa.jp(JAXAの利用条件ページに記載) |\n')
w('**推奨するクレジット表記の例**(アプリの「ヘルプ」や画面の隅などに置く):\n')
w('```text')
w('Elevation data: ALOS World 3D - 30m (AW3D30), provided by the Japan Aerospace Exploration Agency (JAXA).')
w('標高データ: ALOS World 3D - 30m (AW3D30)(提供: 宇宙航空研究開発機構 JAXA)。地形タイルは、このデータをSim3dViewの前処理ツールで変換したものです。')
w('```\n')
w('> **要確認**: 上の内容は2026年9月時点でJAXAのWebページに書かれていた条件の要約です。**商用で配布・提供する場合は、'
  'JAXAの現行の利用条件を直接確認し、必要ならJAXAへ事前に通知してください**(特に、事前通知の要否と、地形タイルを配布物・サービスに含めることの扱い)。\n')

# ---- 2. Rustクレート
w('## 2. ブラウザに配布されるもの(Rustクレート)\n')
w('`sim3dview`と`sample/sim_frontend`をWebAssemblyにビルドしたときに、実行時にリンクされるクレートです'
  f'(`Cargo.lock`から`wasm32-unknown-unknown`向けに解決した**{len(crates)}個**。単体テスト専用の依存(`naga`)、ビルド時だけ動くもの'
  '(proc-macro・ビルドスクリプトの依存)、開発用ツール(`trunk`・`wasm-bindgen`のCLI)は含めない)。\n')
w('ライセンスが「A OR B」の形のクレートは、AとBのどちらの条件でも利用できる二重ライセンスです。\n')
c = {}
for p in crates:
    c[p['license']] = c.get(p['license'], 0) + 1
w('**ライセンスの内訳**:\n')
for k, v in sorted(c.items(), key=lambda kv: (-kv[1], kv[0])):
    w(f'- {v}個: `{k}`')
w('')
w('主要なクレートの役割: `leptos`(UIフレームワーク)、`wgpu`(WebGPU描画)、`glam`(ベクトル・行列)、`earcutr`(多角形の三角形分割)、`serde`/`serde_json`/`rmp-serde`(シリアライズ)、'
  '`gloo-net`/`gloo-timers`/`web-sys`/`wasm-bindgen`/`js-sys`(ブラウザAPI)、`bytemuck`(GPUバッファへのコピー)。\n')
w('| クレート | 版 | ライセンス | 配布元 |')
w('|---|---|---|---|')
for p in crates:
    repo = p.get('repository') or p.get('homepage') or f"https://crates.io/crates/{p['name']}"
    w(f"| {md_escape(p['name'])} | {p['version']} | {md_escape(p['license'])} | <{repo}> |")
w('')
w('WebAssemblyにはRustの標準ライブラリ(`std`・`core`・`alloc`、`compiler_builtins`など。`MIT OR Apache-2.0`)のコードも含まれます。'
  '配布元: <https://github.com/rust-lang/rust>\n')

# ---- 3. サーバー
w('## 3. サーバー(`sim_server`)に含まれるもの(C++)\n')
w('`sample/sim_server`をビルドした`sim_server.exe`に、ソースの取り込み(サブモジュール)またはvcpkgのライブラリとしてリンクされます。\n')
w('| ライブラリ | 版 | ライセンス | 配布元・ライセンスファイル |')
w('|---|---|---|---|')
w('| uWebSockets | v20.80.0系(サブモジュール) | Apache-2.0 | <https://github.com/uNetworking/uWebSockets> (`sample/sim_server/third_party/uWebSockets/LICENSE`) |')
w('| uSockets | v0.8.8系(uWebSocketsに同梱) | Apache-2.0 | <https://github.com/uNetworking/uSockets> (`.../uWebSockets/uSockets/LICENSE`) |')
w('| msgpack-c(C++版、msgpack-cxx) | cpp-9.0.0(サブモジュール) | BSL-1.0(Boost Software License 1.0)。著作権表示: Copyright (C) 2008-2015 FURUHASHI Sadayuki | <https://github.com/msgpack/msgpack-c> (`.../msgpack-cxx/LICENSE_1_0.txt`・`COPYING`・`NOTICE`) |')
w('| libuv | 1.52.1(vcpkg) | MIT | <https://github.com/libuv/libuv> |')
w('| OpenSSL | 3.6.4(vcpkg) | Apache-2.0 | <https://www.openssl.org/> (TLS用。ビルドには常に必要) |')
w('| zlib | 1.3.2(vcpkg) | Zlib | <https://zlib.net/> |\n')
w('- msgpack-cxxは、Boost PredefとBoost Preprocessor(いずれもBoost Software License 1.0)を同梱しています(`msgpack-cxx/NOTICE`)。')
w('- uSocketsのソースには、BoringSSL・lsquicのディレクトリがありますが、`sample/sim_server/CMakeLists.txt`はこれらをビルドに使いません(TLSはvcpkgのOpenSSL)。')
w('- サーバー本体(`sample/sim_server/src`・`include`)はこのプロジェクトのコードです。')
w('- 配布物にサブモジュールのソースを含める場合は、各サブモジュールの`LICENSE`ファイルを一緒に配布してください。\n')
w('### 3.1 msgpack-cxxのライセンス全文(Boost Software License 1.0)\n')
w('```text')
w(bsl)
w('```\n')
w('uWebSockets・uSockets・OpenSSLのApache-2.0、libuvのMIT、zlibのZlibライセンスの全文は、[6. ライセンス全文](#6-ライセンス全文)にあります'
  '(libuvは`Copyright (c) 2015-present libuv project contributors.`、zlibは`(C) 1995-2026 Jean-loup Gailly and Mark Adler`)。'
  'uWebSockets・uSocketsのソースには、著作権者名の記載も`NOTICE`ファイルもありません(`LICENSE`はApache-2.0の全文のみ)。配布するときは、上流のリポジトリの表示を確認してください。\n')

# ---- 4. 前処理ツール
w('## 4. 前処理ツール(`geotiff_preprocess`)が使うもの\n')
w('`tools/geotiff_preprocess`(GeoTIFF→地形タイルの変換CLI)がvcpkg経由でリンクするライブラリです。'
  'このツールは開発・データ生成用で、ブラウザやサーバーには含まれませんが、**ツールのバイナリを配布する場合は**下記の表示が必要です。\n')
w('| ライブラリ | 版 | ライセンス(vcpkgの`copyright`による) |')
w('|---|---|---|')
for row in [
    ('GDAL', '3.12.4', 'MIT(X11)。GDAL/OGRのソースツリー内のその他のライセンスは、GDALの`LICENSE.TXT`を参照'),
    ('PROJ', '9.8.1', 'MIT'),
    ('libtiff', '4.7.2', 'libtiffライセンス(BSD風)'),
    ('libgeotiff', '1.7.4', 'MITまたはパブリックドメイン'),
    ('curl', '8.22.0', 'curlライセンス(MIT風)'),
    ('SQLite', '3.53.4', 'パブリックドメイン'),
    ('json-c', '0.19-20260627', 'MIT'),
    ('libjpeg-turbo', '3.2.0', 'BSD-3-Clause・IJGライセンス・Zlib'),
    ('liblzma(XZ Utils)', '5.8.4', '0BSD(ライブラリ本体)'),
    ('nlohmann-json', '3.12.0', 'MIT'),
    ('zlib', '1.3.2', 'Zlib'),
]:
    w(f'| {row[0]} | {row[1]} | {row[2]} |')
w('\nGDALの推移的な依存はvcpkgの版によって変わります。配布するときは、そのビルドの`vcpkg_installed/<triplet>/share/<port>/copyright`を、この表と照合してください。\n')

# ---- 5. 著作権表示
w('## 5. 著作権表示(Rustクレートごと)\n')
w('各クレートのライセンスファイル(`LICENSE*`・`COPYING*`・`NOTICE*`)から抜き出した著作権表示です。ファイルに著作権表示が無いクレートは、`Cargo.toml`の`authors`を記載しています。\n')
for p in crates:
    cr = copyrights(p)
    w(f"### {p['name']} {p['version']}")
    w(f"`{p['license']}`\n")
    if cr:
        for line in cr:
            w(f'- {md_escape(line)}')
    else:
        w('- (著作権表示の記載なし。配布元を参照)')
    w('')

# ---- 6. ライセンス全文
w('## 6. ライセンス全文\n')
w('上の各項目で使われているライセンスの全文です。MITライセンスは、著作権者ごとに`Copyright (c) <year> <copyright holders>`の部分だけが異なるので、'
  '雛形を1つだけ載せ、実際の著作権表示は[5.](#5-著作権表示rustクレートごと)(Rustクレート)と、[3.](#3-サーバーsim_serverに含まれるものc)のC++ライブラリの記載を参照してください。\n')
order = [('MIT', None), ('Apache-2.0', 'Apache-2.0'), ('Zlib', 'Zlib'), ('Unicode-3.0', 'Unicode-3.0'), ('BSD-2-Clause', 'BSD-2-Clause'),
         ('ISC', 'ISC'), ('BSL-1.0', None), ('CC0-1.0', 'CC0-1.0'), ('Unlicense', 'Unlicense')]
for name, key in order:
    w(f'### {name}\n')
    if name == 'MIT':
        body = MIT_TEXT
        w('(雛形)\n')
    elif name == 'BSL-1.0':
        body = bsl
    else:
        if key not in texts:
            body = '(全文は <https://spdx.org/licenses/%s.html> を参照)' % key
        else:
            src, body = texts[key]
            w(f'(`{src}`クレート同梱のファイルから)\n')
    w('```text')
    w(body)
    w('```\n')

# ---- 7. 含めないもの・更新
w('## 7. この文書に含めないもの・更新方法\n')
w('**含めないもの**')
w('- 単体テスト専用の依存(`naga`)。テストのときだけビルドされ、配布物には入らない')
w('- ビルド時だけ動くツール(`trunk`・`wasm-bindgen`のCLI・`cargo`・vcpkg本体・CMake)。生成物には入らない(ただし`wasm-bindgen`が生成するJSグルーコードは、`wasm-bindgen`クレートとして2.に含めている)')
w('- proc-macroクレート(`serde_derive`・`leptos_macro`など)。コンパイル時に動き、生成物には含まれない')
w('- ブラウザ・OSが提供する機能(WebGPU・WebSocket・システムフォントなど)。フォントは、CSSで`system-ui, sans-serif`を指定しているだけで、フォントファイルは同梱していない')
w('- 解説ノート(Artifact「Sim3dViewのしくみ」)。Google Fontsを外部から読み込んでいるが、このリポジトリの配布物には含まれない\n')
w('**更新方法**(依存を追加・更新したときに再実行する)')
w('```powershell')
w('# リポジトリのルートで実行する(cargo metadataは、スクリプトが自分で実行する)')
w('python scripts/gen_third_party_notice.py')
w('```')
w('C++側(3.・4.)の版とライセンスは、`sample/sim_server/vcpkg.json`・`tools/geotiff_preprocess/vcpkg.json`とサブモジュールの版(`git submodule status`)を見て、手で更新してください。')

open(OUT, 'w', encoding='utf-8', newline='\n').write('\n'.join(out) + '\n')
print('written', OUT, len(out), 'lines;', 'texts:', sorted(texts))
