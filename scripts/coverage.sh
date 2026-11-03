#!/bin/sh
# Test coverage for the workspace, as a Markdown report.
#
#   scripts/coverage.sh
#       Runs the tests under cargo-llvm-cov, saves the raw numbers to
#       target/coverage/coverage.json and prints the report.
#
#   scripts/coverage.sh report HEAD.json [BASE.json [CHANGED]]
#       Prints the report for HEAD.json (cargo llvm-cov --json
#       --summary-only output). With BASE.json it shows the change from
#       it; CHANGED is a file listing paths (one per line) to call out as
#       the files a pull request touched. CI uses this to comment on PRs.
#
# Needs cargo-llvm-cov and jq. The report starts with a hidden marker,
# so CI can find and update its own comment.
set -eu

MARKER='<!-- tidings-coverage -->'

run() {
    out=target/coverage
    mkdir -p "$out"
    # Test output to stderr, so stdout is just the report.
    cargo llvm-cov --workspace --json --summary-only --output-path "$out/coverage.json" >&2
    report "$out/coverage.json"
}

report() {
    head_json=$1
    base_json=${2:-}
    changed=${3:-/dev/null}
    if [ -n "$base_json" ]; then
        base_arg="--slurpfile base $base_json"
    else
        base_arg="--argjson base []"
    fi
    # shellcheck disable=SC2086 # base_arg is two words on purpose
    jq -rn --arg marker "$MARKER" --slurpfile head "$head_json" $base_arg \
        --rawfile changed "$changed" "$JQ_REPORT"
}

# The report. Paths are made relative to the repository (from crates/ on),
# so a base checked out elsewhere still lines up.
JQ_REPORT='
def files($cov):
  [$cov[0].data[0].files[]
   | (.filename | sub("^.*?/(?<p>crates/)"; "\(.p)")) as $path
   | {path: $path,
      crate: ($path | capture("^crates/(?<c>[^/]+)/").c // "other"),
      lines: .summary.lines, functions: .summary.functions}];

def sum(f): {count: (map(f.count) | add // 0), covered: (map(f.covered) | add // 0)};

def pct(s): if s == null or s.count == 0 then null else s.covered * 100 / s.count end;

def fmt(p): if p == null then "–" else "\((p * 10 | round) / 10)%" end;

def delta(now; was):
  if now == null or was == null then ""
  else ((now - was) * 10 | round / 10) as $d
    | if $d > 0 then "+\($d)" elif $d < 0 then "\($d)" else "±0" end
  end;

files($head) as $h
| (if $base == [] then null else files($base) end) as $b
| ($changed | split("\n") | map(select(length > 0))) as $touched
| ($h | sum(.lines)) as $total
| (if $b == null then null else $b | sum(.lines) end) as $btotal
| def was($path): if $b == null then null
    else ($b | map(select(.path == $path)) | first) end;
  def crate_was($c): if $b == null then null
    else pct($b | map(select(.crate == $c)) | sum(.lines)) end;
  def row(cells): "| " + ((if $b == null then cells[:-1] else cells end) | join(" | ")) + " |";
[
  $marker,
  "### Coverage: \(fmt(pct($total))) of lines"
    + (if $btotal == null then ""
       else " (\(delta(pct($total); pct($btotal))) vs base)" end),
  "",
  # The Change column only when there is a base to compare with.
  row(["Crate", "Lines", "Functions", "Change"]),
  row(["---", "---:", "---:", "---:"]),
  ($h | group_by(.crate)[]
   | {crate: .[0].crate, lines: sum(.lines), functions: sum(.functions)}
   | row(["`\(.crate)`", fmt(pct(.lines)), fmt(pct(.functions)),
          delta(pct(.lines); crate_was(.crate))])),
  row(["**Total**", "**\(fmt(pct($total)))**", "**\(fmt(pct($h | sum(.functions))))**",
       delta(pct($total); pct($btotal))]),
  "",
  ([$h[] | select(.path as $p | $touched | index($p))]
   | if length == 0 then empty else
       "#### Files changed in this pull request",
       "",
       "| File | Lines | Change |",
       "|---|---:|---:|",
       (.[] | was(.path) as $w
        | "| `\(.path)` | \(fmt(pct(.lines))) (\(.lines.covered)/\(.lines.count)) | "
          + (if $b != null and $w == null then "new" else delta(pct(.lines); $w | if . == null then null else pct(.lines) end) end)
          + " |"),
       ""
     end),
  "<details><summary>All \($h | length) files, least covered first</summary>",
  "",
  "| File | Lines | Functions |",
  "|---|---:|---:|",
  ($h | sort_by(pct(.lines) // 100)[]
   | "| `\(.path)` | \(fmt(pct(.lines))) (\(.lines.covered)/\(.lines.count)) | \(fmt(pct(.functions))) |"),
  "",
  "</details>",
  "",
  "<sub>Line coverage from `cargo llvm-cov` over the workspace tests. Run `scripts/coverage.sh` for the same report locally.</sub>"
] | .[]
'

case "${1:-}" in
    "") run ;;
    report)
        shift
        report "$@"
        ;;
    *)
        echo "usage: $0 [report HEAD.json [BASE.json [CHANGED]]]" >&2
        exit 2
        ;;
esac
