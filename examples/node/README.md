# Node project, no Rust

Replaces commitlint, `@commitlint/config-conventional`, husky and commit-and-tag-version (or
standard-version) with a single binary. Nothing Rust-related is needed: CI installs a prebuilt
`ccr-lint`, and so can developers (from the [GitHub Releases](https://github.com/Forsrobin/ccr-lint/releases),
or `cargo binstall ccr-lint`).

```
.github/workflows/commits.yml     lint the commits of every pull request
.github/workflows/release.yml     changelog, version bump, tag, build, push to main, GitHub Release
.githooks/commit-msg              lint each commit locally; "prepare" in package.json enables it on npm install
ccr-lint.toml                     lifecycle scripts that bump package.json
scripts/sync-package-version.sh   used by the postchangelog script
```

## How the version is bumped

`ccr-lint release` bumps only `Cargo.toml` / `Cargo.lock`. Without them it takes the current version
from the newest `v` tag, then writes `CHANGELOG.md`, commits it and tags as usual. Two lifecycle scripts
in `ccr-lint.toml` bring `package.json` along:

1. `postchangelog` runs `scripts/sync-package-version.sh`, which reads the new version from the first
   heading of `CHANGELOG.md` and runs `npm version <version> --no-git-tag-version`.
2. `postcommit` amends `package.json` and `package-lock.json` into the release commit, before the tag is
   created, so the tag points at a commit with the right version.

```
✔ outputting changes to CHANGELOG.md
✔ Running lifecycle script "postchangelog"
ℹ - execute command: "sh scripts/sync-package-version.sh"
✔ committing CHANGELOG.md
✔ Running lifecycle script "postcommit"
ℹ - execute command: "git commit --amend --no-edit --no-verify package.json package-lock.json"
✔ tagging release v1.5.0
```

The commit is `chore(release): 1.5.0` with `CHANGELOG.md`, `package.json` and `package-lock.json`.

## Before the first release

The version comes from tags, and with no `v` tag at all ccr-lint starts from 1.0.0. Tag the version
that is in `package.json` once:

```sh
git tag -a "v$(node -p 'require("./package.json").version')" -m "chore(release): $(node -p 'require("./package.json").version')"
git push origin --tags
```

## Notes

- Without a `package-lock.json` (pnpm, Yarn, Bun), change `postcommit` to amend only `package.json`
  (plus your own lockfile if it records the version). `npm version` still works for editing
  `package.json`; it does not need npm to be your package manager.
- `--skip-changelog` also skips the `postchangelog` script, so the version would not be synced.
- `ccr-lint release --dry-run` works locally and prints the next version and changelog. A real release
  outside GitHub Actions is stopped by the `prerelease` script.
