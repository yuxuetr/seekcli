#!/usr/bin/env python3
"""Report how sub-agent delegations end, from the session event log.

`ChildRun` records template, status, iterations and duration for every
delegation. Two questions this answers that nothing else can:

  * **Is a template's `max_iter` set right?** A run that ends in
    `max_iterations` spent its whole budget and was still working. A few are
    normal; a majority means the cap is wrong, not that the work was hard.
  * **Is delegation being used at all?** Its value is multi-step autonomy and
    concurrency, and neither shows up anywhere else in the log.

It deliberately does **not** try to judge whether a delegation *should* have
happened — there is no honest proxy for that yet, and a made-up one would read
as evidence. See the 委派质量反馈闭环 entry in TODOs.md.

Sessions from on or before the day peripheral sensors were stripped ran a
different tool surface and are excluded, for the same reason as in
`tool-batch-shapes.py`.

Usage:  python3 scripts/delegation-health.py [sessions-dir]
Exit 1 if more than CAP_SHARE of delegations ended at their iteration cap.
"""

import collections
import glob
import json
import os
import sys

STRIP_DAY = "2026-05-16"
# Above this share of `max_iterations` endings, the cap is the problem rather
# than the task. A third is generous: hitting the ceiling should be the
# exception that the salvage pass exists to make survivable, not the norm.
CAP_SHARE = 1 / 3


def child_runs(sessions_dir: str):
  """Yields (day, template, status, iterations, duration_ms)."""
  for session in sorted(glob.glob(os.path.join(sessions_dir, "*/"))):
    meta_path = os.path.join(session, "meta.json")
    events_path = os.path.join(session, "events.jsonl")
    if not (os.path.exists(meta_path) and os.path.exists(events_path)):
      continue
    try:
      meta = json.load(open(meta_path, encoding="utf-8"))
      lines = open(events_path, encoding="utf-8").read().splitlines()
    except (OSError, json.JSONDecodeError):
      continue
    day = str(meta.get("created", ""))[:10]
    if not day or day <= STRIP_DAY:
      continue
    for line in lines:
      try:
        payload = json.loads(line).get("payload", {})
      except json.JSONDecodeError:
        continue
      if isinstance(payload, dict) and "ChildRun" in payload:
        run = payload["ChildRun"]
        yield (day, run["template"], run["status"], run["iterations"],
               run["duration_ms"])


def main() -> int:
  default = os.path.expanduser("~/.seekcli/sessions")
  sessions_dir = sys.argv[1] if len(sys.argv) > 1 else default
  runs = list(child_runs(sessions_dir))

  if not runs:
    print(f"No delegations recorded under {sessions_dir} after {STRIP_DAY}.")
    print("Nothing to judge — that is a finding too, not a pass.")
    return 0

  by_status: collections.Counter = collections.Counter(r[2] for r in runs)
  by_day: collections.Counter = collections.Counter(r[0] for r in runs)
  print(f"{len(runs)} delegation(s) after {STRIP_DAY}\n")
  for status, count in by_status.most_common():
    print(f"  {count:4}  {status}")
  # A day that dwarfs the others is usually testing, not usage.
  print("\n  by day:", ", ".join(f"{d}={n}" for d, n in sorted(by_day.items())))

  for template in sorted({r[1] for r in runs}):
    rows = [r for r in runs if r[1] == template]
    iters = sorted(r[3] for r in rows)
    capped = sum(1 for r in rows if r[2] == "max_iterations")
    median = iters[len(iters) // 2]
    print(f"\n  {template}: {len(rows)} run(s), median {median} iteration(s), "
          f"{capped} at the cap")

  capped = by_status["max_iterations"]
  share = capped / len(runs)
  if share <= CAP_SHARE:
    print(f"\n{capped}/{len(runs)} ended at the iteration cap "
          f"({share:.0%} <= {CAP_SHARE:.0%}). Caps look adequate.")
    return 0

  print(f"\n{capped}/{len(runs)} ended at the iteration cap "
        f"({share:.0%} > {CAP_SHARE:.0%}).")
  print("A cap this often reached is a cap set too low — revisit max_iter.")
  return 1


if __name__ == "__main__":
  sys.exit(main())
