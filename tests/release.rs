//! End-to-end release in a temporary Cargo workspace. Exact parity with commit-and-tag-version is
//! checked by scripts/diff_release.py; this pins the expected output so `cargo test` catches regressions.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn commit(dir: &Path, msg: &str) {
    git(dir, &["commit", "-q", "--allow-empty", "-m", msg]);
}

fn write(dir: &Path, path: &str, text: &str) {
    let p = dir.join(path);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, text).expect("write");
}

fn today() -> String {
    ccr_lint::changelog::today()
}

#[test]
fn workspace_release() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    write(
        dir,
        "Cargo.toml",
        "[workspace]\nmembers = [\"a\", \"b\"]\n\n[workspace.package]\nversion = \"1.2.3\"\n",
    );
    write(
        dir,
        "a/Cargo.toml",
        "[package]\nname = \"a\"\nversion.workspace = true\nedition = \"2021\"\n",
    );
    write(
        dir,
        "b/Cargo.toml",
        "[package]\nname = \"b\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(dir, "a/src/lib.rs", "");
    write(dir, "b/src/lib.rs", "");
    write(
        dir,
        "Cargo.lock",
        "version = 4\n\n[[package]]\nname = \"a\"\nversion = \"1.2.3\"\n\n[[package]]\nname = \"b\"\nversion = \"0.1.0\"\n",
    );
    git(dir, &["add", "-A"]);
    commit(dir, "chore: init");
    git(dir, &["tag", "-a", "v1.2.3", "-m", "v1.2.3"]);
    git(dir, &["remote", "add", "origin", "git@github.com:acme/widgets.git"]);
    commit(dir, "fix(core): handle #4");
    commit(dir, "feat: add x\n\nCloses #9");
    commit(dir, "docs: hidden");

    let out = Command::new(env!("CARGO_BIN_EXE_ccr-lint"))
        .arg("release")
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("FORCE_COLOR")
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "✔ bumping version in Cargo.toml from 1.2.3 to 1.3.0\n\
         ✔ bumping version in Cargo.lock from 1.2.3 to 1.3.0\n\
         ✔ created CHANGELOG.md\n\
         ✔ outputting changes to CHANGELOG.md\n\
         ✔ committing Cargo.lock and Cargo.toml and CHANGELOG.md\n\
         ✔ tagging release v1.3.0\n\
         ℹ Run `git push --follow-tags origin main` to publish\n"
    );
    let log = git(dir, &["log", "--format=%H %s", "-4"]);
    let hash = |subject: &str| {
        log.lines()
            .find(|l| l.ends_with(subject))
            .and_then(|l| l.split(' ').next())
            .expect("commit")
            .to_owned()
    };
    let (fix, feat) = (hash("fix(core): handle #4"), hash("feat: add x"));
    let changelog = std::fs::read_to_string(dir.join("CHANGELOG.md")).expect("changelog");
    let r = "https://github.com/acme/widgets";
    assert_eq!(
        changelog,
        format!(
            "# Changelog\n\nAll notable changes to this project will be documented in this file. See [commit-and-tag-version](https://github.com/absolute-version/commit-and-tag-version) for commit guidelines.\n\n\
             ## [1.3.0]({r}/compare/v1.2.3...v1.3.0) ({})\n\n\
             ### Features\n\n\
             * add x ([{}]({r}/commit/{feat})), closes [#9]({r}/issues/9)\n\n\
             ### Bug Fixes\n\n\
             * **core:** handle [#4]({r}/issues/4) ([{}]({r}/commit/{fix}))\n",
            today(),
            &feat[..7],
            &fix[..7],
        )
    );
    let toml = std::fs::read_to_string(dir.join("Cargo.toml")).expect("toml");
    assert!(toml.contains("version = \"1.3.0\""));
    let lock = std::fs::read_to_string(dir.join("Cargo.lock")).expect("lock");
    assert!(lock.contains("name = \"a\"\nversion = \"1.3.0\"") && lock.contains("name = \"b\"\nversion = \"0.1.0\""));
    assert_eq!(git(dir, &["log", "-1", "--format=%s"]), "chore(release): 1.3.0\n");
    assert!(git(dir, &["tag", "-n1", "v1.3.0"]).contains("chore(release): 1.3.0"));
    assert_eq!(git(dir, &["status", "--porcelain"]), "");
}
