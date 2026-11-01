#!/bin/sh
# Tests scripts/noctalia-colors.sh against fixtures. Never touches ~/.config:
# SRC, OUT and MODE are always overridden, and PATH hides any real `noctalia`.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SCRIPT="$ROOT/scripts/noctalia-colors.sh"
FIXTURES="$ROOT/tests/fixtures"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# A fake `noctalia` so the mode query is deterministic and offline.
mkdir -p "$WORK/bin"
cat > "$WORK/bin/noctalia" <<'FAKE'
#!/bin/sh
[ "$*" = "msg theme-mode-get" ] && cat "$FAKE_MODE_FILE"
FAKE
chmod +x "$WORK/bin/noctalia"
export FAKE_MODE_FILE="$WORK/mode"

pass=0
fail=0
check() {
  name=$1
  shift
  if "$@"; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FAIL: $name" >&2
  fi
}

run() {
  env PATH="$WORK/bin:/usr/bin:/bin" "$@" sh "$SCRIPT"
}

# Dark mode from an explicit MODE, into a directory that does not exist yet.
OUT="$WORK/out/nested/colors.json"
run SRC="$FIXTURES/noctalia.lua" OUT="$OUT" MODE=dark
check "dark output matches fixture" cmp -s "$OUT" "$FIXTURES/colors-dark.json"
check "no temp files left" test "$(ls "$WORK/out/nested")" = "colors.json"

# Light mode.
run SRC="$FIXTURES/noctalia.lua" OUT="$OUT" MODE=light
check "light output matches fixture" cmp -s "$OUT" "$FIXTURES/colors-light.json"

# Mode from `noctalia msg theme-mode-get`.
echo light > "$WORK/mode"
run SRC="$FIXTURES/noctalia.lua" OUT="$WORK/q.json" MODE=
check "mode queried from noctalia" cmp -s "$WORK/q.json" "$FIXTURES/colors-light.json"

# Unknown mode and a failing query both fall back to dark.
echo sepia > "$WORK/mode"
run SRC="$FIXTURES/noctalia.lua" OUT="$WORK/u.json" MODE=
check "unknown mode → dark" cmp -s "$WORK/u.json" "$FIXTURES/colors-dark.json"
printf '#!/bin/sh\nexit 1\n' > "$WORK/bin/noctalia"
run SRC="$FIXTURES/noctalia.lua" OUT="$WORK/n.json" MODE=
check "failing query → dark" cmp -s "$WORK/n.json" "$FIXTURES/colors-dark.json"

# Missing palette, or one without a usable primary: exit 0, write nothing.
run SRC="$WORK/missing.lua" OUT="$WORK/none.json" MODE=dark
check "missing palette writes nothing" test ! -e "$WORK/none.json"
printf 'local primary = "#123456"\n' > "$WORK/bad.lua"
run SRC="$WORK/bad.lua" OUT="$WORK/bad.json" MODE=dark
check "unparseable primary writes nothing" test ! -e "$WORK/bad.json"

# The script itself is POSIX sh (no bashisms) when a checker is available.
if command -v checkbashisms >/dev/null 2>&1; then
  check "no bashisms" checkbashisms "$SCRIPT"
fi
if command -v dash >/dev/null 2>&1; then
  env PATH="$WORK/bin:/usr/bin:/bin" SRC="$FIXTURES/noctalia.lua" OUT="$WORK/dash.json" MODE=dark \
    dash "$SCRIPT"
  check "runs under dash" cmp -s "$WORK/dash.json" "$FIXTURES/colors-dark.json"
fi

echo "colors.sh: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
