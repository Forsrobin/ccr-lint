//! Git access the ported tools perform (`@conventional-changelog/git-client` and direct `git` calls).

use std::path::Path;
use std::process::{Command, Output};
use std::sync::LazyLock;

use fancy_regex::Regex;

use crate::js;
use crate::semver;

pub const SCISSOR: &str = "------------------------ >8 ------------------------";

#[derive(Debug)]
pub struct GitError(pub String);

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub fn output(cwd: &Path, args: &[&str]) -> Result<Output, GitError> {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| GitError(format!("git: {e}")))
}

/// Run git, returning stdout; a non-zero exit is an error carrying stderr.
pub fn run(cwd: &Path, args: &[&str]) -> Result<String, GitError> {
    let out = output(cwd, args)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        Err(GitError(if stderr.is_empty() {
            format!("git {} failed", args.join(" "))
        } else {
            stderr
        }))
    }
}

/// `GitClient#getRawCommits`: `git log --format=<format>%n<SCISSOR> [--no-merges] [from..]to`, split on
/// the scissor line.
pub fn raw_commits(
    cwd: &Path,
    format: &str,
    from: Option<&str>,
    to: &str,
    no_merges: bool,
) -> Result<Vec<String>, GitError> {
    let fmt = format!("--format={format}%n{SCISSOR}");
    let range = [from.unwrap_or(""), to]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("..");
    let mut args = vec!["log", fmt.as_str()];
    if no_merges {
        args.push("--no-merges");
    }
    args.push("--end-of-options");
    args.push(&range);
    let stdout = run(cwd, &args)?;
    let sep = format!("{SCISSOR}\n");
    let mut parts: Vec<String> = stdout.split(sep.as_str()).map(str::to_owned).collect();
    // splitStream yields the remainder only when it is non-empty
    if parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    Ok(parts)
}

static TAG: LazyLock<Regex> = LazyLock::new(|| js::re(&format!(r"(?i)tag:{}*({}+?)[,)]", js::WS, js::DOT)));

/// `GitClient#getTags`: every `tag: X` in `git log --decorate --no-color --date-order HEAD`.
pub fn tags(cwd: &Path) -> Result<Vec<String>, GitError> {
    let stdout = run(
        cwd,
        &[
            "log",
            "--decorate",
            "--no-color",
            "--date-order",
            "--end-of-options",
            "HEAD",
        ],
    )?;
    Ok(TAG
        .captures_iter(js::trim(&stdout))
        .flatten()
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_owned()))
        .collect())
}

static UNSTABLE: LazyLock<Regex> = LazyLock::new(|| js::re(r"[0-9]+\.[0-9]+\.[0-9]+-.+"));

/// `ConventionalGitClient#getSemverTags({ prefix, skipUnstable })` (tags are not cleaned).
pub fn semver_tags(cwd: &Path, prefix: &str, skip_unstable: bool) -> Result<Vec<String>, GitError> {
    Ok(tags(cwd)?
        .into_iter()
        .filter(|tag| !(skip_unstable && js::test(&UNSTABLE, tag)))
        .filter(|tag| {
            if prefix.is_empty() {
                semver::valid(tag).is_some()
            } else {
                tag.starts_with(prefix) && semver::valid(&tag.replacen(prefix, "", 1)).is_some()
            }
        })
        .collect())
}

pub fn config_get(cwd: &Path, key: &str) -> Option<String> {
    run(cwd, &["config", "--get", key])
        .ok()
        .map(|s| js::trim(&s).to_owned())
        .filter(|s| !s.is_empty())
}

pub fn verify(cwd: &Path, rev: &str) -> bool {
    output(cwd, &["rev-parse", "--verify", "--end-of-options", rev]).is_ok_and(|o| o.status.success())
}
