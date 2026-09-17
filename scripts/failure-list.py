#!/usr/bin/env python3
"""Collect everything marked wrong with `/bad` into the failure list.

The roadmap's 阶段零 is "a week of real use, producing a failure list", and
every B-class phase downstream is calibrated against it. Before `/bad` existed
that list could only have come from memory, which is to say it would not have
existed.

Each entry prints the note, the prompt that led to it, and the session id — the
session replays the whole turn (`seekcli --resume <id>`) and `/trace` shows
what the loop actually did, so the note never has to carry the evidence.

Usage:  python3 scripts/failure-list.py [sessions-dir]
        python3 scripts/failure-list.py --json    # for feeding an eval suite
"""

import json
import glob
import os
import sys


def marks(sessions_dir: str):
  """Yields one dict per `/bad`, with the prompt it followed."""
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

    # The prompt in force when the mark was made — the last one before it, not
    # the session title, which is only ever the first.
    prompt = ""
    for line in lines:
      try:
        payload = json.loads(line).get("payload", {})
      except json.JSONDecodeError:
        continue
      if not isinstance(payload, dict):
        continue
      if "UserMessage" in payload:
        prompt = payload["UserMessage"].get("content") or ""
      elif "MarkedBad" in payload:
        yield {
          "session": os.path.basename(session.rstrip("/")),
          "created": meta.get("created", ""),
          "model": meta.get("model", ""),
          "prompt": prompt,
          "note": payload["MarkedBad"].get("note") or "",
        }


def main() -> int:
  as_json = "--json" in sys.argv
  args = [a for a in sys.argv[1:] if not a.startswith("--")]
  sessions_dir = args[0] if args else os.path.expanduser("~/.seekcli/sessions")
  found = list(marks(sessions_dir))

  if as_json:
    print(json.dumps(found, ensure_ascii=False, indent=2))
    return 0

  if not found:
    print(f"Nothing marked with /bad under {sessions_dir}.")
    print("Either nothing went wrong, or nothing was recorded when it did —")
    print("and those two look identical from here. Use /bad while it is fresh.")
    return 0

  print(f"{len(found)} marked failure(s)\n")
  for entry in found:
    print(f"── {entry['created'][:16]}  {entry['session'][:8]}  {entry['model']}")
    print(f"   prompt: {entry['prompt'][:100]}")
    print(f"   note:   {entry['note'] or '(none — see the session and its trace)'}")
    print(f"   replay: seekcli --resume {entry['session'][:8]}\n")
  return 0


if __name__ == "__main__":
  sys.exit(main())
