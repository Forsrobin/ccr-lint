# Monorepo with a Rust package

A Node monorepo where one package, `packages/engine`, is a Rust crate:

```
package.json                          npm workspaces
ccr-lint.toml                         lint rules for the whole repository
.githooks/commit-msg                  lint each commit locally
.github/workflows/commits.yml         lint the commits of every pull request
.github/workflows/release-engine.yml  release packages/engine
packages/
  web/package.json                    a Node package
  engine/
    Cargo.toml                        version bumped by the engine release
    Cargo.lock
    ccr-lint.toml                     release overrides for the engine
    CHANGELOG.md                      written by the engine release
```

## How it works

`ccr-lint release` works on the folder it runs in: it bumps the `Cargo.toml` and `Cargo.lock` there and
writes `CHANGELOG.md` there. Configuration is the exception: by default it comes from the repository
root, so the engine's release passes its own file:

```sh
cd packages/engine
ccr-lint release --config ccr-lint.toml --dry-run
```

The overrides in [`packages/engine/ccr-lint.toml`](packages/engine/ccr-lint.toml):

- `tag-prefix = "engine-v"`: tags are shared by the whole repository, so each package needs its own
  prefix. The engine's previous release is the newest `engine-v*` tag.
- `release-commit-message-format`: `chore(release): engine 0.4.3`, so the history shows which package
  was released.
- `types` with a `scope`: the changelog only lists `feat(engine)`, `fix(engine)`, `perf(engine)` and
  `revert(engine)` commits. The root `ccr-lint.toml` makes the scope required (`scope-enum` and
  `scope-empty`), so this works only if every commit names its package.
- `prerelease` script: stops the release when nothing under `packages/engine` changed since the last
  `engine-v` tag.

The `--config` file replaces the root `ccr-lint.toml` rather than adding to it. That makes no difference
here, because the root file has only `[lint]` settings and the engine file only `[release]` settings.

## Limitation: the bump counts every commit

ccr-lint does not filter commits by path (commit-and-tag-version's `--path` is not ported). The
changelog can be filtered by scope as above, but the version bump looks at every commit since the last
`engine-v` tag: a `feat(web): ...` commit makes the next engine release a minor one, and a breaking
`feat(web)!: ...` makes it a major one and also shows up in the engine's changelog. Override the bump
when needed with the workflow's `release-as` input (`patch`, `minor`, `major` or an exact version).

## Releasing a Node package of the same repository

Follow the [`node`](../node) example inside `packages/web`, with its own config:
`tag-prefix = "web-v"`, `types` scoped to `web`, and the two lifecycle scripts for `package.json`.
Without a `Cargo.toml` the current version comes from the newest `web-v` tag, so create
`web-v2.1.0` once before the first release.
