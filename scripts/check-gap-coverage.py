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

And a third direction: a gap a layer doc still lists as open must be visibly
unfinished in the roadmap too. L7-1 and L7-4 stayed marked open long after the
work shipped, because nothing compared the two.

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

  # Three buckets, and every gap must land in exactly one:
  #   scheduled     -- a phase owns it
  #   known-open    -- deliberately left at a phase boundary, with a reason
  #   non-goal (P3) -- decided against
  p3 = todos_text.split("## ⚪ P3", 1)
  non_goals = ids(p3[1]) if len(p3) > 1 else set()
  before_p3 = p3[0]
  open_split = before_p3.split("## 🔵 已知仍开放", 1)
  known_open = ids(open_split[1]) if len(open_split) > 1 else set()
  scheduled = ids(open_split[0])

  cited = ids(todos_text)
  for path in sorted(architecture.glob("*.md")):
    cited |= ids(path.read_text())

  # A gap row that is NOT struck through in a layer doc claims to be open.
  # Cross-checking that against the roadmap catches the drift this script was
  # originally blind to: L7-1 and L7-4 stayed listed as open long after the
  # work shipped, because nothing compared the two.
  claimed_open: set[str] = set()
  for path in sorted(architecture.glob("*.md")):
    for line in path.read_text().splitlines():
      m = re.match(r"\|\s*(?:⚠️\s*)?(L[0-8]-\d+)\s*\|", line.strip())
      if m:
        claimed_open.add(m.group(1))

  failures = []

  # Anything a layer doc still shows as open must be visibly unfinished in the
  # roadmap too: either an unchecked item, an explicit non-goal, or flagged.
  p3_text = p3[1] if len(p3) > 1 else ""
  unchecked = set()
  for line in todos_text.splitlines():
    stripped = line.strip()
    if stripped.startswith("- [ ]") or stripped.startswith("⚠️"):
      unchecked |= ids(line)
  stale = sorted(
    g
    for g in claimed_open
    if g not in non_goals
    and g not in known_open
    and g not in unchecked
    and g not in ids(p3_text)
  )
  if stale:
    failures.append(
      "架构文档的缺口表仍把这些标为未完成，但路线图里既没有未勾选项也不在 P3：\n"
      + "".join(f"    {g}\n" for g in stale)
      + "  修法：做完了就在层文档里划掉（~~L7-1~~），没做完就在 TODOs 留一个未勾选项。"
    )

  orphaned = sorted(defined - scheduled - known_open - non_goals)
  if orphaned:
    failures.append(
      "缺口已在评估中定义，但三个桶都没有它（已排期 / 已知仍开放 / 明确不排期）：\n"
      + "".join(f"    {g}\n" for g in orphaned)
      + "  修法：排进某个阶段，或列进「已知仍开放」，或加进 P3 并写明理由。"
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
    f"（{len(defined & scheduled)} 已排期 / {len(defined & known_open)} 已知仍开放 / "
    f"{len(defined & non_goals)} 明确不排期）；层文档与路线图状态一致"
  )
  return 0


if __name__ == "__main__":
  sys.exit(main())
