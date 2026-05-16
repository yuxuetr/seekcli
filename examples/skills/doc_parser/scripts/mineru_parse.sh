#!/bin/bash
# Parse a document (PDF, Docx, PPTX, Xlsx, image) via MinerU V4 API.
# Returns the path to a Markdown file containing the extracted content.
#
# API docs: https://mineru.net/apiManage/docs
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

# --- Pre-check: 200MB file size limit (per MinerU docs) --------------------
FILE_BYTES=$(stat -f '%z' "$FILE_PATH" 2>/dev/null || wc -c <"$FILE_PATH")
MAX_BYTES=$((200 * 1024 * 1024))
if [ "$FILE_BYTES" -gt "$MAX_BYTES" ]; then
  echo "ERROR: file exceeds MinerU's 200MB limit ($FILE_BYTES bytes)" >&2
  echo "Hint: split the document or extract specific pages first." >&2
  exit 1
fi

BASE="https://mineru.net/api/v4"
FILENAME=$(basename "$FILE_PATH")

# --- Step 1: request upload URL --------------------------------------------
# model_version=vlm gives the highest accuracy (recommended in docs).
# is_ocr=true forces OCR even on PDFs with a text layer; preserves the
# original behavior of the deleted Rust client. For pure text PDFs this
# is somewhat wasteful; users who want speed-over-fidelity can flip to
# false in their local copy.
echo "[mineru] Requesting upload slot for $FILENAME..." >&2
REQ_PAYLOAD=$(jq -n --arg name "$FILENAME" '{
  files: [{name: $name}],
  model_version: "vlm",
  is_ocr: true,
  enable_formula: true,
  enable_table: true,
  language: "ch"
}')

RESP=$(curl -sS -X POST "$BASE/file-urls/batch" \
  -H "Authorization: Bearer $MINERU_API_KEY" \
  -H "Content-Type: application/json" \
  -d "$REQ_PAYLOAD")

CODE=$(echo "$RESP" | jq -r '.code // -1')
if [ "$CODE" != "0" ]; then
  MSG=$(echo "$RESP" | jq -r '.msg // "unknown error"')
  case "$CODE" in
    A0202)
      echo "ERROR: MinerU rejected the API key (A0202: Invalid Token)." >&2
      echo "Check MINERU_API_KEY is correct and starts with the expected prefix." >&2 ;;
    A0211)
      echo "ERROR: MinerU API key expired (A0211)." >&2
      echo "Generate a new token at https://mineru.net" >&2 ;;
    *)
      echo "ERROR: MinerU upload-slot request failed (code=$CODE): $MSG" >&2 ;;
  esac
  exit 1
fi

BATCH_ID=$(echo "$RESP" | jq -r '.data.batch_id')
UPLOAD_URL=$(echo "$RESP" | jq -r '.data.file_urls[0]')

if [ -z "$BATCH_ID" ] || [ "$BATCH_ID" = "null" ]; then
  echo "ERROR: MinerU did not return batch_id. Raw response:" >&2
  echo "$RESP" >&2
  exit 1
fi

# --- Step 2: upload file via PUT (no Content-Type) -------------------------
# CRITICAL: must use `curl -T` (upload-file mode), NOT `--data-binary @file`.
# --data-binary auto-adds Content-Type: application/x-www-form-urlencoded
# which makes MinerU's presigned S3-style URL silently reject the upload
# and the job hangs forever in state=waiting-file. -T sends raw bytes via
# PUT with NO Content-Type, matching the presigned signature's expectation.
echo "[mineru] Uploading (batch_id=$BATCH_ID, ${FILE_BYTES} bytes)..." >&2
if ! curl -sS --fail -T "$FILE_PATH" "$UPLOAD_URL" -o /tmp/seekcli_mineru_upload_err 2>/dev/null; then
  echo "ERROR: file upload to MinerU presigned URL failed" >&2
  [ -s /tmp/seekcli_mineru_upload_err ] && cat /tmp/seekcli_mineru_upload_err >&2
  rm -f /tmp/seekcli_mineru_upload_err
  exit 1
fi
rm -f /tmp/seekcli_mineru_upload_err

# --- Step 3: poll for completion -------------------------------------------
# MinerU state machine (per docs):
#   waiting-file → pending → running → converting → done
#                                              ↘ failed
echo "[mineru] Processing (poll up to 120s)..." >&2
ZIP_URL=""
ERR_MSG=""
LAST_STATE=""
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
      ERR_MSG=$(echo "$POLL" | jq -r '.data.extract_result[0].err_msg // "unknown error"')
      echo "ERROR: MinerU extraction failed: $ERR_MSG" >&2
      exit 1 ;;
    running)
      # Surface page-level progress when available.
      EXTRACTED=$(echo "$POLL" | jq -r '.data.extract_result[0].extract_progress.extracted_pages // empty')
      TOTAL=$(echo "$POLL" | jq -r '.data.extract_result[0].extract_progress.total_pages // empty')
      if [ -n "$EXTRACTED" ] && [ -n "$TOTAL" ]; then
        printf '  attempt %2d state=running (%s/%s pages)\n' "$i" "$EXTRACTED" "$TOTAL" >&2
      elif [ "$STATE" != "$LAST_STATE" ]; then
        printf '  attempt %2d state=running\n' "$i" >&2
      fi ;;
    waiting-file|pending|converting|*)
      # Only print on state transition to keep stderr quiet.
      if [ "$STATE" != "$LAST_STATE" ]; then
        printf '  attempt %2d state=%s\n' "$i" "$STATE" >&2
      fi ;;
  esac
  LAST_STATE="$STATE"
done

if [ -z "$ZIP_URL" ] || [ "$ZIP_URL" = "null" ]; then
  echo "ERROR: MinerU did not complete within 120s (last state=$LAST_STATE)" >&2
  if [ "$LAST_STATE" = "waiting-file" ]; then
    echo "Hint: state never advanced past 'waiting-file' — the upload likely" >&2
    echo "was not accepted. Check the script uses 'curl -T' not '--data-binary'." >&2
  fi
  exit 1
fi

# --- Step 4: download zip, extract full.md ---------------------------------
TMP_ZIP=$(mktemp -t seekcli_mineru_zip)
if ! curl -sS --fail -o "$TMP_ZIP" "$ZIP_URL"; then
  echo "ERROR: failed to download MinerU result zip from $ZIP_URL" >&2
  rm -f "$TMP_ZIP"
  exit 1
fi

FULL_MD_PATH=$(unzip -Z -1 "$TMP_ZIP" | grep 'full\.md$' | head -1)
if [ -z "$FULL_MD_PATH" ]; then
  echo "ERROR: no full.md found in MinerU result zip. Contents:" >&2
  unzip -Z -1 "$TMP_ZIP" >&2
  rm -f "$TMP_ZIP"
  exit 1
fi

OUTPUT_PATH="/tmp/seekcli_mineru_$(date +%s).md"
unzip -p "$TMP_ZIP" "$FULL_MD_PATH" > "$OUTPUT_PATH"
rm -f "$TMP_ZIP"

echo "[mineru] Saved: $OUTPUT_PATH" >&2

# Print just the path on stdout so the agent can pass it to read_file.
echo "$OUTPUT_PATH"
