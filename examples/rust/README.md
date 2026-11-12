# Rust project

For a single crate or a Cargo workspace at the repository root.

```
.github/workflows/commits.yml   lint the commits of every pull request
.github/workflows/release.yml   bump, changelog, tag, build, push to main, GitHub Release
.githooks/commit-msg            lint each commit locally (git config core.hooksPath .githooks)
ccr-lint.toml                   optional overrides
```

`ccr-lint release` reads and bumps the `version` of `[workspace.package]` (or `[package]`) in
`Cargo.toml`, and the matching entries of `Cargo.lock`.

In `release.yml`, set `BIN` to your binary's name. To build for more targets, copy the `binaries` matrix
job of this repository's own [`github-release.yml`](../../.github/workflows/github-release.yml).

Before the first `v` tag exists, a release bumps the version in `Cargo.toml` based on the whole history.
To start counting from the current version instead, tag it once:
`git tag -a v0.1.0 -m "chore(release): 0.1.0" && git push origin v0.1.0`.
