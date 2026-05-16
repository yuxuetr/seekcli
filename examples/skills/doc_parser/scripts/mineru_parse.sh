#!/bin/bash
# Parse a document (PDF, Docx, PPTX, Xlsx, image) via MinerU V4 API.
# Returns the path to a Markdown file containing the extracted content.
#
# Usage: mineru_parse.sh <file_path>

set -e

FILE_PATH="${1:?usage: mineru_parse.sh <file_path>}"

if [ -z "$MINERU_API_KEY" ]; then
  echo "ERROR: MINERU_API_KEY not set. Export it first:" >&2
  echo "  export MINERU_API_KEY=..." >&2
  exit 1
fi
if [ ! -f "$FILE_PATH" ]; then
  echo "ERROR: file not found: $FILE_PATH" >&2
  exit 1
fi

BASE="https://mineru.net/api/v4"
FILENAME=$(basename "$FILE_PATH")

# --- Step 1: request upload URL --------------------------------------------
echo "[mineru] Requesting upload slot for $FILENAME..." >&2
REQ_PAYLOAD=$(jq -n --arg name "$FILENAME" '{
  files: [{name: $name}],
  is_ocr: true,
  model_version: "vlm"
}')

RESP=$(curl -sS -X POST "$BASE/file-urls/batch" \
  -H "Authorization: Bearer $MINERU_API_KEY" \
  -H "Content-Type: application/json" \
  -d "$REQ_PAYLOAD")

CODE=$(echo "$RESP" | jq -r '.code // -1')
if [ "$CODE" != "0" ]; then
  MSG=$(echo "$RESP" | jq -r '.msg // "unknown error"')
  echo "ERROR: MinerU upload-slot request failed (code=$CODE): $MSG" >&2
  exit 1
fi

BATCH_ID=$(echo "$RESP" | jq -r '.data.batch_id')
UPLOAD_URL=$(echo "$RESP" | jq -r '.data.file_urls[0]')

if [ -z "$BATCH_ID" ] || [ "$BATCH_ID" = "null" ]; then
  echo "ERROR: MinerU did not return batch_id" >&2
  exit 1
fi

# --- Step 2: upload file (PUT raw bytes, no Content-Type) ------------------
# CRITICAL: use `curl -T` (upload-file mode), NOT `--data-binary`.
# --data-binary auto-adds Content-Type: application/x-www-form-urlencoded
# which makes MinerU's presigned URL silently reject the upload and the
# job hangs forever in state=waiting-file. -T sends raw bytes via PUT
# with no Content-Type, matching what MinerU's presigned signature expects.
echo "[mineru] Uploading (batch_id=$BATCH_ID)..." >&2
if ! curl -sS --fail -T "$FILE_PATH" "$UPLOAD_URL" -o /tmp/seekcli_mineru_upload_err 2>/dev/null; then
  echo "ERROR: file upload to MinerU presigned URL failed" >&2
  [ -s /tmp/seekcli_mineru_upload_err ] && cat /tmp/seekcli_mineru_upload_err >&2
  rm -f /tmp/seekcli_mineru_upload_err
  exit 1
fi
rm -f /tmp/seekcli_mineru_upload_err

# --- Step 3: poll for completion -------------------------------------------
echo "[mineru] Processing (max 120s)..." >&2
ZIP_URL=""
for i in $(seq 1 60); do
  sleep 2
  POLL=$(curl -sS "$BASE/extract-results/batch/$BATCH_ID" \
    -H "Authorization: Bearer $MINERU_API_KEY")
  POLL_CODE=$(echo "$POLL" | jq -r '.code // -1')
  if [ "$POLL_CODE" != "0" ]; then
    POLL_MSG=$(echo "$POLL" | jq -r '.msg // "unknown error"')
    echo "ERROR: MinerU poll failed (code=$POLL_CODE): $POLL_MSG" >&2
    exit 1
  fi
  STATE=$(echo "$POLL" | jq -r '.data.extract_result[0].state // "pending"')
  case "$STATE" in
    done)
      ZIP_URL=$(echo "$POLL" | jq -r '.data.extract_result[0].full_zip_url')
      break ;;
    failed)
      ERR=$(echo "$POLL" | jq -r '.data.extract_result[0].err_msg // "unknown error"')
      echo "ERROR: MinerU extraction failed: $ERR" >&2
      exit 1 ;;
    *)
      printf '  attempt %d state=%s\n' "$i" "$STATE" >&2 ;;
  esac
done

if [ -z "$ZIP_URL" ] || [ "$ZIP_URL" = "null" ]; then
  echo "ERROR: MinerU did not complete within 120s" >&2
  exit 1
fi

# --- Step 4: download zip, extract full.md ---------------------------------
TMP_ZIP=$(mktemp -t seekcli_mineru_zip)
curl -sS -o "$TMP_ZIP" "$ZIP_URL"

FULL_MD_PATH=$(unzip -Z -1 "$TMP_ZIP" | grep 'full\.md$' | head -1)
if [ -z "$FULL_MD_PATH" ]; then
  echo "ERROR: no full.md found in MinerU result zip" >&2
  unzip -Z -1 "$TMP_ZIP" >&2
  rm -f "$TMP_ZIP"
  exit 1
fi

# Final output path: /tmp/seekcli_mineru_<timestamp>.md
OUTPUT_PATH="/tmp/seekcli_mineru_$(date +%s).md"
unzip -p "$TMP_ZIP" "$FULL_MD_PATH" > "$OUTPUT_PATH"
rm -f "$TMP_ZIP"

echo "[mineru] Saved: $OUTPUT_PATH" >&2

# Print just the path on stdout so the agent can pass it to read_file.
echo "$OUTPUT_PATH"
