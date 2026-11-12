//! Port of commit-and-tag-version 13.3 (`index.js`, `lib/lifecycles/*`), with Cargo.toml / Cargo.lock as
//! the package and bump files.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use fancy_regex::Regex;

use crate::changelog::{self, Generator};
use crate::color::{Palette, figures};
use crate::config::ReleaseConfig;
use crate::git;
use crate::js;
use crate::parser::{CommitParser, ParserOptions};
use crate::semver::{self, Release};

#[derive(Debug, Clone, Default)]
pub struct ReleaseArgs {
    pub dry_run: bool,
    pub release_as: Option<String>,
    pub first_release: bool,
    pub skip_bump: bool,
    pub skip_changelog: bool,
    pub skip_commit: bool,
    pub skip_tag: bool,
    pub silent: bool,
}

/// A failure that has already been reported the way commit-and-tag-version reports it.
#[derive(Debug)]
pub struct Failed;

struct Ctx<'a> {
    cwd: &'a Path,
    cfg: &'a ReleaseConfig,
    args: &'a ReleaseArgs,
    pc: Palette,
    updated: Vec<String>,
    commit_message_format: String,
}

impl Ctx<'_> {
    /// `checkpoint(args, msg, args, figure)`.
    fn checkpoint(&self, msg: &str, values: &[&str], figure: Option<String>) {
        if self.args.silent {
            return;
        }
        let fig = figure.unwrap_or_else(|| {
            let t = figures().tick;
            if self.args.dry_run {
                self.pc.yellow(t)
            } else {
                self.pc.green(t)
            }
        });
        println!(
            "{fig} {}",
            util_format(msg, &values.iter().map(|v| self.pc.bold(v)).collect::<Vec<_>>())
        );
    }

    fn print_error(&self, msg: &str, warn: bool) {
        if self.args.silent {
            return;
        }
        let colored = if warn { self.pc.yellow(msg) } else { self.pc.red(msg) };
        eprintln!("{colored}");
    }

    /// `runLifecycleScript`: returns stdout (`None` when there is no script or in a dry run).
    fn lifecycle(&self, hook: &str) -> Result<Option<String>, Failed> {
        let Some(cmd) = self.cfg.scripts.get(hook).filter(|c| !c.is_empty()) else {
            return Ok(None);
        };
        self.checkpoint("Running lifecycle script \"%s\"", &[hook], None);
        self.checkpoint("- execute command: \"%s\"", &[cmd], Some(self.pc.blue(figures().info)));
        if self.args.dry_run {
            return Ok(None);
        }
        let out = Command::new("sh").arg("-c").arg(cmd).current_dir(self.cwd).output();
        match out {
            Ok(o) if o.status.success() => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                if !stderr.is_empty() {
                    self.print_error(&stderr, true);
                }
                Ok(Some(String::from_utf8_lossy(&o.stdout).into_owned()))
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
                let message = format!("Command failed: {cmd}\n{stderr}");
                self.print_error(if stderr.is_empty() { &message } else { &stderr }, false);
                // index.js reports the error message once more
                self.print_error(&message, false);
                Err(Failed)
            }
            Err(e) => {
                self.print_error(&e.to_string(), false);
                self.print_error(&e.to_string(), false);
                Err(Failed)
            }
        }
    }

    /// `runExecFile(args, 'git', ...)`.
    fn git(&self, git_args: &[&str]) -> Result<(), Failed> {
        if self.args.dry_run {
            return Ok(());
        }
        match git::output(self.cwd, git_args) {
            Ok(o) if o.status.success() => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                if !stderr.is_empty() {
                    self.print_error(&stderr, true);
                }
                Ok(())
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
                let message = format!("Command failed: git {}\n{stderr}", git_args.join(" "));
                self.print_error(if stderr.is_empty() { &message } else { &stderr }, false);
                self.print_error(&message, false);
                Err(Failed)
            }
            Err(e) => {
                self.print_error(&e.0, false);
                self.print_error(&e.0, false);
                Err(Failed)
            }
        }
    }

    fn fail(&self, msg: &str) -> Failed {
        self.print_error(msg, false);
        Failed
    }
}

/// `util.format(msg, ...values)` for `%s` placeholders.
fn util_format(msg: &str, values: &[String]) -> String {
    let mut out = String::new();
    let mut it = values.iter();
    let mut rest = msg;
    while let Some(i) = rest.find("%s") {
        out.push_str(&rest[..i]);
        match it.next() {
            Some(v) => out.push_str(v),
            None => out.push_str("%s"),
        }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    for v in it {
        out.push(' ');
        out.push_str(v);
    }
    out
}

// ---------------------------------------------------------------- Cargo files

/// Where the project version lives in the root Cargo.toml.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionSection {
    Workspace,
    Package,
}

/// The `version = "..."` line of a section: (line index, value start, value end) in byte offsets.
fn find_version(contents: &str, section: &str) -> Option<(usize, usize)> {
    static HEADER: LazyLock<Regex> = LazyLock::new(|| js::re(r"^\s*\[\s*([^\]\[]+?)\s*\]\s*(?:#.*)?$"));
    static VERSION: LazyLock<Regex> = LazyLock::new(|| js::re(r#"^\s*version\s*=\s*"([^"]*)""#));
    let mut in_section = false;
    let mut offset = 0;
    for line in contents.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.trim_start().starts_with('[') {
            in_section = HEADER
                .captures(trimmed)
                .ok()
                .flatten()
                .and_then(|c| c.get(1).map(|m| m.as_str() == section))
                .unwrap_or(false)
                && !trimmed.trim_start().starts_with("[[");
        } else if in_section
            && let Ok(Some(c)) = VERSION.captures(trimmed)
            && let Some(m) = c.get(1)
        {
            return Some((offset + m.start(), offset + m.end()));
        }
        offset += line.len();
    }
    None
}

fn manifest_version(contents: &str) -> Option<(VersionSection, String)> {
    for (section, kind) in [
        ("workspace.package", VersionSection::Workspace),
        ("package", VersionSection::Package),
    ] {
        if let Some((s, e)) = find_version(contents, section) {
            return Some((kind, contents[s..e].to_owned()));
        }
    }
    None
}

fn write_manifest_version(contents: &str, version: &str) -> Option<String> {
    for section in ["workspace.package", "package"] {
        if let Some((s, e)) = find_version(contents, section) {
            return Some(format!("{}{version}{}", &contents[..s], &contents[e..]));
        }
    }
    None
}

/// Names of the packages whose version is the root manifest's: the workspace members that inherit
/// `version.workspace = true`, or the root package.
fn versioned_packages(cwd: &Path, root: &str, kind: VersionSection) -> Vec<String> {
    let root_doc: Option<toml::Table> = toml::from_str(root).ok();
    if kind == VersionSection::Package {
        return root_doc
            .as_ref()
            .and_then(|d| d.get("package")?.get("name")?.as_str().map(str::to_owned))
            .into_iter()
            .collect();
    }
    let Ok(out) = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1", "--offline"])
        .current_dir(cwd)
        .output()
    else {
        return Vec::new();
    };
    let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return Vec::new();
    };
    let members: Vec<&str> = meta
        .get("workspace_members")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .collect();
    let mut names = Vec::new();
    for pkg in meta.get("packages").and_then(|v| v.as_array()).into_iter().flatten() {
        let Some(id) = pkg["id"].as_str() else { continue };
        if !members.contains(&id) {
            continue;
        }
        let Some(manifest) = pkg["manifest_path"].as_str() else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(manifest) else {
            continue;
        };
        let Ok(doc) = toml::from_str::<toml::Table>(&text) else {
            continue;
        };
        let inherits = doc
            .get("package")
            .and_then(|p| p.get("version"))
            .and_then(|v| v.get("workspace"))
            .and_then(toml::Value::as_bool)
            .unwrap_or(false);
        if inherits && let Some(name) = pkg["name"].as_str() {
            names.push(name.to_owned());
        }
    }
    names
}

fn lock_entry(name: &str) -> Regex {
    js::re(&format!(
        r#"(\[\[package\]\]\r?\nname = "{}"\r?\nversion = ")([^"]+)(")"#,
        js::escape(name)
    ))
}

fn lock_version(contents: &str, names: &[String]) -> Result<String, String> {
    let first = names
        .first()
        .ok_or("Cargo.lock: no workspace package inherits the version")?;
    lock_entry(first)
        .captures(contents)
        .ok()
        .flatten()
        .and_then(|c| c.get(2).map(|m| m.as_str().to_owned()))
        .ok_or_else(|| format!("Cargo.lock: no entry for {first}"))
}

fn write_lock_version(contents: &str, names: &[String], version: &str) -> Result<String, String> {
    let mut out = contents.to_owned();
    for name in names {
        let re = lock_entry(name);
        if !js::test(&re, &out) {
            return Err(format!("Cargo.lock: no entry for {name}"));
        }
        out = re
            .replace(&out, |c: &fancy_regex::Captures<'_, str>| {
                format!(
                    "{}{version}{}",
                    c.get(1).map_or("", |m| m.as_str()),
                    c.get(3).map_or("", |m| m.as_str())
                )
            })
            .into_owned();
    }
    Ok(out)
}

// ---------------------------------------------------------------- lifecycles

pub fn run(cwd: &Path, cfg: &ReleaseConfig, args: &ReleaseArgs) -> Result<(), Failed> {
    let mut ctx = Ctx {
        cwd,
        cfg,
        args,
        pc: Palette::chalk(),
        updated: Vec::new(),
        commit_message_format: cfg.release_commit_message_format.clone(),
    };
    let manifest_path = cwd.join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path).ok();
    let current = manifest.as_deref().and_then(manifest_version);
    let version = match &current {
        Some((_, v)) => v.clone(),
        None => latest_semver_tag(cwd, &cfg.tag_prefix),
    };

    let Some(new_version) = bump(&mut ctx, &version, manifest.as_deref())? else {
        return Ok(());
    };
    if !args.skip_changelog {
        ctx.lifecycle("prechangelog")?;
        output_changelog(&ctx, &new_version)?;
        ctx.lifecycle("postchangelog")?;
    }
    if !args.skip_commit {
        let message = ctx.lifecycle("precommit")?;
        if let Some(m) = message.filter(|m| !m.is_empty()) {
            ctx.commit_message_format = m;
        }
        exec_commit(&ctx, &new_version)?;
        ctx.lifecycle("postcommit")?;
    }
    if !args.skip_tag {
        ctx.lifecycle("pretag")?;
        exec_tag(&ctx, &new_version)?;
        ctx.lifecycle("posttag")?;
    }
    Ok(())
}

fn latest_semver_tag(cwd: &Path, prefix: &str) -> String {
    let tags = git::semver_tags(cwd, prefix, false).unwrap_or_default();
    let mut versions: Vec<semver::Version> = tags
        .iter()
        .filter_map(|t| semver::clean(&t.replacen(prefix, "", 1)))
        .filter_map(|v| semver::parse(&v))
        .collect();
    versions.sort_by(|a, b| semver::compare(b, a));
    versions.first().map_or_else(|| "1.0.0".to_owned(), ToString::to_string)
}

fn bump(ctx: &mut Ctx<'_>, version: &str, manifest: Option<&str>) -> Result<Option<String>, Failed> {
    let args = ctx.args;
    if args.skip_bump {
        return Ok(Some(version.to_owned()));
    }
    let mut release_as = args.release_as.clone();
    if let Some(r) = &release_as
        && Release::parse(r).is_none()
        && semver::valid(r).is_none()
    {
        return Err(ctx.fail("releaseAs must be one of 'major', 'minor' or 'patch', or a valid semvar version."));
    }
    ctx.lifecycle("prerelease")?;
    if let Some(stdout) = ctx.lifecycle("prebump")? {
        let s = js::trim(&stdout).replace(['\'', '"'], "");
        if !s.is_empty() && semver::valid(&s).is_some() {
            release_as = Some(s);
        }
    }
    let mut new_version = version.to_owned();
    if !args.first_release {
        if let Some(v) = release_as.as_deref().and_then(semver::parse) {
            new_version = v.to_string();
            if !v.build.is_empty() {
                new_version = format!("{new_version}+{}", v.build.join("."));
            }
        } else {
            let release = match release_as.as_deref().and_then(Release::parse) {
                Some(r) => r,
                None => what_bump(ctx, version)?,
            };
            new_version =
                semver::inc(version, release).ok_or_else(|| ctx.fail(&format!("Invalid Version: {version}")))?;
        }
        update_configs(ctx, &new_version, manifest);
    } else {
        ctx.checkpoint(
            "skip version bump on first release",
            &[],
            Some(ctx.pc.red(figures().cross)),
        );
    }
    ctx.lifecycle("postbump")?;
    Ok(Some(new_version))
}

/// conventional-recommended-bump with commit-and-tag-version's `legacyWhatBump`.
fn what_bump(ctx: &Ctx<'_>, version: &str) -> Result<Release, Failed> {
    let cwd = ctx.cwd;
    let from = git::semver_tags(cwd, &ctx.cfg.tag_prefix, true)
        .map_err(|e| ctx.fail(&e.0))?
        .into_iter()
        .next();
    let chunks = git::raw_commits(cwd, "%B%n-hash-%n%H", from.as_deref(), "HEAD", true).map_err(|e| ctx.fail(&e.0))?;
    let parser = CommitParser::new(ParserOptions {
        issue_prefixes: ctx.cfg.issue_prefixes.clone(),
        ..Default::default()
    });
    let commits: Vec<_> = chunks.iter().filter_map(|c| parser.parse(c)).collect();
    let commits = changelog::filter_reverted(
        &commits,
        |c| c.header.clone(),
        |c| c.field("hash").map(str::to_owned),
        |c| c.revert.clone(),
    );
    let mut level = 2u8;
    for c in &commits {
        if !c.notes.is_empty() {
            level = 0;
        } else if matches!(c.r#type.as_deref(), Some("feat" | "feature")) && level == 2 {
            level = 1;
        }
    }
    let pre_major = ctx.cfg.pre_major || semver::lt(version, "1.0.0").unwrap_or(false);
    if pre_major && level < 2 {
        level += 1;
    }
    Ok(Release::from_level(level))
}

fn is_git_ignored(cwd: &Path, file: &str) -> bool {
    git::output(cwd, &["check-ignore", "-q", "--", file]).is_ok_and(|o| o.status.success())
}

fn update_configs(ctx: &mut Ctx<'_>, new_version: &str, manifest: Option<&str>) {
    let cwd = ctx.cwd;
    // Cargo.toml
    if let Some(contents) = manifest {
        if is_git_ignored(cwd, "Cargo.toml") {
            println!("Not updating file 'Cargo.toml', as it is ignored in Git");
        } else if let (Some((kind, old)), Some(new_contents)) = (
            manifest_version(contents),
            write_manifest_version(contents, new_version),
        ) {
            let real = manifest_version(&new_contents).map(|(_, v)| v).unwrap_or_default();
            ctx.checkpoint("bumping version in Cargo.toml from %s to %s", &[&old, &real], None);
            write_file(ctx, &cwd.join("Cargo.toml"), &new_contents);
            ctx.updated.push("Cargo.toml".into());

            // Cargo.lock
            let lock_path = cwd.join("Cargo.lock");
            if lock_path.is_file() {
                if is_git_ignored(cwd, "Cargo.lock") {
                    println!("Not updating file 'Cargo.lock', as it is ignored in Git");
                    return;
                }
                let names = versioned_packages(cwd, contents, kind);
                let result = std::fs::read_to_string(&lock_path)
                    .map_err(|e| e.to_string())
                    .and_then(|lock| {
                        let old = lock_version(&lock, &names)?;
                        let new_lock = write_lock_version(&lock, &names, new_version)?;
                        let real = lock_version(&new_lock, &names)?;
                        Ok((old, new_lock, real))
                    });
                match result {
                    Ok((old, new_lock, real)) => {
                        ctx.checkpoint("bumping version in Cargo.lock from %s to %s", &[&old, &real], None);
                        write_file(ctx, &lock_path, &new_lock);
                        ctx.updated.push("Cargo.lock".into());
                    }
                    Err(e) => eprintln!("{e}"),
                }
            }
        } else {
            eprintln!("Cargo.toml: no version in [workspace.package] or [package]");
        }
    }
}

fn write_file(ctx: &Ctx<'_>, path: &PathBuf, contents: &str) {
    if ctx.args.dry_run {
        return;
    }
    if let Err(e) = std::fs::write(path, contents) {
        ctx.print_error(&format!("{}: {e}", path.display()), false);
    }
}

static START_OF_LAST_RELEASE: LazyLock<Regex> =
    LazyLock::new(|| js::re(&format!(r"(?:{}#+ \[?[0-9]+\.[0-9]+\.[0-9]+|<a name=)", js::ML_START)));

fn output_changelog(ctx: &Ctx<'_>, new_version: &str) -> Result<(), Failed> {
    let cfg = ctx.cfg;
    let infile = ctx.cwd.join(&cfg.infile);
    if !infile.exists() {
        ctx.checkpoint("created %s", &[&cfg.infile], None);
        write_file(ctx, &infile, "\n");
    }
    let old = if ctx.args.dry_run {
        String::new()
    } else {
        std::fs::read_to_string(&infile).map_err(|e| ctx.fail(&format!("{}: {e}", infile.display())))?
    };
    let old_body = match START_OF_LAST_RELEASE.find(&old).ok().flatten() {
        Some(m) => &old[m.start()..],
        None => &old[..],
    };
    let front_matter = old.find("# Changelog").map_or("", |i| &old[..i]);

    let content = Generator {
        cwd: ctx.cwd,
        cfg,
        new_version,
    }
    .write()
    .map_err(|e| ctx.fail(&e.0))?;

    ctx.checkpoint("outputting changes to %s", &[&cfg.infile], None);
    if ctx.args.dry_run {
        if !ctx.args.silent {
            println!("\n---\n{}\n---\n", ctx.pc.gray(js::trim(&content)));
        }
    } else {
        let tail = format!("{content}\n{old_body}");
        // `.replace(/\n+$/, '\n')`
        let tail = if tail.ends_with('\n') {
            format!("{}\n", tail.trim_end_matches('\n'))
        } else {
            tail
        };
        write_file(ctx, &infile, &format!("{front_matter}{}\n{tail}", cfg.header));
    }
    Ok(())
}

fn format_commit_message(format: &str, version: &str) -> String {
    format.replace("{{currentTag}}", version)
}

fn exec_commit(ctx: &Ctx<'_>, new_version: &str) -> Result<(), Failed> {
    let mut msg = "committing %s".to_owned();
    let mut paths: Vec<String> = Vec::new();
    let mut to_add: Vec<String> = Vec::new();
    if !ctx.args.skip_changelog {
        paths.push(ctx.cfg.infile.clone());
        to_add.push(ctx.cfg.infile.clone());
    }
    for p in &ctx.updated {
        paths.insert(0, p.clone());
        to_add.push(p.clone());
        if paths.len() > 1 {
            msg.push_str(" and %s");
        }
    }
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    ctx.checkpoint(&msg, &refs, None);
    if ctx.args.skip_changelog && ctx.args.skip_bump && to_add.is_empty() {
        return Ok(());
    }
    let mut add = vec!["add"];
    add.extend(to_add.iter().map(String::as_str));
    ctx.git(&add)?;
    let message = format_commit_message(&ctx.commit_message_format, new_version);
    let mut commit = vec!["commit"];
    if ctx.cfg.no_verify {
        commit.push("--no-verify");
    }
    if ctx.cfg.sign {
        commit.push("-S");
    }
    commit.extend(to_add.iter().map(String::as_str));
    commit.push("-m");
    commit.push(&message);
    ctx.git(&commit)
}

fn exec_tag(ctx: &Ctx<'_>, new_version: &str) -> Result<(), Failed> {
    let prefix = &ctx.cfg.tag_prefix;
    ctx.checkpoint("tagging release %s%s", &[prefix, new_version], None);
    let tag = format!("{prefix}{new_version}");
    let message = format_commit_message(&ctx.commit_message_format, new_version);
    ctx.git(&["tag", if ctx.cfg.sign { "-s" } else { "-a" }, &tag, "-m", &message])?;
    let branch = git::run(ctx.cwd, &["rev-parse", "--abbrev-ref", "HEAD"]).map_err(|e| ctx.fail(&e.0))?;
    let message = format!("git push --follow-tags origin {}", js::trim(&branch));
    ctx.checkpoint("Run `%s` to publish", &[&message], Some(ctx.pc.blue(figures().info)));
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::as_conversions
)]
mod tests {
    use super::*;

    #[test]
    fn manifest_versions() {
        let m = "[workspace]\nmembers = [\"a\"]\n\n[workspace.package]\nedition = \"2021\"\nversion = \"1.2.3\"\n\n[workspace.dependencies]\nx = { version = \"1\" }\n";
        assert_eq!(manifest_version(m), Some((VersionSection::Workspace, "1.2.3".into())));
        let w = write_manifest_version(m, "1.3.0").unwrap();
        assert!(w.contains("version = \"1.3.0\"") && w.contains("x = { version = \"1\" }"));
        let p = "[package]\nname = \"x\"\nauthors = [\"a\"]\nversion = \"0.1.0\"\n";
        assert_eq!(manifest_version(p), Some((VersionSection::Package, "0.1.0".into())));
    }

    #[test]
    fn lock() {
        let lock = "[[package]]\nname = \"a\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"b\"\nversion = \"1.0.0\"\n";
        let names = vec!["a".to_owned(), "b".to_owned()];
        let new = write_lock_version(lock, &names, "1.1.0").unwrap();
        assert_eq!(lock_version(&new, &names).unwrap(), "1.1.0");
        assert!(!new.contains("1.0.0"));
    }

    #[test]
    fn formats() {
        assert_eq!(
            util_format("committing %s and %s", &["a".into(), "b".into()]),
            "committing a and b"
        );
    }
}
