# ccr-lint

<p align="center">
  Conventional Commits linting and releases for Rust projects, without Node.
</p>

<p align="center">
  <a href="#license"><img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue"></a>
  <img alt="Rust 1.88+" src="https://img.shields.io/badge/rust-1.88%2B-orange">
</p>

`ccr-lint` is a faithful port of two Node tools, so a Rust repository can drop `package.json` and
`node_modules` and keep exactly the same behavior:

| Node tool | Version ported | `ccr-lint` command |
| --- | --- | --- |
| [commitlint](https://commitlint.js.org) with `@commitlint/config-conventional` | 21.2.4 | `ccr-lint lint` |
| [commit-and-tag-version](https://github.com/absolute-version/commit-and-tag-version) with the `conventionalcommits` preset | 13.3.0 | `ccr-lint release` |

"Faithful" means the same pass/fail decisions, rule order, messages, output format, colors and exit codes
for linting, and the same version bump, `CHANGELOG.md` text, release commit, annotated tag and console
output for releases. Versions live in `Cargo.toml` and `Cargo.lock` instead of `package.json`.

## Install

```sh
cargo binstall ccr-lint          # prebuilt binary (Linux, macOS, Windows), see cargo-binstall
cargo install ccr-lint --locked  # or build from source
```

Prebuilt archives are also attached to each [GitHub Release](https://github.com/Forsrobin/ccr-lint/releases).
In GitHub Actions, `taiki-e/install-action` installs them in seconds:

```yaml
- uses: taiki-e/install-action@v2
  with:
    tool: ccr-lint
```

## Quick start

1. Lint each commit message locally with a `commit-msg` hook (see [Linting commit messages](#linting-commit-messages)).
2. Lint pull requests in CI:

   ```yaml
   # .github/workflows/commits.yml
   on: pull_request
   jobs:
     commitlint:
       runs-on: ubuntu-latest
       steps:
         - uses: actions/checkout@v4
           with:
             fetch-depth: 0
         - uses: taiki-e/install-action@v2
           with:
             tool: ccr-lint
         - run: ccr-lint lint --from ${{ github.event.pull_request.base.sha }} --to ${{ github.event.pull_request.head.sha }} --verbose
   ```

3. Cut releases with `ccr-lint release` (locally with `--dry-run` to preview, or from a release workflow).

## Linting commit messages

```sh
ccr-lint lint --edit .git/COMMIT_EDITMSG   # commit-msg hook (strips comments using core.commentChar)
ccr-lint lint --from origin/main --to HEAD # CI: every commit of a pull request
ccr-lint lint --last                       # the last commit
echo "feat: add x" | ccr-lint lint         # stdin
```

Flags match commitlint's: `--verbose`, `--quiet`, `--strict` (exit 2 for warnings, 3 for errors),
`--help-url`, `--env VAR`, `--from-last-tag`, `--cwd`, `--color` / `--no-color`.

A `commit-msg` hook, without husky:

```sh
#!/bin/sh
exec ccr-lint lint --edit "$1"
```

Output, for `Feat: Add thing.`:

```
⧗   --- input ---
Feat: Add thing.
✖   subject must not be sentence-case [subject-case]
✖   subject may not end with full stop [subject-full-stop]
✖   type must be lower-case [type-case]
✖   type must be one of [build, chore, ci, docs, feat, fix, perf, refactor, revert, style, test] [type-enum]

✖   found 4 problems, 0 warnings
ⓘ   Get help: https://github.com/conventional-changelog/commitlint/#what-is-commitlint
```

## Releasing

```sh
ccr-lint release --dry-run   # preview the next version and changelog section
ccr-lint release             # bump, write CHANGELOG.md, commit `chore(release): X.Y.Z`, tag vX.Y.Z
```

```
✔ bumping version in Cargo.toml from 2.4.0 to 2.5.0
✔ bumping version in Cargo.lock from 2.4.0 to 2.5.0
✔ outputting changes to CHANGELOG.md
✔ committing Cargo.lock and Cargo.toml and CHANGELOG.md
✔ tagging release v2.5.0
ℹ Run `git push --follow-tags origin main` to publish
```

The bump follows commit-and-tag-version: any breaking change (`!` or a `BREAKING CHANGE:` footer) is
major, a `feat` is minor, anything else is a patch, and before 1.0.0 everything moves down one level.
The commits are those since the last `v`-prefixed semver tag on the current branch.

Version files:

- `Cargo.toml`: the `version` of `[workspace.package]`, or of `[package]` for a single crate. Only that
  line is rewritten; formatting and comments are kept.
- `Cargo.lock` (when present and not git-ignored): the entries of every workspace member whose manifest
  says `version.workspace = true` (found with `cargo metadata --no-deps`), or of the single package.

Flags: `--dry-run`, `--release-as major|minor|patch|X.Y.Z`, `--first-release`, `--skip-bump`,
`--skip-changelog`, `--skip-commit`, `--skip-tag`, `--tag-prefix`, `--no-verify`, `--sign`, `--silent`.

The changelog links commits, issues, users and the compare view to the repository from `package.json`
`repository` if there is one, else `git config remote.origin.url` (GitHub, GitLab, Bitbucket and other
hosts, exactly as conventional-changelog parses them).

## Configuration

With no configuration, `ccr-lint` behaves like commitlint with `@commitlint/config-conventional` and like
commit-and-tag-version with these changelog sections: `feat` → Features, `fix` → Bug Fixes,
`perf` → Performance, `revert` → Reverts, and `refactor`, `docs`, `style`, `test`, `build`, `ci`, `chore`
hidden.

Overrides go in `ccr-lint.toml` at the repository root, or in `[workspace.metadata.ccr-lint]` /
`[package.metadata.ccr-lint]` of the root `Cargo.toml` (same keys), or in a file given with `--config`:

```toml
[lint]
help-url = "https://example.com/commit-guidelines"
default-ignores = true          # merges, reverts, fixup!/squash!, version bumps, ...
ignores = ["^WIP"]              # extra regexes; a matching message is not linted

[lint.rules]                    # commitlint rules: [level, "always" | "never", value]
header-max-length = [2, "always", 72]
scope-enum = [2, "always", ["api", "cli", "deps"]]
body-leading-blank = [0]        # level 0 turns a rule off

[release]
tag-prefix = "v"
infile = "CHANGELOG.md"
header = "# Changelog\n"
release-commit-message-format = "chore(release): {{currentTag}}"
pre-major = false
# repository-url = "https://github.com/owner/repo"
types = [
  { type = "feat", section = "Features" },
  { type = "fix", section = "Bug Fixes" },
  { type = "chore", hidden = true },
]

[release.scripts]               # lifecycle scripts, run with `sh -c` (not in --dry-run)
prerelease = 'test "$GITHUB_ACTIONS" = true || { echo "Versions are bumped only by the release workflow." >&2; exit 1; }'
```

Overridden rules keep their position; new rules run after the default ones, as with commitlint's
`extends`. All 38 commitlint rules are available. Lifecycle hooks: `prerelease`, `prebump`, `postbump`,
`prechangelog`, `postchangelog`, `precommit`, `postcommit`, `pretag`, `posttag` (as in
commit-and-tag-version, the stdout of `prebump` can set the version and that of `precommit` the commit
message).

## Contributing

Commits follow [Conventional Commits](https://www.conventionalcommits.org) and are linted by ccr-lint
itself: in pull requests by CI, and locally by a `commit-msg` hook. Enable the hook once per clone:

```sh
git config core.hooksPath .githooks
```

Releases are cut only by the Release workflow (Actions → Release → Run workflow).

## How parity is checked

The port follows the JavaScript sources line by line, including their JavaScript semantics (UTF-16
lengths, JS regex classes and multiline anchors, ICU `localeCompare` for sorting, es-toolkit/lodash word
splitting for the case rules). It is then compared against the real tools:

- `scripts/diff-lint.sh` lints a corpus with `commitlint --edit` and `ccr-lint lint --edit` and diffs
  stdout and exit codes. `tests/lint_golden.rs` replays commitlint's recorded output for the edge-case
  corpus in `tests/corpus/` on every `cargo test`.
- `scripts/diff_release.py` clones a repository twice, runs `commit-and-tag-version` in one and
  `ccr-lint release` in the other with fixed git dates, and compares stdout, stderr, exit code,
  `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, the release commit and the tag. Its `synthetic` mode covers
  workspaces and single crates, breaking changes, issue and user links, code spans, merges, reverts,
  `Release-As`, prerelease tags, pre-1.0 versions, `--release-as`, `--first-release`, skips, lifecycle
  scripts (passing and failing), custom tag prefixes and headers, and GitHub/GitLab/no remotes.

Both pass with no differences, as does a full run over a real repository's history (900+ commits).

## Not ported

- commit-and-tag-version's `--prerelease`, `--commit-all`, `--lerna-package`, `--path`, `--release-count`,
  non-Cargo bump files and custom updaters.
- commitlint plugins, custom formatters, prompt settings and JavaScript ignore functions (use regexes in
  `ignores`), and `--git-log-args`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at
your option.
