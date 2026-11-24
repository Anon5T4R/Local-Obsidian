#!/usr/bin/env bash
# Regenerate every app icon file from assets/app-icon/localnotebook.svg (and its small-size
# variant, localnotebook-small.svg).
#
# Needs: resvg (brew install resvg / cargo install resvg). The outputs are committed, so building
# and packaging never need these tools.
#
#   packaging/icons.sh
#
# Two masters on purpose: 40 px and up come from the master, 16-32 px from the small variant, whose
# `#` is bolder and wider so it does not decay into "a page with lines of text" (Local Word's mark)
# at the sizes where the two icons compete. See assets/app-icon/README.md.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/app-icon"
SVG="$DIR/localnotebook.svg"
SMALL="$DIR/localnotebook-small.svg"
ID="com.localobsidian.app"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

command -v resvg >/dev/null || { echo "error: resvg not found (cargo install resvg)" >&2; exit 1; }

# Segoe Script renders the signature; where fontdb cannot find it, point resvg at the file so the
# raster is deterministic (the committed PNGs must not depend on the machine that made them).
FONT_ARGS=()
for f in /usr/share/fonts/truetype/msttcorefonts/segoesc.ttf "$HOME/Library/Fonts/segoesc.ttf"; do
  if [ -f "$f" ]; then FONT_ARGS=(--use-font-file "$f"); break; fi
done

render() {
  local src="$1" size="$2" out="$3"
  resvg -w "$size" -h "$size" "${FONT_ARGS[@]}" "$src" "$out" </dev/null
}

# The master for any size the eye reads as "the icon", the small variant below that.
source_for() {
  if [ "$1" -le 32 ]; then echo "$SMALL"; else echo "$SVG"; fi
}

render "$SVG" 1024 "$DIR/localnotebook-1024.png"

# Linux hicolor theme (also the runtime window icon on Windows and Linux: 256x256).
for s in 16 24 32 48 64 128 256 512; do
  mkdir -p "$DIR/hicolor/${s}x${s}/apps"
  render "$(source_for "$s")" "$s" "$DIR/hicolor/${s}x${s}/apps/$ID.png"
done
mkdir -p "$DIR/hicolor/scalable/apps"
cp "$SVG" "$DIR/hicolor/scalable/apps/$ID.svg"

# Windows .ico.
ICO_PNGS=()
for s in 16 20 24 32 40 48 64 128 256; do
  render "$(source_for "$s")" "$s" "$TMP/ico-$s.png"
  ICO_PNGS+=("$TMP/ico-$s.png")
done
(cd "$ROOT" && cargo run -q -p xtask -- ico "$DIR/localnotebook.ico" "${ICO_PNGS[@]}")

echo "icons written to $DIR"
