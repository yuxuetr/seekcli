"""把 SUMMARY 列出的 Markdown 按仓库原结构复制进 site/src，供 mdbook 构建。

只做一件 mdbook 自己做不到的事：mdbook 只改写指向 .md 的链接，
指向目录或非 Markdown 文件的相对链接（`./examples/launchd/`、`LICENSE`）
在站点里没有对应页面，改写成 GitHub 上的源文件地址，避免 404。
"""

import posixpath
import re
import shutil
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SITE = REPO / "site"
SRC = SITE / "src"
BLOB = "https://github.com/yuxuetr/seekcli/tree/main/"

LINK = re.compile(r"\]\((?!https?://|#|mailto:)([^)\s#]+)(#[^)\s]*)?\)")
CHAPTER = re.compile(r"\]\(([^)]+\.md)\)")


def rewrite(text: str, rel_dir: str) -> str:
  def repl(m: re.Match[str]) -> str:
    target, anchor = m.group(1), m.group(2) or ""
    if target.endswith(".md"):
      return m.group(0)
    resolved = posixpath.normpath(posixpath.join(rel_dir, target))
    return f"]({BLOB}{resolved}{anchor})"

  return LINK.sub(repl, text)


def main() -> None:
  shutil.rmtree(SRC, ignore_errors=True)
  summary = (SITE / "SUMMARY.md").read_text(encoding="utf-8")
  for rel in CHAPTER.findall(summary):
    dest = SRC / rel
    dest.parent.mkdir(parents=True, exist_ok=True)
    text = (REPO / rel).read_text(encoding="utf-8")
    dest.write_text(rewrite(text, posixpath.dirname(rel)), encoding="utf-8")
  shutil.copy(SITE / "SUMMARY.md", SRC / "SUMMARY.md")


if __name__ == "__main__":
  main()
