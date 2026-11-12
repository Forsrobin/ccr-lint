#!/usr/bin/env bash
# Differential test: lint every message of a NUL-separated corpus with the real commitlint CLI
# (`commitlint --edit <file>`, the commit-msg hook path) and with ccr-lint, and diff stdout + exit code.
#
#   NODE_PROJECT=/path/to/project/with/node_modules scripts/diff-lint.sh tests/corpus/lint-cases.txt
#
# NODE_PROJECT must contain node_modules with @commitlint/cli and @commitlint/config-conventional and a
# commitlint config extending config-conventional.
set -euo pipefail
corpus=$(realpath "$1")
here=$(cd "$(dirname "$0")/.." && pwd)
bin=${CCR_LINT:-$here/target/release/ccr-lint}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
python3 - "$corpus" "$work" <<'PY'
import sys
data = open(sys.argv[1], 'rb').read().split(b'\0')
for i, m in enumerate(data):
    open(f"{sys.argv[2]}/{i:05}.msg", 'wb').write(m)
PY
run_one() {
  f=$1
  (cd "$NODE_PROJECT" && NO_COLOR=1 npx --no-install commitlint --edit "$f" >"$f.node" 2>&1; echo "exit $?" >>"$f.node")
  (cd "$NODE_PROJECT" && NO_COLOR=1 "$bin" lint --edit "$f" >"$f.rust" 2>&1; echo "exit $?" >>"$f.rust")
}
export -f run_one; export NODE_PROJECT bin
ls "$work"/*.msg | xargs -P 8 -I{} bash -c 'run_one {}'
fail=0
for f in "$work"/*.msg; do
  if ! diff -q "$f.node" "$f.rust" >/dev/null; then
    fail=$((fail + 1)); echo "=== MISMATCH: $(head -c 80 "$f" | tr '\n' '|')"; diff "$f.node" "$f.rust" | head -20
  fi
done
total=$(ls "$work"/*.msg | wc -l)
echo "$((total - fail))/$total identical"
[ "$fail" -eq 0 ]
