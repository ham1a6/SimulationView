#!/usr/bin/env python3
"""docs/impl/*.md に埋め込んだWGSLを、実ファイルと同期する。

ドキュメント中の
    <!-- BEGIN-WGSL sim3dview/src/terrain/terrain.wgsl -->
    <!-- END-WGSL -->
の間を、そのファイルの内容(```wgsl のコードブロック)で置き換える。

  python scripts/sync_impl_wgsl.py           # ドキュメントを実ファイルに合わせて書き換える
  python scripts/sync_impl_wgsl.py --check   # 食い違いがあれば内容を表示して終了コード1(書き換えない)

シェーダーを編集したら、コミット前にこのスクリプトを実行すること
(再実装用ドキュメントが実装とずれないようにするため)。
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
DOCS = sorted((ROOT / "docs" / "impl").glob("*.md"))
BLOCK = re.compile(
    r"(<!-- BEGIN-WGSL (?P<path>[^ >]+) -->\n)(?P<body>.*?)(<!-- END-WGSL -->)",
    re.DOTALL,
)


def render(path: str) -> str:
    source = (ROOT / path).read_text(encoding="utf-8")
    if not source.endswith("\n"):
        source += "\n"
    return f"```wgsl\n{source}```\n"


def main() -> int:
    check = "--check" in sys.argv[1:]
    stale = []
    for doc in DOCS:
        text = doc.read_text(encoding="utf-8")

        def replace(m: re.Match) -> str:
            return m.group(1) + render(m.group("path")) + m.group(4)

        new_text = BLOCK.sub(replace, text)
        if new_text != text:
            stale.append(doc)
            if not check:
                doc.write_text(new_text, encoding="utf-8", newline="\n")
    if check:
        if stale:
            for doc in stale:
                print(f"out of sync: {doc.relative_to(ROOT)}")
            print("run: python scripts/sync_impl_wgsl.py")
            return 1
        print("docs/impl WGSL blocks are in sync")
    else:
        for doc in stale:
            print(f"updated: {doc.relative_to(ROOT)}")
        if not stale:
            print("nothing to update")
    return 0


if __name__ == "__main__":
    sys.exit(main())
