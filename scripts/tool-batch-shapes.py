#!/usr/bin/env python3
"""Measure what shapes of multi-tool turns the model actually emits.

The agent loop runs a turn's tool calls concurrently only when *every* call
qualifies (all read-only, or all fan-out-safe delegations); otherwise the whole
turn is sequential. A batch that mixes them loses the concurrency its safe
calls could have had. Whether that costs anything is an empirical question.

**The gate is on time saved, not on opportunities counted.** Measured
2026-09-17: two concurrent `grep`s take 19ms for the whole execute span, in a
turn whose model call alone took 999ms. Grouping local file tools would save
single-digit milliseconds against a ~1s turn — real, and worth nothing. What
would cost seconds is two *delegations* or two *network* reads stuck behind an
unrelated call in the same batch, so those are what this fails on.

An earlier version of this script counted any adjacent read-only pair and did
not include `invoke_agent` at all — so it fired on `[grep, grep, run_shell]`
(worth ~10ms) while being structurally unable to see `[invoke_agent,
invoke_agent, run_shell]` (worth seconds). It measured the wrong thing in both
directions.

Usage:  python3 scripts/tool-batch-shapes.py [sessions-dir]
Exit 1 if any mixed batch has 2+ adjacent SLOW parallel-safe calls.
"""

import collections
import glob
import json
import os
import sys

# Local reads: concurrency-safe, but each costs milliseconds. Mirrors
# tools::registry::is_parallel_readonly minus the network pair below.
FAST_SAFE = {
  "read_file", "read_image", "list_dir", "glob", "grep",
  "job_list", "job_output", "harness_inspect",
}
# Safe *and* slow enough that running two at once is worth real time.
SLOW_SAFE = {"web_search", "web_fetch"}
# Only templates marked `parallel_safe` in subagents::registry fan out.
PARALLEL_SAFE_AGENTS = {"explore"}
# Sessions created on or before the day peripheral sensors were stripped ran a
# materially different tool surface (mineru PDF extraction, a `read_file` with
# an `offset` argument, a 50KB truncation that no longer exists). Counting them
# answers a question about code that is gone.
STRIP_DAY = "2026-05-16"


def safe_kind(call: dict) -> str:
  """"slow" if this call is both concurrency-safe and worth parallelising,
  "fast" if safe but negligible, "" if it forces the turn sequential."""
  name = call["function"]["name"]
  if name in SLOW_SAFE:
    return "slow"
  if name in FAST_SAFE:
    return "fast"
  if name == "invoke_agent":
    try:
      kind = json.loads(call["function"].get("arguments") or "{}").get("subagent_type")
    except json.JSONDecodeError:
      return ""
    return "slow" if kind in PARALLEL_SAFE_AGENTS else ""
  return ""


def longest_slow_run(kinds: list[str]) -> int:
  """Longest consecutive stretch a grouping scheme could run together that
  would actually save time. A `fast` call does not break the stretch — it just
  does not justify one by itself."""
  best = current = 0
  for kind in kinds:
    if kind == "slow":
      current += 1
      best = max(best, current)
    elif kind != "fast":
      current = 0
  return best


def classify(names: list[str], kinds: list[str]) -> str:
  if all(n == "invoke_agent" for n in names):
    return "all-delegation (already concurrent)"
  if all(k in ("fast", "slow") for k in kinds):
    return "all-readonly (already concurrent)"
  if any(kinds):
    return "mixed (sequential today)"
  return "no parallel candidate"


def batches(sessions_dir: str):
  """Yields (day, title, calls) for every multi-call turn after STRIP_DAY."""
  for session in sorted(glob.glob(os.path.join(sessions_dir, "*/"))):
    meta_path, events_path = (os.path.join(session, f)
                              for f in ("meta.json", "events.jsonl"))
    if not (os.path.exists(meta_path) and os.path.exists(events_path)):
      continue
    try:
      meta = json.load(open(meta_path, encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
      continue
    day = str(meta.get("created", ""))[:10]
    if not day or day <= STRIP_DAY:
      continue
    try:
      lines = open(events_path, encoding="utf-8").read().splitlines()
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
        yield day, str(meta.get("title", ""))[:44], calls


def main() -> int:
  default = os.path.expanduser("~/.seekcli/sessions")
  sessions_dir = sys.argv[1] if len(sys.argv) > 1 else default
  kinds_seen: collections.Counter = collections.Counter()
  per_day: collections.Counter = collections.Counter()
  costly: list[tuple[str, str, list[str]]] = []
  total = 0

  for day, title, calls in batches(sessions_dir):
    total += 1
    per_day[day] += 1
    names = [c["function"]["name"] for c in calls]
    kinds = [safe_kind(c) for c in calls]
    kind = classify(names, kinds)
    kinds_seen[kind] += 1
    if kind.startswith("mixed") and longest_slow_run(kinds) >= 2:
      costly.append((day, title, names))

  print(f"{total} multi-call turn(s) under {sessions_dir}, created after {STRIP_DAY}\n")
  for kind, count in kinds_seen.most_common():
    print(f"  {count:4}  {kind}")
  # Printed because a day that dwarfs the others is usually a testing session,
  # not usage — and a verdict drawn from it says little about real behaviour.
  print("\n  by day:", ", ".join(f"{d}={n}" for d, n in sorted(per_day.items())))

  if not costly:
    print("\nNo mixed batch stranded 2+ adjacent slow parallel-safe calls.")
    print("Grouping would save milliseconds; the decision not to build it holds.")
    return 0

  print(f"\n{len(costly)} mixed batch(es) stranded concurrency worth real time:")
  for day, title, names in costly:
    print(f"  {day}  «{title}»  {names}")
  print("\nThe reassessment condition has been met — revisit 混合批次分组.")
  return 1


if __name__ == "__main__":
  sys.exit(main())
