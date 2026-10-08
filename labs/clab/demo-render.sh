#!/usr/bin/env bash
# GitHub READMEs cannot run asciinema's JavaScript player.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CAST="${1:-$SCRIPT_DIR/demo.cast}"
OUTPUT="${2:-${CAST%.cast}}"
AGG="${AGG:-agg}"

for tool in "$AGG" ffmpeg python3; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "$tool not found; see labs/clab/README.md for rendering requirements." >&2
    exit 2
  fi
done

TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

# tmux's final detach clears the terminal; retain the visible scan results.
python3 - "$CAST" "$TEMP_DIR/demo.cast" <<'PY'
import json
import sys

with open(sys.argv[1]) as source, open(sys.argv[2], "w") as destination:
    destination.write(next(source))
    for line in source:
        event = json.loads(line)
        if event[1] == "o" and "\x1b[?1049l" in event[2]:
            break
        destination.write(line)
PY

"$AGG" --quiet --font-family 'DejaVu Sans Mono' --font-size 14 \
  --line-height 1.25 --theme asciinema --fps-cap 10 \
  --idle-time-limit 5 --last-frame-duration 3 \
  "$TEMP_DIR/demo.cast" "$TEMP_DIR/demo.gif"

ffmpeg -v error -y -i "$TEMP_DIR/demo.gif" \
  -vf 'pad=ceil(iw/2)*2:ceil(ih/2)*2' -r 20 \
  -c:v libx264 -crf 18 -pix_fmt yuv420p -movflags +faststart \
  "$TEMP_DIR/demo.mp4"

mv "$TEMP_DIR/demo.gif" "$OUTPUT.gif"
mv "$TEMP_DIR/demo.mp4" "$OUTPUT.mp4"
echo "Saved animation: $OUTPUT.gif"
echo "Saved video: $OUTPUT.mp4"
