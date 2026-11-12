#!/bin/sh
# ccr-lint `postchangelog` script: copy the version that was just written to CHANGELOG.md into
# package.json and package-lock.json. No git tag is made and no commit is created here.
set -eu

# The first release heading: "## [1.5.0](https://...) (2026-01-31)" or "## 1.0.0 (2026-01-31)".
version=$(sed -n -E 's/^#+ \[?([0-9]+\.[0-9]+\.[0-9]+[^] (]*).*/\1/p' CHANGELOG.md | head -n 1)
if [ -z "$version" ]; then
  echo "sync-package-version: no version heading in CHANGELOG.md" >&2
  exit 1
fi

npm version "$version" --no-git-tag-version --allow-same-version >/dev/null
