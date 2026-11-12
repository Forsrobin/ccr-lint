# Examples

Each folder is laid out like the root of a repository, so its files can be copied as they are (for
example `cp -r examples/node/. ../my-app/`). The workflows only run once they sit in a repository's own
`.github/workflows/`; here they are just files.

| Example | Project | What it shows |
| --- | --- | --- |
| [`rust/`](rust) | A single crate or a Cargo workspace | Lint pull requests; a release workflow that bumps the version, builds the binaries, pushes to `main` and creates the GitHub Release |
| [`monorepo/`](monorepo) | A Node monorepo with a Rust crate in `packages/engine` | Releasing one package from a subfolder with its own tag prefix (`engine-v1.2.3`), changelog and config overrides |
| [`node/`](node) | Only a `package.json`, no Rust at all | Bumping `package.json` and `package-lock.json` through lifecycle scripts, so commitlint, husky and commit-and-tag-version can go |

All release workflows follow the same order:

1. `ccr-lint release`: pick the next version from the commits since the last tag, update the version
   files and `CHANGELOG.md`, commit `chore(release): X.Y.Z` and tag it. Nothing is pushed yet.
2. Read the new version from the tag.
3. Build and upload the artifacts. A failed build stops the run before anything reaches `main`.
4. `git push --follow-tags origin HEAD:main`, then create the GitHub Release with the changelog section
   as its notes and the artifacts attached.

Each one is started by hand (Actions → Release → Run workflow) and has a `dry-run` input that only
prints the next version and changelog.

## Pushing to a protected `main`

The workflows push the release commit with the default `GITHUB_TOKEN`. If `main` is protected, either
let GitHub Actions bypass the rule (Settings → Rules → the ruleset → Bypass list) or check out with a
token that may push:

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
    token: ${{ secrets.RELEASE_TOKEN }} # fine-grained PAT or GitHub App token with contents: write
```

A push made with `GITHUB_TOKEN` does not start other workflows. If a workflow should run on the release
tag (publishing, deploying), use a token as above, or call that workflow from the release job with
`workflow_call`, as this repository's own `.github/workflows/release.yml` does.
