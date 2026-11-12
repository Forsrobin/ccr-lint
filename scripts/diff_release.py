#!/usr/bin/env python3
"""Differential test of `ccr-lint release` against commit-and-tag-version.

Clones a repository twice, runs commit-and-tag-version (with Cargo updaters equivalent to ccr-lint's
built-in ones) in one clone and `ccr-lint release` in the other, with fixed git dates, then compares
stdout, stderr, exit code, CHANGELOG.md, Cargo.toml, Cargo.lock, the release commit and the tag.

    NODE_MODULES=/path/to/node_modules scripts/diff_release.py synthetic
    NODE_MODULES=... scripts/diff_release.py repo /path/to/repo REF [--delete-tag vX] [--remote URL] [-- args]

NODE_MODULES must contain commit-and-tag-version 13.3.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.environ.get("CCR_LINT", os.path.join(HERE, "target", "release", "ccr-lint"))
NODE_MODULES = os.environ["NODE_MODULES"]

CARGO_TOML_UPDATER = r"""
function find(contents) {
  for (const section of ['workspace.package', 'package']) {
    let inSection = false, offset = 0;
    for (const line of contents.split(/(?<=\n)/)) {
      const t = line.replace(/[\r\n]+$/, '');
      if (t.trimStart().startsWith('[')) {
        const m = t.match(/^\s*\[\s*([^\]\[]+?)\s*\]\s*(?:#.*)?$/);
        inSection = !!m && m[1] === section && !t.trimStart().startsWith('[[');
      } else if (inSection) {
        const m = t.match(/^(\s*version\s*=\s*")([^"]*)"/);
        if (m) return [offset + m[1].length, offset + m[1].length + m[2].length];
      }
      offset += line.length;
    }
  }
  throw new Error('Cargo.toml: no version in [workspace.package] or [package]');
}
module.exports.readVersion = (c) => { const [s, e] = find(c); return c.slice(s, e); };
module.exports.writeVersion = (c, v) => { const [s, e] = find(c); return c.slice(0, s) + v + c.slice(e); };
"""

CARGO_LOCK_UPDATER = r"""
const MEMBERS = %s;
function entry(name) {
  return new RegExp(`(\\[\\[package\\]\\]\\r?\\nname = "${name}"\\r?\\nversion = ")([^"]+)(")`);
}
module.exports.readVersion = function (contents) {
  if (!MEMBERS.length) throw new Error('Cargo.lock: no workspace package inherits the version');
  const match = contents.match(entry(MEMBERS[0]));
  if (!match) throw new Error(`Cargo.lock: no entry for ${MEMBERS[0]}`);
  return match[2];
};
module.exports.writeVersion = function (contents, version) {
  return MEMBERS.reduce((out, name) => {
    if (!entry(name).test(out)) throw new Error(`Cargo.lock: no entry for ${name}`);
    return out.replace(entry(name), `$1${version}$3`);
  }, contents);
};
"""


def sh(cmd, cwd, env=None, check=True):
    r = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True, shell=isinstance(cmd, str))
    if check and r.returncode != 0:
        raise SystemExit(f"command failed: {cmd}\n{r.stdout}\n{r.stderr}")
    return r


def git_env(base=None):
    env = dict(base or os.environ)
    env.update({
        "GIT_AUTHOR_NAME": "Test", "GIT_AUTHOR_EMAIL": "test@example.com",
        "GIT_COMMITTER_NAME": "Test", "GIT_COMMITTER_EMAIL": "test@example.com",
        "GIT_AUTHOR_DATE": "2026-01-02T03:04:05+01:00", "GIT_COMMITTER_DATE": "2026-01-02T03:04:05+01:00",
        "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1",
    })
    if base is None:
        env.pop("GITHUB_ACTIONS", None)
        env.pop("CI", None)
    env["NO_COLOR"] = "1"
    return env


def members_inheriting(repo):
    r = sh(["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"], repo, check=False)
    if r.returncode != 0:
        return []
    meta = json.loads(r.stdout)
    root = open(os.path.join(repo, "Cargo.toml")).read()
    import re
    if not re.search(r"^\s*\[\s*workspace\.package\s*\]", root, re.M):
        m = re.search(r'^\s*\[\s*package\s*\][\s\S]*?^\s*name\s*=\s*"([^"]+)"', root, re.M)
        return [m.group(1)] if m else []
    names = []
    for p in meta["packages"]:
        if p["id"] not in meta["workspace_members"]:
            continue
        text = open(p["manifest_path"]).read()
        if re.search(r"^\s*version\s*\.\s*workspace\s*=\s*true", text, re.M) or re.search(
            r"^\s*version\s*=\s*\{\s*workspace\s*=\s*true", text, re.M
        ):
            names.append(p["name"])
    return names


def setup_node(clone, scripts, cfg_extra):
    vdir = os.path.join(clone, ".ccr-test")
    os.makedirs(vdir, exist_ok=True)
    open(os.path.join(vdir, "cargo-toml.cjs"), "w").write(CARGO_TOML_UPDATER)
    open(os.path.join(vdir, "cargo-lock.cjs"), "w").write(CARGO_LOCK_UPDATER % json.dumps(members_inheriting(clone)))
    bump = [{"filename": "Cargo.toml", "updater": ".ccr-test/cargo-toml.cjs"}]
    if os.path.exists(os.path.join(clone, "Cargo.lock")):
        bump.append({"filename": "Cargo.lock", "updater": ".ccr-test/cargo-lock.cjs"})
    types = [
        {"type": "feat", "section": "Features"}, {"type": "fix", "section": "Bug Fixes"},
        {"type": "perf", "section": "Performance"}, {"type": "revert", "section": "Reverts"},
        *({"type": t, "hidden": True} for t in ["refactor", "docs", "style", "test", "build", "ci", "chore"]),
    ]
    cfg = {"packageFiles": bump[:1], "bumpFiles": bump, "tagPrefix": "v", "scripts": scripts, "types": types}
    cfg.update(cfg_extra)
    open(os.path.join(clone, ".versionrc.json"), "w").write(json.dumps(cfg, indent=2))
    os.symlink(NODE_MODULES, os.path.join(clone, "node_modules"))
    with open(os.path.join(clone, ".git", "info", "exclude"), "a") as f:
        f.write("\n.ccr-test/\n.versionrc.json\nnode_modules\nccr-lint.toml\n")


def setup_rust(clone, scripts, toml_extra):
    lines = ["[release]"] + toml_extra
    if scripts:
        lines.append("[release.scripts]")
        lines += [f"{k} = {json.dumps(v)}" for k, v in scripts.items()]
    open(os.path.join(clone, "ccr-lint.toml"), "w").write("\n".join(lines) + "\n")
    with open(os.path.join(clone, ".git", "info", "exclude"), "a") as f:
        f.write("\n.ccr-test/\n.versionrc.json\nnode_modules\nccr-lint.toml\n")


def snapshot(clone, r):
    def read(p):
        p = os.path.join(clone, p)
        return open(p).read() if os.path.exists(p) else None
    head = sh(["git", "log", "-1", "--format=%B%n%T%n%an%n%ad"], clone, check=False).stdout
    tags = sh(["git", "tag", "-n99", "--sort=refname"], clone, check=False).stdout
    files = sh(["git", "show", "--stat", "--format=", "HEAD"], clone, check=False).stdout
    status = sh(["git", "status", "--porcelain"], clone, check=False).stdout
    return {
        "exit": r.returncode, "stdout": r.stdout, "stderr": r.stderr,
        "CHANGELOG.md": read("CHANGELOG.md"), "Cargo.toml": read("Cargo.toml"), "Cargo.lock": read("Cargo.lock"),
        "head": head, "tags": tags, "files": files, "status": status,
    }


def compare(name, src, run_env=None, args=(), scripts=None, cfg_extra=None, toml_extra=None, remote=None):
    scripts = scripts or {}
    with tempfile.TemporaryDirectory() as tmp:
        results = {}
        for kind in ("node", "rust"):
            clone = os.path.join(tmp, kind)
            sh(["git", "clone", "-q", src, clone], tmp)
            sh(["git", "fetch", "-q", "--tags"], clone)
            if remote:
                sh(["git", "remote", "set-url", "origin", remote], clone)
            else:
                sh(["git", "remote", "remove", "origin"], clone)
            env = git_env(run_env)
            if kind == "node":
                setup_node(clone, scripts, cfg_extra or {})
                node_args = [a.replace("--skip-", "--skip.", 1) if a.startswith("--skip-") else a for a in args]
                cmd = ["npx", "--no-install", "commit-and-tag-version", *node_args]
            else:
                setup_rust(clone, scripts, toml_extra or [])
                cmd = [BIN, "release", *args]
            r = subprocess.run(cmd, cwd=clone, env=env, capture_output=True, text=True)
            results[kind] = snapshot(clone, r)
        diffs = [k for k in results["node"] if results["node"][k] != results["rust"][k]]
        print(f"{'OK  ' if not diffs else 'DIFF'} {name} (exit {results['node']['exit']})")
        for k in diffs:
            a, b = results["node"][k], results["rust"][k]
            print(f"  --- {k} differs")
            import difflib
            for line in difflib.unified_diff(str(a).splitlines(), str(b).splitlines(), "node", "rust", lineterm="", n=2):
                print("   ", line)
        return not diffs


# ------------------------------------------------------------------ synthetic repository

WORKSPACE_TOML = """[workspace]
resolver = "2"
members = ["crates/alpha", "crates/beta", "crates/gamma"]

[workspace.package]
edition = "2021"
version = "{version}"

[workspace.dependencies]
serde = {{ version = "1" }}
"""

MEMBER_TOML = """[package]
name = "{name}"
version.workspace = true
edition.workspace = true
"""

FIXED_MEMBER_TOML = """[package]
name = "{name}"
version = "0.0.7"
edition = "2021"
"""


def make_repo(path, version, commits, single=False):
    os.makedirs(path)
    env = git_env()
    sh(["git", "init", "-q", "-b", "main"], path, env)
    if single:
        open(os.path.join(path, "Cargo.toml"), "w").write(
            f'[package]\nname = "solo"\nauthors = ["x"]\nversion = "{version}"\nedition = "2021"\n')
        os.makedirs(os.path.join(path, "src"))
        open(os.path.join(path, "src", "lib.rs"), "w").write("")
    else:
        open(os.path.join(path, "Cargo.toml"), "w").write(WORKSPACE_TOML.format(version=version))
        for name in ("alpha", "beta"):
            d = os.path.join(path, "crates", name)
            os.makedirs(os.path.join(d, "src"))
            open(os.path.join(d, "Cargo.toml"), "w").write(MEMBER_TOML.format(name=name))
            open(os.path.join(d, "src", "lib.rs"), "w").write("")
        d = os.path.join(path, "crates", "gamma")
        os.makedirs(os.path.join(d, "src"))
        open(os.path.join(d, "Cargo.toml"), "w").write(FIXED_MEMBER_TOML.format(name="gamma"))
        open(os.path.join(d, "src", "lib.rs"), "w").write("")
    sh(["cargo", "generate-lockfile", "--offline"], path, env)
    sh(["git", "add", "-A"], path, env)
    sh(["git", "commit", "-q", "-m", "chore: initial"], path, env)
    n = 0
    for c in commits:
        if isinstance(c, tuple) and c[0] == "tag":
            sh(["git", "tag", "-a", c[1], "-m", c[1]], path, env)
            continue
        if isinstance(c, tuple) and c[0] == "merge":
            sh(["git", "checkout", "-q", "-b", c[1]], path, env)
            for m in c[2]:
                n += 1
                open(os.path.join(path, f"f{n}"), "w").write(str(n))
                sh(["git", "add", "-A"], path, env)
                sh(["git", "commit", "-q", "-m", m], path, env)
            sh(["git", "checkout", "-q", "main"], path, env)
            sh(["git", "merge", "-q", "--no-ff", c[1], "-m", f"Merge branch '{c[1]}'"], path, env)
            continue
        if isinstance(c, tuple) and c[0] == "revert-last":
            last = sh(["git", "rev-parse", "HEAD"], path, env).stdout.strip()
            sh(["git", "revert", "--no-edit", last], path, env)
            continue
        n += 1
        open(os.path.join(path, f"f{n}"), "w").write(str(n))
        sh(["git", "add", "-A"], path, env)
        sh(["git", "commit", "-q", "--allow-empty", "-m", c], path, env)


SYNTH_COMMITS = [
    "feat: first feature", ("tag", "v1.0.0"),
    "fix: a bug (#12)", "fix(parser): handle `#3` in code and #4 outside",
    "feat(api): add endpoint\n\nCloses #7, fixes org/other#8\nRefs #9",
    "feat: mention @someone and @org/team",
    "perf: faster [#5](https://x/y) and https://example.com/#6",
    "docs: hidden docs", "chore: hidden chore", "refactor(core)!: drop old api",
    "fix: Zebra capital", "fix: apple lower", "fix(b): beta scoped", "fix(a): alpha scoped",
    "feat(*): star scope", "build: hidden\n\nRelease-As: 3.0.0",
    "fix: second\n\nBREAKING CHANGE: the config moved\nto a new place",
    ("merge", "topic", ["feat(topic): merged feature", "fix: merged fix"]),
    "feat: will be reverted", ("revert-last",),
    "unknown: type with note\n\nBREAKING-CHANGE: weird", "Not conventional at all",
    "fix: ünïcode sörting", "fix: Émile", "fix: émile",
]


def synthetic():
    ok = True
    with tempfile.TemporaryDirectory() as tmp:
        repo = os.path.join(tmp, "repo")
        make_repo(repo, "1.0.0", SYNTH_COMMITS)
        gh = "git@github.com:acme/widgets.git"
        ok &= compare("workspace dry-run", repo, args=["--dry-run"], remote=gh)
        ok &= compare("workspace release", repo, remote=gh)
        ok &= compare("no remote", repo)
        ok &= compare("gitlab https remote", repo, remote="https://gitlab.com/group/sub/proj.git")
        ok &= compare("release-as minor", repo, args=["--release-as", "minor"], remote=gh)
        ok &= compare("release-as exact", repo, args=["--release-as", "5.0.0-rc.1+b.7"], remote=gh)
        ok &= compare("release-as invalid", repo, args=["--release-as", "nope"], remote=gh)
        ok &= compare("failing prerelease script", repo, remote=gh,
                      scripts={"prerelease": 'test "$GITHUB_ACTIONS" = true || { echo "Versions are bumped only by the release workflow." >&2; exit 1; }'})
        ok &= compare("passing prerelease script", repo, remote=gh, run_env={**os.environ, "GITHUB_ACTIONS": "true"},
                      scripts={"prerelease": 'test "$GITHUB_ACTIONS" = true || exit 1', "postbump": "echo bumped",
                               "precommit": "echo 'chore(release): custom {{currentTag}}'", "posttag": "echo warn >&2"})
        ok &= compare("custom header and prefix", repo, remote=gh, args=["--dry-run"],
                      cfg_extra={"header": "# History\n", "tagPrefix": "release-"},
                      toml_extra=['header = "# History\\n"', 'tag-prefix = "release-"'])

        pre = os.path.join(tmp, "pre")
        make_repo(pre, "0.3.0", ["feat: a", ("tag", "v0.3.0"), "feat: b", "fix!: c", ("tag", "v0.4.0-rc.1"), "fix: d"])
        ok &= compare("pre-1.0 with prerelease tag", pre, remote=gh)

        empty = os.path.join(tmp, "empty")
        make_repo(empty, "0.1.0", [("tag", "v0.1.0")])
        ok &= compare("no commits since tag", empty, remote=gh)

        notag = os.path.join(tmp, "notag")
        make_repo(notag, "0.1.0", ["feat: x", "fix: y"])
        ok &= compare("no tags yet", notag, remote=gh)
        ok &= compare("first release", notag, args=["--first-release"], remote=gh)
        ok &= compare("skip changelog", notag, args=["--skip-changelog"], remote=gh)

        single = os.path.join(tmp, "single")
        make_repo(single, "2.0.0", ["fix: a", ("tag", "v2.0.0"), "feat: b"], single=True)
        ok &= compare("single crate", single, remote=gh)
    return ok


def main():
    if sys.argv[1] == "synthetic":
        sys.exit(0 if synthetic() else 1)
    if sys.argv[1] == "repo":
        src, ref = sys.argv[2], sys.argv[3]
        rest = sys.argv[4:]
        delete, remote, args, scripts = [], None, [], {}
        while rest:
            a = rest.pop(0)
            if a == "--delete-tag":
                delete.append(rest.pop(0))
            elif a == "--remote":
                remote = rest.pop(0)
            elif a == "--prerelease-script":
                scripts["prerelease"] = rest.pop(0)
            elif a == "--":
                args = rest
                break
        with tempfile.TemporaryDirectory() as tmp:
            base = os.path.join(tmp, "base")
            sh(["git", "clone", "-q", "--no-local", src, base], tmp)
            sh(["git", "checkout", "-q", "-B", "main", ref], base)
            for t in delete:
                sh(["git", "tag", "-d", t], base)
            env = {**os.environ, "GITHUB_ACTIONS": "true"} if scripts else None
            ok = compare(f"{src}@{ref}", base, run_env=env, args=args, scripts=scripts, remote=remote)
        sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
