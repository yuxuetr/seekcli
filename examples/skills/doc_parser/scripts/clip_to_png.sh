#!/bin/bash
# Save macOS clipboard image (PNG) to /tmp/seekcli_clip.png and print the path.
# Returns non-zero exit if clipboard does not contain an image.

set -e

OUT="/tmp/seekcli_clip.png"

# osascript writes raw PNG bytes from the «class PNGf» pasteboard type.
RESULT=$(osascript <<APPLESCRIPT 2>&1
try
  set theData to the clipboard as «class PNGf»
  set theFile to (POSIX file "$OUT")
  set theOpenFile to open for access theFile with write permission
  set eof theOpenFile to 0
  write theData to theOpenFile
  close access theOpenFile
  return "ok"
on error errMsg
  return "ERROR:" & errMsg
end try
APPLESCRIPT
)

if [[ "$RESULT" == ERROR:* ]]; then
  echo "Clipboard does not contain an image: ${RESULT#ERROR:}" >&2
  exit 1
fi

# Sanity check the file was actually written and isn't empty.
if [ ! -s "$OUT" ]; then
  echo "Clipboard image was empty after write" >&2
  exit 1
fi

echo "$OUT"
