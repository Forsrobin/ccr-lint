//! Configuration: built-in defaults reproduce commitlint with `@commitlint/config-conventional` and
//! commit-and-tag-version with the conventionalcommits preset; `ccr-lint.toml` (or
//! `[workspace.metadata.ccr-lint]` / `[package.metadata.ccr-lint]` in Cargo.toml) overrides them.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::lint::{self, RuleConfig};

pub const DEFAULT_HEADER: &str = "# Changelog\n\nAll notable changes to this project will be documented in this file. See [commit-and-tag-version](https://github.com/absolute-version/commit-and-tag-version) for commit guidelines.\n";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitType {
    pub r#type: String,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub scope: Option<String>,
}

fn ty(t: &str, section: Option<&str>) -> CommitType {
    CommitType {
        r#type: t.into(),
        section: section.map(Into::into),
        hidden: section.is_none(),
        scope: None,
    }
}

pub fn default_types() -> Vec<CommitType> {
    vec![
        ty("feat", Some("Features")),
        ty("fix", Some("Bug Fixes")),
        ty("perf", Some("Performance")),
        ty("revert", Some("Reverts")),
        ty("refactor", None),
        ty("docs", None),
        ty("style", None),
        ty("test", None),
        ty("build", None),
        ty("ci", None),
        ty("chore", None),
    ]
}

#[derive(Debug, Clone)]
pub struct ReleaseConfig {
    pub tag_prefix: String,
    pub header: String,
    pub infile: String,
    pub release_commit_message_format: String,
    pub types: Vec<CommitType>,
    pub issue_prefixes: Vec<String>,
    pub commit_url_format: String,
    pub compare_url_format: String,
    pub issue_url_format: String,
    pub user_url_format: String,
    pub repository_url: Option<String>,
    pub pre_major: bool,
    pub scripts: BTreeMap<String, String>,
    pub sign: bool,
    pub no_verify: bool,
}

impl Default for ReleaseConfig {
    fn default() -> Self {
        ReleaseConfig {
            tag_prefix: "v".into(),
            header: DEFAULT_HEADER.into(),
            infile: "CHANGELOG.md".into(),
            release_commit_message_format: "chore(release): {{currentTag}}".into(),
            types: default_types(),
            issue_prefixes: vec!["#".into()],
            commit_url_format: "{{host}}/{{owner}}/{{repository}}/commit/{{hash}}".into(),
            compare_url_format: "{{host}}/{{owner}}/{{repository}}/compare/{{previousTag}}...{{currentTag}}".into(),
            issue_url_format: "{{host}}/{{owner}}/{{repository}}/issues/{{id}}".into(),
            user_url_format: "{{host}}/{{user}}".into(),
            repository_url: None,
            pre_major: false,
            scripts: BTreeMap::new(),
            sign: false,
            no_verify: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LintConfig {
    pub rules: Vec<RuleConfig>,
    pub default_ignores: bool,
    pub ignores: Vec<String>,
    pub help_url: String,
}

impl Default for LintConfig {
    fn default() -> Self {
        LintConfig {
            rules: lint::default_rules(),
            default_ignores: true,
            ignores: Vec::new(),
            help_url: lint::DEFAULT_HELP_URL.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub lint: LintConfig,
    pub release: ReleaseConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct FileConfig {
    #[serde(default)]
    lint: Option<FileLint>,
    #[serde(default)]
    release: Option<FileRelease>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct FileLint {
    rules: Option<toml::Table>,
    default_ignores: Option<bool>,
    ignores: Option<Vec<String>>,
    help_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct FileRelease {
    tag_prefix: Option<String>,
    header: Option<String>,
    infile: Option<String>,
    release_commit_message_format: Option<String>,
    types: Option<Vec<CommitType>>,
    issue_prefixes: Option<Vec<String>>,
    commit_url_format: Option<String>,
    compare_url_format: Option<String>,
    issue_url_format: Option<String>,
    user_url_format: Option<String>,
    repository_url: Option<String>,
    pre_major: Option<bool>,
    scripts: Option<BTreeMap<String, String>>,
    sign: Option<bool>,
    no_verify: Option<bool>,
}

pub const SCRIPT_HOOKS: &[&str] = &[
    "prerelease",
    "prebump",
    "postbump",
    "prechangelog",
    "postchangelog",
    "precommit",
    "postcommit",
    "pretag",
    "posttag",
];

fn toml_to_json(v: &toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(i) => Value::from(*i),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => Value::Object(t.iter().map(|(k, v)| (k.clone(), toml_to_json(v))).collect()),
    }
}

fn rule_from(name: &str, v: &toml::Value) -> Result<RuleConfig, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| format!("config for rule {name} must be array"))?;
    let level = arr
        .first()
        .and_then(toml::Value::as_integer)
        .and_then(|l| u8::try_from(l).ok())
        .ok_or_else(|| format!("level for rule {name} must be number"))?;
    if arr.len() > 3 {
        return Err(format!("config for rule {name} must be 1 to 3 items long"));
    }
    let when = match arr.get(1) {
        None => None,
        Some(w) => Some(
            w.as_str()
                .ok_or_else(|| format!("condition for rule {name} must be string"))?
                .to_owned(),
        ),
    };
    Ok(RuleConfig {
        name: name.to_owned(),
        level,
        when,
        value: arr.get(2).map(toml_to_json),
    })
}

/// Merge rule overrides: an existing rule is replaced in place, a new one appended (commitlint `extends`).
fn merge_rules(base: &mut Vec<RuleConfig>, table: &toml::Table) -> Result<(), String> {
    for (name, v) in table {
        let r = rule_from(name, v)?;
        match base.iter_mut().find(|b| b.name == *name) {
            Some(slot) => *slot = r,
            None => base.push(r),
        }
    }
    lint::validate_rules(base)
}

impl Config {
    fn apply(&mut self, f: FileConfig) -> Result<(), String> {
        if let Some(l) = f.lint {
            if let Some(rules) = &l.rules {
                merge_rules(&mut self.lint.rules, rules)?;
            }
            if let Some(v) = l.default_ignores {
                self.lint.default_ignores = v;
            }
            if let Some(v) = l.ignores {
                for p in &v {
                    fancy_regex::Regex::new(p).map_err(|e| format!("invalid ignore pattern {p:?}: {e}"))?;
                }
                self.lint.ignores = v;
            }
            if let Some(v) = l.help_url {
                self.lint.help_url = v;
            }
        }
        if let Some(r) = f.release {
            let c = &mut self.release;
            macro_rules! set {
                ($($field:ident),*) => { $( if let Some(v) = r.$field { c.$field = v; } )* };
            }
            set!(
                tag_prefix,
                header,
                infile,
                release_commit_message_format,
                types,
                issue_prefixes,
                commit_url_format,
                compare_url_format,
                issue_url_format,
                user_url_format,
                pre_major,
                scripts,
                sign,
                no_verify
            );
            if r.repository_url.is_some() {
                c.repository_url = r.repository_url;
            }
            if let Some(bad) = c.scripts.keys().find(|k| !SCRIPT_HOOKS.contains(&k.as_str())) {
                return Err(format!(
                    "unknown lifecycle script {bad:?}; expected one of {}",
                    SCRIPT_HOOKS.join(", ")
                ));
            }
            static START_OF_LAST_RELEASE: std::sync::LazyLock<fancy_regex::Regex> = std::sync::LazyLock::new(|| {
                crate::js::re(&format!(
                    r"(?:{}#+ \[?[0-9]+\.[0-9]+\.[0-9]+|<a name=)",
                    crate::js::ML_START
                ))
            });
            if crate::js::test(&START_OF_LAST_RELEASE, &c.header) {
                return Err(
                    "custom changelog header must not match /(^#+ \\[?[0-9]+\\.[0-9]+\\.[0-9]+|<a name=)/m".into(),
                );
            }
        }
        Ok(())
    }

    /// Load the configuration for the project in `dir`: `--config` file, else `ccr-lint.toml`, else
    /// Cargo.toml metadata, else the defaults.
    pub fn load(dir: &Path, explicit: Option<&Path>) -> Result<Config, String> {
        let mut cfg = Config::default();
        let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
        let parse = |text: &str, p: &Path| -> Result<FileConfig, String> {
            toml::from_str(text).map_err(|e| format!("{}: {e}", p.display()))
        };
        if let Some(p) = explicit {
            let f = parse(&read(p)?, p)?;
            cfg.apply(f)?;
            return Ok(cfg);
        }
        let own = dir.join("ccr-lint.toml");
        if own.is_file() {
            let f = parse(&read(&own)?, &own)?;
            cfg.apply(f)?;
            return Ok(cfg);
        }
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file() {
            let doc: toml::Table =
                toml::from_str(&read(&manifest)?).map_err(|e| format!("{}: {e}", manifest.display()))?;
            for section in ["workspace", "package"] {
                if let Some(meta) = doc
                    .get(section)
                    .and_then(|s| s.get("metadata"))
                    .and_then(|m| m.get("ccr-lint"))
                {
                    let f: FileConfig = meta
                        .clone()
                        .try_into()
                        .map_err(|e| format!("{}: [{section}.metadata.ccr-lint]: {e}", manifest.display()))?;
                    cfg.apply(f)?;
                    return Ok(cfg);
                }
            }
        }
        Ok(cfg)
    }
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
    fn overrides_merge_in_place() {
        let mut cfg = Config::default();
        let f: FileConfig = toml::from_str(
            r#"
            [lint.rules]
            header-max-length = [2, "always", 72]
            scope-enum = [2, "always", ["api", "cli"]]
            [release]
            tag-prefix = ""
            [release.scripts]
            prerelease = "true"
            "#,
        )
        .unwrap();
        cfg.apply(f).unwrap();
        let hml = cfg
            .lint
            .rules
            .iter()
            .position(|r| r.name == "header-max-length")
            .unwrap();
        assert_eq!(hml, 4);
        assert_eq!(cfg.lint.rules.last().unwrap().name, "scope-enum");
        assert_eq!(cfg.release.tag_prefix, "");
    }
}
