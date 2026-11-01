#!/bin/sh
# Generate tidings' colors.json from Noctalia's palette.
#
# Noctalia rewrites ~/.config/hypr/noctalia.lua on every palette change; this
# reads its accent colours from there and the light/dark mode from
# `noctalia msg theme-mode-get`. Run it from a Noctalia hook (colours_changed /
# theme-mode change); the daemon picks the file up within a second.
#
# Surfaces are fixed per mode rather than taken from the palette, mirroring
# ~/.config/noctalia/hooks/swaync-colors.sh: a wallpaper-derived surface can
# make the glass illegible. The error colour is also fixed per mode, because
# on warm wallpapers Noctalia's error and primary collapse to the same hue.
#
# Environment overrides (used by tests/colors.sh):
#   SRC   palette file   (default ~/.config/hypr/noctalia.lua)
#   OUT   output file    (default $XDG_CONFIG_HOME/tidings/colors.json)
#   MODE  dark | light   (default: ask noctalia, falling back to dark)
set -eu

SRC="${SRC:-$HOME/.config/hypr/noctalia.lua}"
OUT="${OUT:-${XDG_CONFIG_HOME:-$HOME/.config}/tidings/colors.json}"

# Palette not generated yet: nothing to do (tidings uses its defaults).
[ -f "$SRC" ] || exit 0

# Prints the 6-digit hex of `local NAME = "rgb(xxxxxx)"`, or nothing.
lua_color() {
  value=$(grep -E "^local $1 = " "$SRC" | head -n 1 |
    sed -n -E 's/.*rgb\(([0-9a-fA-F]{6})\).*/\1/p')
  printf '%s' "$value" | tr 'A-F' 'a-f'
}

PRIMARY=$(lua_color primary)
[ -n "$PRIMARY" ] || exit 0

if [ -z "${MODE:-}" ]; then
  MODE=$(noctalia msg theme-mode-get 2>/dev/null || echo dark)
fi
case "$MODE" in
  light) ;;
  *) MODE=dark ;;
esac

if [ "$MODE" = light ]; then
  SURFACE="#f9f9ff"
  SURFACE_VARIANT="#eef1f9"
  ON_SURFACE="#171a21"
  ON_SURFACE_VARIANT="#55607a"
  ON_PRIMARY="#ffffff"
  ERROR="#c0261f"
  OUTLINE="#485f85"
else
  SURFACE="#18181d"
  SURFACE_VARIANT="#22232a"
  ON_SURFACE="#e4e1e7"
  ON_SURFACE_VARIANT="#a3abbd"
  ON_PRIMARY="#0b0e13"
  ERROR="#d3453b"
  OUTLINE="#a0b2d2"
fi
ON_ERROR="#ffffff"
# Panel glass opacity, matched to the approved mockup (0.62 over a
# full-brightness, saturated blur). Must stay above the tidings-panel layer
# rule's ignore_alpha (0.55), or nothing is blurred.
SURFACE_ALPHA="0.62"

mkdir -p "$(dirname "$OUT")"
# Write then rename, so the daemon never reads a half-written file.
TMP="$OUT.tmp.$$"
trap 'rm -f "$TMP"' EXIT
cat > "$TMP" <<JSON
{
  "mode": "$MODE",
  "surface": "$SURFACE",
  "surfaceVariant": "$SURFACE_VARIANT",
  "onSurface": "$ON_SURFACE",
  "onSurfaceVariant": "$ON_SURFACE_VARIANT",
  "primary": "#$PRIMARY",
  "onPrimary": "$ON_PRIMARY",
  "error": "$ERROR",
  "onError": "$ON_ERROR",
  "outline": "$OUTLINE",
  "surfaceAlpha": $SURFACE_ALPHA
}
JSON
mv "$TMP" "$OUT"
trap - EXIT
