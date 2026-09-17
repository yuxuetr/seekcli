#!/usr/bin/env python3
"""Measure what shapes of multi-tool turns the model actually emits.

The agent loop runs a turn's tool calls concurrently only when *every* call
qualifies (all read-only, or all fan-out-safe delegations); otherwise the whole
turn is sequential. A batch that mixes them therefore loses the concurrency its
safe calls could have had.

Whether that costs anything is an empirical question, and this answers it. The
grouping fix only pays when a mixed batch contains **two or more adjacent**
parallel-safe calls: one alone has nobody to run beside.

Measured 2026-09-17 over 114 sessions / 39 multi-call turns: every mixed batch
had a longest safe run of 1, so grouping would have saved zero. See the "不做"
entry for 混合批次分组 in TODOs.md. Re-run this before revisiting that decision.

Usage:  python3 scripts/tool-batch-shapes.py [sessions-dir]
Exit 1 if any mixed batch has a safe run of 2+ — i.e. the decision is stale.
"""

import collections
import glob
import json
import os
import sys

# Mirrors tools::registry::is_parallel_readonly.
READ_ONLY = {
  "read_file", "read_image", "list_dir", "glob", "grep",
  "job_list", "job_output", "harness_inspect", "web_search", "web_fetch",
}


def longest_safe_run(names: list[str]) -> int:
  """Longest consecutive stretch of parallel-safe calls, which is what a
  grouping scheme could actually run together."""
  best = current = 0
  for name in names:
    current = current + 1 if name in READ_ONLY else 0
    best = max(best, current)
  return best


def classify(names: list[str]) -> str:
  if all(n == "invoke_agent" for n in names):
    return "all-delegation (already concurrent)"
  if all(n in READ_ONLY for n in names):
    return "all-readonly (already concurrent)"
  if any(n in READ_ONLY or n == "invoke_agent" for n in names):
    return "mixed (sequential today)"
  return "no parallel candidate"


def batches(sessions_dir: str):
  for path in glob.glob(os.path.join(sessions_dir, "*", "events.jsonl")):
    try:
      lines = open(path, encoding="utf-8").read().splitlines()
    except OSError:
      continue
    for line in lines:
      try:
        payload = json.loads(line).get("payload", {})
      except json.JSONDecodeError:
        continue
      if not isinstance(payload, dict) or "AssistantMessage" not in payload:
        continue
      calls = payload["AssistantMessage"].get("tool_calls") or []
      if len(calls) > 1:
        yield [c["function"]["name"] for c in calls]


def main() -> int:
  default = os.path.expanduser("~/.seekcli/sessions")
  sessions_dir = sys.argv[1] if len(sys.argv) > 1 else default
  kinds: collections.Counter = collections.Counter()
  wasted: list[list[str]] = []
  total = 0

  for names in batches(sessions_dir):
    total += 1
    kind = classify(names)
    kinds[kind] += 1
    if kind.startswith("mixed") and longest_safe_run(names) >= 2:
      wasted.append(names)

  print(f"{total} multi-call turn(s) under {sessions_dir}\n")
  for kind, count in kinds.most_common():
    print(f"  {count:4}  {kind}")

  if not wasted:
    print("\nNo mixed batch had 2+ adjacent parallel-safe calls.")
    print("Grouping them would save nothing; the decision not to build it holds.")
    return 0

  print(f"\n{len(wasted)} mixed batch(es) COULD have run part of the turn concurrently:")
  for names in wasted:
    print(f"  longest safe run={longest_safe_run(names)}  {names}")
  print("\nThe reassessment condition has been met — revisit 混合批次分组.")
  return 1


if __name__ == "__main__":
  sys.exit(main())
