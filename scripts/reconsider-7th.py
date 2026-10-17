#!/usr/bin/env python3
"""The seventh assessment's reconsideration conditions, as one exit code.

`docs/evaluation/2026-10-17-claude-code-comparison.md` §3 parks four features
and says when each should come back. Written as prose they would never be
checked; this is the check.

**Only data produced after the assessment counts.** Everything before it was
read by hand when the assessment was written, and almost all of it is the
author's own experiments: both `Compaction` events on record come from one
2026-09-16 session titled `cat big1.txt 用 run_shell，原样输出`, and all four
`Interrupted` events from `sleep 40..52` probes the same afternoon. Counting
them would turn §3.4 red on day one — answering "did I test compaction",
not "does compaction happen in real use". The audit log is worse: until
2026-10-17 every `cargo test` appended to it (seventh assessment §4).

Runs in temp directories, or inside SeekCLI's own data directory, are reported
but not counted: the first is where test scratch dirs and agent-driven
experiments live, the second is `--bench`'s testbeds. Without the second
exclusion §3.1 went red on 12 benchmark edits in `~/.seekcli/bench/*` — and
would again after every benchmark run.

Usage:  python3 scripts/reconsider-7th.py [--home ~/.seekcli] [--since ISO]
Exit 1 if any §3 condition is met, 0 if none.
"""

import argparse
import glob
import json
import os
import sys
import tempfile
from datetime import datetime, timezone

ASSESSED = "2026-10-17T00:00:00+00:00"
RULE_FILES = ("AGENTS.md", "CLAUDE.md")  # agent::prompt::WORKSPACE_RULE_FILES
STEER_WINDOW_S = 60
STEER_THRESHOLD = 3


def ts(text: str) -> datetime:
  return datetime.fromisoformat(text.replace("Z", "+00:00"))


def is_temp(path: str, home: str) -> bool:
  """Not the user's own work: temp dirs, or SeekCLI's data dir (bench testbeds)."""
  roots = {os.path.realpath(tempfile.gettempdir()), "/tmp", "/private/tmp", "/var/folders",
           "/private/var/folders", os.path.realpath(home)}
  real = os.path.realpath(path)
  return any(real == r or real.startswith(r + os.sep) for r in roots)


def ancestors(path: str, home: str):
  """cwd's parents up to and including the repo root (`.git`) or $HOME."""
  cur = os.path.realpath(path)
  while True:
    parent = os.path.dirname(cur)
    if parent == cur or cur == home:
      return
    if os.path.exists(os.path.join(cur, ".git")):
      return
    cur = parent
    yield cur


def in_git(path: str) -> bool:
  cur = os.path.realpath(path)
  while True:
    if os.path.exists(os.path.join(cur, ".git")):
      return True
    parent = os.path.dirname(cur)
    if parent == cur:
      return False
    cur = parent


def session_events(home: str, since: datetime):
  for f in sorted(glob.glob(os.path.join(home, "sessions", "*", "events.jsonl"))):
    events = []
    for line in open(f, encoding="utf-8", errors="replace"):
      try:
        e = json.loads(line)
        when = ts(e["ts"])
      except (ValueError, KeyError):
        continue  # a torn last line from a crash is not evidence of anything
      if when >= since:
        p = e["payload"]
        events.append((when, next(iter(p)) if isinstance(p, dict) else p))
    yield f, events


def checkpoint(home: str, since: datetime):
  """§3.1: an edit that ran where nothing (no git) can undo it."""
  hits, temp = [], 0
  path = os.path.join(home, "audit.jsonl")
  if os.path.exists(path):
    for line in open(path, encoding="utf-8", errors="replace"):
      try:
        e = json.loads(line)
        when = ts(e["ts"])
      except (ValueError, KeyError):
        continue  # pre-fix interleaved lines (seventh assessment §4)
      if when < since or e.get("tool") not in ("write_file", "edit_file"):
        continue
      if e.get("outcome") != "executed":
        continue
      cwd = e.get("cwd", "")
      if is_temp(cwd, home):
        temp += 1
      elif cwd and not in_git(cwd):
        hits.append(cwd)
  return len(hits) >= 1, f"{len(hits)} edit(s) outside git ({temp} in temp / data dirs ignored)", hits


def steer(home: str, since: datetime):
  """§3.2: Ctrl-C followed within a minute by a new prompt — the user
  stopping the agent to say something. "Same task" is not computable from the
  log, so this over-counts; the threshold is what absorbs it."""
  n = 0
  for _, events in session_events(home, since):
    for i, (when, kind) in enumerate(events):
      if kind != "Interrupted":
        continue
      nxt = next(((w, k) for w, k in events[i + 1:] if k == "UserMessage"), None)
      if nxt and (nxt[0] - when).total_seconds() <= STEER_WINDOW_S:
        n += 1
  return n >= STEER_THRESHOLD, f"{n} interrupt→prompt within {STEER_WINDOW_S}s (need {STEER_THRESHOLD})", []


def upward_rules(home: str, since: datetime):
  """§3.3: started in a subdirectory, so the rules file above was missed."""
  user_home = os.path.dirname(os.path.realpath(home))
  hits, temp = [], 0
  for f in glob.glob(os.path.join(home, "traces", "*.json")):
    if datetime.fromtimestamp(os.path.getmtime(f), timezone.utc) < since:
      continue
    try:
      ws = json.load(open(f, encoding="utf-8")).get("workspace") or ""
    except ValueError:
      continue
    if not ws or not os.path.isdir(ws):
      continue
    if is_temp(ws, home):
      temp += 1
      continue
    if any(os.path.exists(os.path.join(ws, r)) for r in RULE_FILES):
      continue
    if any(os.path.exists(os.path.join(a, r)) for a in ancestors(ws, user_home) for r in RULE_FILES):
      hits.append(ws)
  return len(hits) >= 1, f"{len(hits)} run(s) below an unread rules file ({temp} in temp / data dirs ignored)", hits


def context_view(home: str, since: datetime):
  """§3.4: compaction happened in real use, so "what fills the context" is a
  question a user can now have."""
  n = sum(k == "Compaction" for _, events in session_events(home, since) for _, k in events)
  return n >= 1, f"{n} compaction(s)", []


def main() -> int:
  ap = argparse.ArgumentParser()
  ap.add_argument("--home", default=os.path.expanduser("~/.seekcli"))
  ap.add_argument("--since", default=ASSESSED)
  args = ap.parse_args()
  since = ts(args.since)
  red = False
  print(f"seventh assessment §3, data since {since.date()} from {args.home}")
  for label, check in (("3.1 checkpoint", checkpoint), ("3.2 steer", steer),
                       ("3.3 upward AGENTS.md", upward_rules), ("3.4 /context", context_view)):
    met, summary, examples = check(args.home, since)
    red |= met
    print(f"  {'RECONSIDER' if met else 'not yet   '}  {label:<22} {summary}")
    for ex in examples[:3]:
      print(f"              e.g. {ex}")
  return 1 if red else 0


if __name__ == "__main__":
  sys.exit(main())
