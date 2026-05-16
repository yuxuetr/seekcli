#!/bin/bash
# Describe an image via StepFun VLM. Optional second argument is the
# question to ask the VLM (defaults to a generic detailed description).
#
# Usage: vlm_describe.sh <image_path> [question]

set -e

IMG_PATH="${1:?usage: vlm_describe.sh <image_path> [question]}"
QUESTION="${2:-请用中文详细描述这张图片的内容，包括文字、布局、关键元素与可能的用途。}"

if [ -z "$STEP_API_KEY" ]; then
  echo "ERROR: STEP_API_KEY not set. Export it first:" >&2
  echo "  export STEP_API_KEY=sk-..." >&2
  exit 1
fi

if [ ! -f "$IMG_PATH" ]; then
  echo "ERROR: image not found: $IMG_PATH" >&2
  exit 1
fi

MIME=$(file --mime-type -b "$IMG_PATH" 2>/dev/null || echo "image/png")
case "$MIME" in
  image/*) ;;
  *)
    echo "ERROR: $IMG_PATH is not an image (mime=$MIME)" >&2
    exit 1 ;;
esac

# base64 the image. Use jq for safe JSON construction (avoids escaping headaches
# with the very long base64 string).
B64=$(base64 -i "$IMG_PATH")

PAYLOAD=$(jq -n \
  --arg model "step-1.5v-mini" \
  --arg prompt "$QUESTION" \
  --arg image_url "data:$MIME;base64,$B64" \
  '{
    model: $model,
    messages: [{
      role: "user",
      content: [
        {type: "text", text: $prompt},
        {type: "image_url", image_url: {url: $image_url}}
      ]
    }]
  }')

RESPONSE=$(curl -sS https://api.stepfun.com/v1/chat/completions \
  -H "Authorization: Bearer $STEP_API_KEY" \
  -H "Content-Type: application/json" \
  -d "$PAYLOAD")

# Try happy-path first; if that fails, surface the error.
CONTENT=$(echo "$RESPONSE" | jq -r '.choices[0].message.content // empty')
if [ -n "$CONTENT" ]; then
  echo "$CONTENT"
  exit 0
fi

# Error path: bubble up something useful for the model to read.
ERR_MSG=$(echo "$RESPONSE" | jq -r '.error.message // .message // "unknown error"')
echo "ERROR: StepFun VLM returned no content. Message: $ERR_MSG" >&2
echo "Raw response: $RESPONSE" >&2
exit 1
