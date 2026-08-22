#!/usr/bin/env python3
"""Enforce that the roadmap is actually derived from the evaluation.

Every gap ID defined in docs/evaluation/*.md must be either:

  * referenced by a phase in TODOs.md   -> scheduled, or
  * listed in the TODOs.md P3 table     -> a deliberate non-goal.

A gap that is in neither has silently fallen off the plan. That is exactly
what happened to L2-7 (ask_user_question): the layer doc carried a full
design, phase 23 assumed the tool existed, and no phase ever built it.

Also checks the reverse direction: a gap ID cited by TODOs.md or an
architecture doc but never defined in the evaluation cannot be traced back to
evidence. L6-5 was invented in the L6 layer doc that way.

Run: python3 scripts/check-gap-coverage.py
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GAP = re.compile(r"\bL[0-8]-\d+\b")


def ids(text: str) -> set[str]:
  return set(GAP.findall(text))


def main() -> int:
  evaluation = ROOT / "docs" / "evaluation"
  todos = ROOT / "TODOs.md"
  architecture = ROOT / "docs" / "architecture"

  # A gap is DEFINED where the evaluation gives it a table row of its own.
  defined: set[str] = set()
  for path in sorted(evaluation.glob("*.md")):
    for line in path.read_text().splitlines():
      m = re.match(r"\|\s*\**~?~?(L[0-8]-\d+)~?~?\**\s*\|", line.strip())
      if m:
        defined.add(m.group(1))

  if not defined:
    print("error: no gap IDs defined in docs/evaluation/ — is the report missing?")
    return 1

  todos_text = todos.read_text()

  # The P3 section is where deliberate non-goals live.
  p3 = todos_text.split("## ⚪ P3", 1)
  non_goals = ids(p3[1]) if len(p3) > 1 else set()
  scheduled = ids(p3[0])

  cited = ids(todos_text)
  for path in sorted(architecture.glob("*.md")):
    cited |= ids(path.read_text())

  failures = []

  orphaned = sorted(defined - scheduled - non_goals)
  if orphaned:
    failures.append(
      "缺口已在评估中定义，但既未被任何阶段承接，也未列入 P3 明确不排期：\n"
      + "".join(f"    {g}\n" for g in orphaned)
      + "  修法：排进某个阶段，或加进 TODOs.md 的 P3 表并写明理由。"
    )

  undefined = sorted(cited - defined)
  if undefined:
    failures.append(
      "缺口被 TODOs / 架构文档引用，但评估里没有定义，无法追溯到证据：\n"
      + "".join(f"    {g}\n" for g in undefined)
      + "  修法：在 docs/evaluation/ 的对应层缺口表里补一行。"
    )

  if failures:
    print("溯源校验失败：\n")
    for f in failures:
      print("  " + f + "\n")
    return 1

  print(
    f"溯源校验通过：{len(defined)} 个缺口全部有归属"
    f"（{len(defined & scheduled)} 个已排期，{len(defined & non_goals)} 个明确不排期）"
  )
  return 0


if __name__ == "__main__":
  sys.exit(main())
