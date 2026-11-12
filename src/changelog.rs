//! Port of the changelog pipeline commit-and-tag-version 13.3 runs: conventional-changelog 8.1
//! (`ConventionalChangelog`), conventional-changelog-writer 9.3, the conventionalcommits preset 10.4
//! (transform, templates) and @conventional-changelog/template 1.4 (elements, references formatter).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use fancy_regex::{Captures, Regex};
use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed};

use crate::config::{CommitType, ReleaseConfig};
use crate::git::{self, GitError};
use crate::js::{self, DOT, ML_END, ML_START, WS};
use crate::parser::{Commit, CommitParser, Note, ParserOptions, Reference};
use crate::semver;

// ---------------------------------------------------------------- localeCompare

static COLLATOR: LazyLock<Option<CollatorBorrowed<'static>>> =
    LazyLock::new(|| Collator::try_new(Default::default(), CollatorOptions::default()).ok());

/// `String#localeCompare` with Node's ICU root collation.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    match &*COLLATOR {
        Some(c) => c.compare(a, b),
        None => a.cmp(b),
    }
}

// ---------------------------------------------------------------- hosted-git-info

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostedInfo {
    pub url: String,
    pub r#type: String,
    pub host: String,
    pub owner: Option<String>,
    pub project: Option<String>,
}

fn host_type(domain: &str) -> &'static str {
    if domain.contains("github") {
        "github"
    } else if domain.contains("gitlab") {
        "gitlab"
    } else if domain.contains("bitbucket") {
        "bitbucket"
    } else if domain.contains("git.sr.ht") {
        "sourcehut"
    } else {
        ""
    }
}

fn type_host(t: &str) -> &'static str {
    match t {
        "github" => "https://github.com",
        "gitlab" => "https://gitlab.com",
        "bitbucket" => "https://bitbucket.org",
        "sourcehut" => "https://git.sr.ht",
        _ => "",
    }
}

fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn repository_url(t: &str, host: &str, owner: &str, project: &str, branch: Option<&str>) -> String {
    if host.is_empty() {
        return String::new();
    }
    let tree = match branch {
        Some(b) if !b.is_empty() => {
            if t == "bitbucket" {
                format!("/src/{}", encode_uri_component(b))
            } else {
                format!("/tree/{}", encode_uri_component(b))
            }
        }
        _ => String::new(),
    };
    format!("{host}/{owner}/{project}{tree}")
}

static HOSTED_RES: LazyLock<[Regex; 5]> = LazyLock::new(|| {
    [
        js::re(r"^(?:https://)?github\.com/([^/]+)/([^/.#]+)(?:/tree/[^/]+)?$"),
        js::re(&format!(
            r"^(?:(?:git\+)?ssh://(?:[^@]+@)?|[^@]+@)(?:www\.)?([^@:]+):([^/]+(?:/[^/]+)?)/([^/.#]+)(?:\.git)?(?:#({DOT}+))?"
        )),
        js::re(&format!(
            r"^(?:(?:git\+)?https|git)://(?:[^@]+@)?(?:www\.)?([^@/]+)/([^/]+(?:/[^/]+)*)/([^/.#]+)(?:\.git)?(?:#({DOT}+))?$"
        )),
        js::re(&format!(
            r"^(?:([^@:]+):)?(?:[^@:]+@|[^@:]*:[^@:]+@)?([^/#]+(?:/[^/]+)?)/([^/.#]+)(?:\.git)?(?:#({DOT}+))?"
        )),
        js::re(r"^[A-Za-z0-9_]+://([^/]+)(/[^#]+)?"),
    ]
});

/// `parseHostedGitUrl` of @simple-libs/hosted-git-info.
pub fn parse_hosted_git_url(input: &str) -> Option<HostedInfo> {
    let [gh, ssh, https, short, generic] = &*HOSTED_RES;
    let g = |c: &Captures<'_, str>, i: usize| c.get(i).map(|m| m.as_str().to_owned());
    if let Ok(Some(c)) = gh.captures(input) {
        return Some(HostedInfo {
            url: input.to_owned(),
            r#type: "github".into(),
            host: "https://github.com".into(),
            owner: g(&c, 1),
            project: g(&c, 2),
        });
    }
    let build = |c: &Captures<'_, str>, t: String, host: String| {
        let owner = g(c, 2).unwrap_or_default();
        let project = g(c, 3).unwrap_or_default();
        HostedInfo {
            url: repository_url(&t, &host, &owner, &project, g(c, 4).as_deref()),
            r#type: t,
            host,
            owner: Some(owner),
            project: Some(project),
        }
    };
    if let Ok(Some(c)) = ssh.captures(input) {
        let domain = g(&c, 1).unwrap_or_default();
        return Some(build(&c, host_type(&domain).into(), format!("https://{domain}")));
    }
    if let Ok(Some(c)) = https.captures(input) {
        let domain = g(&c, 1).unwrap_or_default();
        let t = host_type(&domain);
        let s = if !t.is_empty() || input.contains("https://") {
            "s"
        } else {
            ""
        };
        return Some(build(&c, t.into(), format!("http{s}://{domain}")));
    }
    if let Ok(Some(c)) = short.captures(input) {
        let t = g(&c, 1).unwrap_or_else(|| "github".into());
        let host = type_host(&t).to_owned();
        return Some(build(&c, t, host));
    }
    if let Ok(Some(c)) = generic.captures(input) {
        let domain = g(&c, 1).unwrap_or_default();
        let t = host_type(&domain);
        let s = if !t.is_empty() || input.contains("https://") {
            "s"
        } else {
            ""
        };
        let host = format!("http{s}://{domain}");
        static GIT_SUFFIX: LazyLock<Regex> = LazyLock::new(|| js::re(&format!(r"/?\.git{DOT}*")));
        let path = g(&c, 2)
            .map(|p: String| GIT_SUFFIX.replace(&p, "").into_owned())
            .unwrap_or_default();
        return Some(HostedInfo {
            url: format!("{host}{path}"),
            r#type: t.into(),
            host,
            owner: None,
            project: None,
        });
    }
    None
}

/// Repository info as `ConventionalChangelog#readPackage` finds it: package.json `repository` (searched
/// upwards from `cwd`), else `git config remote.origin.url`.
pub fn read_repository(cwd: &Path, override_url: Option<&str>) -> Option<HostedInfo> {
    if let Some(url) = override_url {
        return parse_hosted_git_url(url);
    }
    let mut pkg_url: Option<String> = None;
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let p = d.join("package.json");
        if p.is_file() {
            if let Ok(text) = std::fs::read_to_string(&p)
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
            {
                pkg_url = match json.get("repository") {
                    Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s.clone()),
                    Some(serde_json::Value::Object(o)) => o.get("url").and_then(|u| u.as_str()).map(str::to_owned),
                    _ => None,
                }
                .filter(|s| !s.is_empty())
                // normalizeRepository replaces the url with the hosted one
                .map(|u| parse_hosted_git_url(&u).map_or(u, |h| h.url));
            }
            break;
        }
        dir = d.parent();
    }
    let url = pkg_url.or_else(|| git::config_get(cwd, "remote.origin.url"))?;
    parse_hosted_git_url(&url)
}

// ---------------------------------------------------------------- template elements

/// `each(array, callback, separator)`.
fn each<T>(items: &[T], f: impl Fn(&T) -> String, sep: &str) -> String {
    let mut acc = String::new();
    for item in items {
        let rendered = f(item);
        let rendered = js::trim(&rendered);
        if !acc.is_empty() && !rendered.is_empty() {
            acc.push_str(sep);
        }
        acc.push_str(rendered);
    }
    acc
}

fn join_values(values: &[String], sep: &str) -> String {
    each(values, Clone::clone, sep)
}

fn segments(values: &[String]) -> String {
    join_values(values, "\n\n")
}

fn strings(values: &[String]) -> String {
    join_values(values, "")
}

fn words(values: &[String]) -> String {
    join_values(values, " ")
}

fn heading(level: usize, text: &str) -> String {
    format!("{} {text}", "#".repeat(level))
}

fn link(text: &str, url: &str) -> String {
    format!("[{text}]({url})")
}

fn bold(text: &str) -> String {
    format!("**{text}**")
}

/// `list(array, callback)`.
fn list<T>(items: &[T], f: impl Fn(&T) -> String) -> String {
    each(
        items,
        |item| {
            let rendered = f(item);
            let rendered = js::trim(&rendered);
            if rendered.is_empty() {
                return String::new();
            }
            let text = js::split_lines(rendered)
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    if i > 0 && !l.is_empty() {
                        format!("  {l}")
                    } else {
                        (*l).to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("* {text}")
        },
        "\n",
    )
}

/// `expandTemplate` of commit-and-tag-version's preset loader.
fn expand_template(template: &str, vars: &HashMap<&str, String>) -> String {
    static VAR: LazyLock<Regex> = LazyLock::new(|| js::re(&format!(r"\{{\{{{WS}*([A-Za-z0-9_]+){WS}*\}}\}}")));
    VAR.replace_all(template, |c: &Captures<'_, str>| {
        c.get(1).and_then(|k| vars.get(k.as_str()).cloned()).unwrap_or_default()
    })
    .into_owned()
}

// ---------------------------------------------------------------- references formatter

static REFERENCES_FORMATTER: LazyLock<Regex> = LazyLock::new(|| {
    // the protected segments, then issue and user references; `gm` flags
    let ws_inner = &WS[1..WS.len() - 1];
    let patterns = [
        r"\[(?:[^\[\]]|\[[^\[\]]*\])*\]\((?:[^()\n]|\([^()\n]*\))*\)".to_owned(),
        r"\[[^\]]*\]\[[^\]]*\]".to_owned(),
        format!(
            r"{ML_START}[ \t]*(?=(?P<fence>`{{3,}}|~{{3,}}))\k<fence>(?s:.)*?{ML_START}[ \t]*\k<fence>[ \t]*\r?{ML_END}"
        ),
        r"(?=(?P<code>`+))\k<code>[^\n]*?\k<code>".to_owned(),
        format!(r"https?://(?:[^{ws_inner}()]|\([^{ws_inner}()]*\))+"),
        // prefixes and the issue pattern are filled in per formatter; see `references_regex`
    ];
    js::re(&patterns.join("|"))
});

fn references_regex(issue_prefixes: &[String]) -> Regex {
    let base = REFERENCES_FORMATTER.as_str();
    let prefixes = issue_prefixes
        .iter()
        .map(|p| js::escape(p))
        .collect::<Vec<_>>()
        .join("|");
    let nb = r"(?:(?<=[A-Za-z0-9_])(?=[A-Za-z0-9_])|(?<![A-Za-z0-9_])(?![A-Za-z0-9_]))";
    let mut parts = vec![base.to_owned()];
    if !prefixes.is_empty() {
        parts.push(format!("(?P<prefix>{prefixes})(?P<issue>[a-z0-9]+)"));
    }
    parts.push(format!(r"{nb}@(?P<user>[a-z0-9](?:-?[a-z0-9/]){{0,38}})"));
    js::re(&parts.join("|"))
}

pub struct Formatter {
    regex: Regex,
    commit_url: String,
    compare_url: String,
    issue_url: String,
    user_url: String,
}

impl Formatter {
    pub fn new(cfg: &ReleaseConfig) -> Self {
        Formatter {
            regex: references_regex(&cfg.issue_prefixes),
            commit_url: cfg.commit_url_format.clone(),
            compare_url: cfg.compare_url_format.clone(),
            issue_url: cfg.issue_url_format.clone(),
            user_url: cfg.user_url_format.clone(),
        }
    }

    /// The references formatter: links issues and users, collecting the linked issue texts.
    fn format(&self, text: &str, ctx: &Context, issues: &mut Vec<String>) -> String {
        let mut out = String::new();
        let mut last = 0;
        for caps in self.regex.captures_iter(text).flatten() {
            let Some(m) = caps.get(0) else { continue };
            out.push_str(text.get(last..m.start()).unwrap_or(""));
            last = m.end();
            let matched = m.as_str();
            if let Some(issue) = caps.name("issue") {
                let prefix = caps.name("prefix").map_or("", |p| p.as_str());
                let mut vars = ctx.vars();
                vars.insert("id", issue.as_str().to_owned());
                vars.insert("prefix", prefix.to_owned());
                let url = expand_template(&self.issue_url, &vars);
                if url.is_empty() {
                    out.push_str(matched);
                } else {
                    issues.push(matched.to_owned());
                    out.push_str(&link(matched, &url));
                }
            } else if let Some(user) = caps.name("user") {
                if user.as_str().contains('/') {
                    out.push_str(matched);
                } else {
                    let mut vars = ctx.vars();
                    vars.insert("user", user.as_str().to_owned());
                    let url = expand_template(&self.user_url, &vars);
                    if url.is_empty() {
                        out.push_str(matched);
                    } else {
                        out.push_str(&link(matched, &url));
                    }
                }
            } else {
                out.push_str(matched);
            }
        }
        out.push_str(text.get(last..).unwrap_or(""));
        out
    }
}

// ---------------------------------------------------------------- context and commits

/// The template context fields the preset reads.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub version: Option<String>,
    pub date: Option<String>,
    pub host: Option<String>,
    pub owner: Option<String>,
    pub repository: Option<String>,
    pub repo_url: Option<String>,
    pub link_references: bool,
    pub link_compare: bool,
    pub previous_tag: Option<String>,
    pub current_tag: Option<String>,
}

impl Context {
    fn vars(&self) -> HashMap<&'static str, String> {
        let mut v = HashMap::new();
        let mut put = |k: &'static str, val: &Option<String>| {
            if let Some(s) = val {
                v.insert(k, s.clone());
            }
        };
        put("version", &self.version);
        put("date", &self.date);
        put("host", &self.host);
        put("owner", &self.owner);
        put("repository", &self.repository);
        put("repoUrl", &self.repo_url);
        put("previousTag", &self.previous_tag);
        put("currentTag", &self.current_tag);
        v.insert("linkReferences", self.link_references.to_string());
        v.insert("linkCompare", self.link_compare.to_string());
        v.insert("commit", "commit".into());
        v.insert("issue", "issues".into());
        v
    }
}

/// A commit after conventional-changelog's transform (`version`, formatted `committerDate`).
#[derive(Debug, Clone)]
struct RawCommit {
    commit: Commit,
    version: Option<String>,
    committer_date: Option<String>,
}

/// A commit after the preset's transform.
#[derive(Debug, Clone)]
struct Entry {
    raw: RawCommit,
    group: Option<String>,
    scope: Option<String>,
    subject: Option<String>,
    short_hash: Option<String>,
    notes: Vec<Note>,
    references: Vec<Reference>,
}

fn hash_of(c: &Commit) -> Option<&str> {
    c.field("hash")
}

/// `new Date(%ci).toISOString().slice(0, 10)` for git's `YYYY-MM-DD HH:MM:SS +ZZZZ`.
pub fn format_git_date(ci: &str) -> Option<String> {
    let ci = js::trim(ci);
    let (date, rest) = ci.split_once(' ')?;
    let (time, tz) = rest.split_once(' ')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>());
    let (y, mo, da) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let mut t = time.split(':').map(|p| p.parse::<i64>());
    let (h, mi) = (t.next()?.ok()?, t.next()?.ok()?);
    let sign = if tz.starts_with('-') { -1 } else { 1 };
    let tzd = tz.trim_start_matches(['+', '-']);
    let tzh: i64 = tzd.get(0..2)?.parse().ok()?;
    let tzm: i64 = tzd.get(2..4)?.parse().ok()?;
    let minutes = h * 60 + mi - sign * (tzh * 60 + tzm);
    let days = days_from_civil(y, mo, da) + minutes.div_euclid(24 * 60);
    let (y, m, d) = civil_from_days(days);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Today's UTC date, `formatDate(new Date())`.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(0))
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// The prerelease channel: `None` for a stable version, `Some("")` for an unnamed prerelease.
fn prerelease_channel(version: &str) -> Option<String> {
    let v = semver::parse(version)?;
    match v.prerelease.first()? {
        semver::Ident::Str(s) => Some(s.clone()),
        semver::Ident::Num(_) => Some(String::new()),
    }
}

/// `isReleaseBoundary` of commit-and-tag-version's changelog lifecycle.
pub fn is_release_boundary(version: Option<&str>, new_version: &str) -> bool {
    let Some(v) = version.filter(|v| semver::valid(v).is_some()) else {
        return false;
    };
    let ch = prerelease_channel(v);
    ch.is_none() || ch == prerelease_channel(new_version)
}

/// `RevertedCommitsFilter`: drops reverted commits and the reverts that cancel them.
pub fn filter_reverted<T: Clone>(
    items: &[T],
    header: impl Fn(&T) -> Option<String>,
    hash: impl Fn(&T) -> Option<String>,
    revert: impl Fn(&T) -> Option<crate::parser::Revert>,
) -> Vec<T> {
    let trim = |s: Option<String>| s.map(|s| js::trim(&s).to_owned());
    let mut hold: Vec<(usize, T)> = Vec::new();
    let mut hold_reverts: i64 = 0;
    let mut out = Vec::new();
    let mut next_id = 0usize;
    for item in items {
        // findRevertCommit: the first held revert whose data matches this commit
        let found = hold.iter().position(|(_, h)| {
            revert(h).is_some_and(|r| {
                let header_ok = trim(r.header.clone()) == trim(header(item));
                let hash_ok = trim(r.hash.clone()) == trim(hash(item));
                header_ok && hash_ok
            })
        });
        if let Some(i) = found {
            hold.remove(i);
            hold_reverts -= 1;
            continue;
        }
        if revert(item).is_some() {
            hold.push((next_id, item.clone()));
            next_id += 1;
            hold_reverts += 1;
            continue;
        }
        if hold_reverts > 0 {
            hold.push((next_id, item.clone()));
            next_id += 1;
        } else {
            out.extend(hold.drain(..).map(|(_, h)| h));
            out.push(item.clone());
        }
    }
    out.extend(hold.into_iter().map(|(_, h)| h));
    out
}

// ---------------------------------------------------------------- preset transform

static RELEASE_AS: LazyLock<Regex> = LazyLock::new(|| {
    js::re(&format!(
        r"(?i)release-as:{WS}*[A-Za-z0-9_]*@?([0-9]+\.[0-9]+\.[0-9a-z]+(-[0-9a-z.]+)?){WS}*"
    ))
});
static BREAKING_HEADER: LazyLock<Regex> =
    LazyLock::new(|| js::re(&format!(r"^([A-Za-z0-9_]*)(?:\(({DOT}*)\))?!: ({DOT}*)$")));

pub fn note_title(title: &str) -> String {
    let up = title.to_uppercase();
    if up == "BREAKING CHANGE" || up == "BREAKING-CHANGE" {
        "BREAKING CHANGES".to_owned()
    } else {
        up
    }
}

/// `getNotes`: the notes, plus the `!` breaking change when no note is a breaking one.
pub fn get_notes(c: &Commit) -> Vec<Note> {
    let has_breaking = c.notes.iter().any(|n| note_title(&n.title) == "BREAKING CHANGES");
    if has_breaking || !js::test(&BREAKING_HEADER, c.header.as_deref().unwrap_or("")) {
        return c.notes.clone();
    }
    let mut notes = vec![Note {
        title: "BREAKING CHANGE".into(),
        text: c.subject.clone().unwrap_or_default(),
    }];
    notes.extend(c.notes.iter().cloned());
    notes
}

/// `findTypeEntry`.
pub fn find_type_entry<'a>(types: &'a [CommitType], c: &Commit) -> Option<&'a CommitType> {
    let key = if c.revert.is_some() {
        "revert".to_owned()
    } else {
        c.r#type.clone().unwrap_or_default().to_lowercase()
    };
    types
        .iter()
        .find(|t| t.r#type == key && t.scope.as_ref().is_none_or(|s| Some(s) == c.scope.as_ref()))
}

fn transform(raw: &RawCommit, cfg: &ReleaseConfig, fmt: &Formatter, ctx: &Context) -> Option<Entry> {
    let c = &raw.commit;
    let entry = find_type_entry(&cfg.types, c);
    let mut discard = !(c
        .footer
        .as_deref()
        .is_some_and(|f| !f.is_empty() && js::test(&RELEASE_AS, f))
        || c.body
            .as_deref()
            .is_some_and(|b| !b.is_empty() && js::test(&RELEASE_AS, b)));
    let notes: Vec<Note> = get_notes(c)
        .into_iter()
        .map(|n| {
            discard = false;
            Note {
                title: note_title(&n.title),
                text: fmt.format(&n.text, ctx, &mut Vec::new()),
            }
        })
        .collect();
    if discard && entry.is_none_or(|e| e.hidden) {
        return None;
    }
    let group = match entry {
        Some(e) => e.section.clone(),
        None => c.r#type.clone(),
    };
    let scope = if c.scope.as_deref() == Some("*") {
        Some(String::new())
    } else {
        c.scope.clone()
    };
    let short_hash = hash_of(c).map(|h| h.chars().take(7).collect());
    let mut issues = Vec::new();
    let subject = c.subject.as_deref().map(|s| fmt.format(s, ctx, &mut issues));
    let references = c
        .references
        .iter()
        .filter(|r| !issues.contains(&format!("{}{}", r.prefix, r.issue)))
        .cloned()
        .collect();
    Some(Entry {
        raw: raw.clone(),
        group,
        scope,
        subject,
        short_hash,
        notes,
        references,
    })
}

// ---------------------------------------------------------------- template

fn reference_text(r: &Reference) -> String {
    strings(&[
        r.owner
            .as_deref()
            .filter(|o| !o.is_empty())
            .map(|o| format!("{o}/"))
            .unwrap_or_default(),
        r.repository.clone().unwrap_or_default(),
        if r.issue.is_empty() {
            String::new()
        } else {
            format!("{}{}", if r.prefix.is_empty() { "#" } else { &r.prefix }, r.issue)
        },
    ])
}

fn render_references(refs: &[&Reference], ctx: &Context, fmt: &Formatter) -> String {
    each(
        refs,
        |r| {
            if ctx.link_references {
                let mut vars = ctx.vars();
                vars.insert("id", r.issue.clone());
                vars.insert("prefix", r.prefix.clone());
                link(&reference_text(r), &expand_template(&fmt.issue_url, &vars))
            } else {
                reference_text(r)
            }
        },
        " ",
    )
}

fn commit_partial(e: &Entry, ctx: &Context, fmt: &Formatter) -> String {
    let hash = hash_of(&e.raw.commit).filter(|h| !h.is_empty());
    let commit_link = match hash {
        Some(h) => {
            let short = e.short_hash.clone().unwrap_or_default();
            if ctx.link_references {
                let mut vars = ctx.vars();
                vars.insert("hash", h.to_owned());
                format!("({})", link(&short, &expand_template(&fmt.commit_url, &vars)))
            } else {
                short
            }
        }
        None => String::new(),
    };
    let closing: Vec<&Reference> = e.references.iter().filter(|r| r.action.is_some()).collect();
    let other: Vec<&Reference> = e.references.iter().filter(|r| r.action.is_none()).collect();
    let closing = render_references(&closing, ctx, fmt);
    let other = render_references(&other, ctx, fmt);
    let scope = e
        .scope
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| bold(&format!("{s}:")))
        .unwrap_or_default();
    let text = e
        .subject
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| e.raw.commit.header.clone())
        .unwrap_or_default();
    strings(&[
        words(&[scope, text, commit_link]),
        if closing.is_empty() {
            String::new()
        } else {
            format!(", closes {closing}")
        },
        if other.is_empty() {
            String::new()
        } else {
            format!(", references {other}")
        },
    ])
}

/// Group keys in `Object.entries` order: integer-like keys first (ascending), then insertion order.
fn object_key_order(keys: &mut [String]) {
    let index = |k: &str| -> Option<u64> {
        if k == "0" || (!k.starts_with('0') && !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit())) {
            k.parse::<u64>().ok().filter(|n| *n < 4_294_967_295)
        } else {
            None
        }
    };
    let mut ints: Vec<String> = keys.iter().filter(|k| index(k).is_some()).cloned().collect();
    ints.sort_by_key(|k| index(k));
    let rest: Vec<String> = keys.iter().filter(|k| index(k).is_none()).cloned().collect();
    for (slot, k) in keys.iter_mut().zip(ints.into_iter().chain(rest)) {
        *slot = k;
    }
}

fn render(entries: &[Entry], ctx: &Context, cfg: &ReleaseConfig, fmt: &Formatter) -> String {
    // ignoreReverted
    let filtered = filter_reverted(
        entries,
        |e| e.raw.commit.header.clone(),
        |e| hash_of(&e.raw.commit).map(str::to_owned),
        |e| e.raw.commit.revert.clone(),
    );

    // notes, grouped by title in order of appearance, sorted by title then (title, text)
    type NoteGroup = (String, Vec<(Note, Option<String>)>);
    let mut note_groups: Vec<NoteGroup> = Vec::new();
    for e in &filtered {
        for n in &e.notes {
            match note_groups.iter_mut().find(|(t, _)| *t == n.title) {
                Some((_, v)) => v.push((n.clone(), e.scope.clone())),
                None => note_groups.push((n.title.clone(), vec![(n.clone(), e.scope.clone())])),
            }
        }
    }
    note_groups.sort_by(|a, b| locale_compare(&a.0, &b.0));
    for (_, notes) in &mut note_groups {
        notes.sort_by(|(a, _), (b, _)| {
            locale_compare(&a.title, &b.title).then_with(|| locale_compare(&a.text, &b.text))
        });
    }

    // commit groups
    let mut keys: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<&Entry>> = HashMap::new();
    for e in &filtered {
        let key = e.group.clone().unwrap_or_default();
        if !groups.contains_key(&key) {
            keys.push(key.clone());
        }
        groups.entry(key).or_default().push(e);
    }
    object_key_order(&mut keys);
    let order: Vec<&str> = cfg
        .types
        .iter()
        .filter_map(|t| t.section.as_deref())
        .filter(|s| !s.is_empty())
        .collect();
    let rank = |t: &str| {
        order
            .iter()
            .position(|s| *s == t)
            .map_or(-1, |i| i64::try_from(i).unwrap_or(i64::MAX))
    };
    keys.sort_by_key(|k| rank(k));
    let sort_key = |e: &Entry| {
        format!(
            "{}{}",
            e.scope.as_deref().unwrap_or(""),
            e.subject.as_deref().unwrap_or("")
        )
    };

    let header = {
        let version = ctx.version.clone().unwrap_or_default();
        let version_text = if ctx.link_compare {
            link(&version, &expand_template(&fmt.compare_url, &ctx.vars()))
        } else {
            version
        };
        heading(
            2,
            &words(&[
                version_text,
                ctx.date.as_deref().map(|d| format!("({d})")).unwrap_or_default(),
            ]),
        )
    };
    let notes = each(
        &note_groups,
        |(title, notes)| {
            let icon = if title == "BREAKING CHANGES" { "⚠" } else { "" };
            segments(&[
                heading(3, &words(&[icon.to_owned(), title.clone()])),
                list(notes, |(n, scope)| {
                    words(&[
                        scope
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .map(|s| bold(&format!("{s}:")))
                            .unwrap_or_default(),
                        n.text.clone(),
                    ])
                }),
            ])
        },
        "\n\n",
    );
    let commits = each(
        &keys,
        |key| {
            let mut commits: Vec<&Entry> = groups.get(key).cloned().unwrap_or_default();
            commits.sort_by(|a, b| locale_compare(&sort_key(a), &sort_key(b)));
            segments(&[
                if key.is_empty() { String::new() } else { heading(3, key) },
                list(&commits, |e| commit_partial(e, ctx, fmt)),
            ])
        },
        "\n\n",
    );
    segments(&[header, String::new(), notes, commits, String::new()])
}

// ---------------------------------------------------------------- generator

pub struct Generator<'a> {
    pub cwd: &'a Path,
    pub cfg: &'a ReleaseConfig,
    pub new_version: &'a str,
}

impl Generator<'_> {
    /// `ConventionalChangelog#write()` collected into one string, as the changelog lifecycle does.
    pub fn write(&self) -> Result<String, GitError> {
        let cfg = self.cfg;
        let prefix = cfg.tag_prefix.as_str();
        let repo = read_repository(self.cwd, cfg.repository_url.as_deref());
        let host_kind = repo.as_ref().map(|r| r.r#type.as_str()).unwrap_or("");

        let semver_tags: Vec<String> = git::semver_tags(self.cwd, prefix, false)?
            .into_iter()
            .filter(|t| is_release_boundary(Some(&t.replacen(prefix, "", 1)), self.new_version))
            .collect();

        let mut base = Context {
            version: Some(self.new_version.to_owned()),
            date: Some(today()),
            ..Context::default()
        };
        if let Some(r) = &repo {
            base.repo_url = Some(r.url.clone()).filter(|s| !s.is_empty());
            base.host = Some(r.host.clone()).filter(|s| !s.is_empty());
            base.owner = r.owner.clone().filter(|s| !s.is_empty());
            base.repository = r.project.clone().filter(|s| !s.is_empty());
        }
        base.link_references = base.repository.is_some() || base.repo_url.is_some();

        let unreleased = semver_tags
            .first()
            .is_some_and(|t| *t == self.new_version || *t == format!("v{}", self.new_version));
        let do_flush = !unreleased;

        // getCommits: releaseCount 1
        let parser = CommitParser::new(ParserOptions {
            issue_prefixes: cfg.issue_prefixes.clone(),
            reference_actions: match host_kind {
                "gitlab" => [
                    "close", "closes", "closed", "closing", "fix", "fixes", "fixed", "fixing",
                ]
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
                "bitbucket" => [
                    "close",
                    "closes",
                    "closed",
                    "closing",
                    "fix",
                    "fixes",
                    "fixed",
                    "fixing",
                    "resolve",
                    "resolves",
                    "resolved",
                    "resolving",
                ]
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
                _ => crate::parser::default_reference_actions(),
            },
            ..ParserOptions::default()
        });
        let format = "%B%n-hash-%n%H%n-gitTags-%n%d%n-committerDate-%n%ci";
        let ranges: Vec<(Option<String>, String)> = if !git::verify(self.cwd, "HEAD") {
            vec![(semver_tags.first().cloned(), "HEAD".to_owned())]
        } else if let Some(from) = semver_tags.first() {
            vec![(Some(from.clone()), "HEAD".to_owned())]
        } else {
            vec![(None, "HEAD".to_owned())]
        };
        let mut raws = Vec::new();
        for (from, to) in &ranges {
            for chunk in git::raw_commits(self.cwd, format, from.as_deref(), to, false)? {
                if let Some(commit) = parser.parse(&chunk) {
                    raws.push(commit);
                }
            }
        }

        let version_re = js::re(&format!(r"(?i)tag:{WS}*[v=]?{}({DOT}*?)[,)]", js::escape(prefix)));
        let fmt = Formatter::new(cfg);

        let mut result = String::new();
        let mut first_release = true;
        let mut never_generated = true;
        let mut group: Vec<Entry> = Vec::new();
        let mut saved_key: Option<RawCommit> = None;
        let mut out = String::new();
        let render_group = |group: &[Entry], key: Option<&RawCommit>, subsequent: bool| -> String {
            let mut ctx = base.clone();
            if let Some(k) = key {
                if k.version.is_some() {
                    ctx.version = k.version.clone();
                }
                if let Some(d) = &k.committer_date {
                    ctx.date = Some(d.clone());
                }
            }
            self.finalize_context(&mut ctx, key, group, &semver_tags);
            let rendered = render(group, &ctx, cfg, &fmt);
            let rendered = js::trim(&rendered);
            if rendered.is_empty() {
                String::new()
            } else {
                format!("{}{rendered}\n", if subsequent { "\n" } else { "" })
            }
        };
        for commit in raws {
            let gitt = commit.field("gitTags").map(str::to_owned);
            let version = gitt
                .as_deref()
                .and_then(|g| version_re.captures(g).ok().flatten())
                .and_then(|c| c.get(1).map(|m| m.as_str().to_owned()));
            let committer_date = commit
                .field("committerDate")
                .filter(|d| !d.is_empty())
                .and_then(format_git_date);
            let raw = RawCommit {
                commit,
                version,
                committer_date,
            };
            let ctx_for_transform = base.clone();
            let entry = transform(&raw, cfg, &fmt, &ctx_for_transform);
            if is_release_boundary(raw.version.as_deref(), self.new_version) {
                never_generated = false;
                result = render_group(&group, saved_key.as_ref(), !result.is_empty());
                group.clear();
                if !first_release || do_flush {
                    out.push_str(&result);
                }
                first_release = false;
                saved_key = Some(raw.clone());
            }
            if let Some(e) = entry {
                group.push(e);
            }
        }
        if do_flush || !never_generated {
            let r = render_group(&group, saved_key.as_ref(), !result.is_empty());
            out.push_str(&r);
        }
        Ok(out)
    }

    /// `finalizeContext` of `ConventionalChangelog#finalizeWriterOptions`.
    fn finalize_context(&self, ctx: &mut Context, key: Option<&RawCommit>, group: &[Entry], semver_tags: &[String]) {
        static VERSION_TAG: LazyLock<Regex> = LazyLock::new(|| js::re(&format!(r"(?i)tag:{WS}*({DOT}*?)[,)]")));
        if let Some(k) = key {
            let m = k
                .commit
                .field("gitTags")
                .and_then(|g| VERSION_TAG.captures(g).ok().flatten())
                .and_then(|c| c.get(1).map(|m| m.as_str().to_owned()));
            ctx.current_tag = m.filter(|s| !s.is_empty());
            let index = ctx
                .current_tag
                .as_ref()
                .and_then(|t| semver_tags.iter().position(|s| s == t));
            match index {
                None => ctx.current_tag = None,
                Some(i) => {
                    ctx.previous_tag = semver_tags.get(i + 1).cloned().or_else(|| {
                        group
                            .last()
                            .and_then(|e| hash_of(&e.raw.commit).map(str::to_owned))
                            .filter(|h| !h.is_empty())
                    });
                }
            }
        } else {
            ctx.previous_tag = semver_tags.first().cloned();
            let prefix = &self.cfg.tag_prefix;
            let version = ctx.version.clone().unwrap_or_default();
            ctx.current_tag = Some(if prefix.is_empty() {
                guess_next_tag(semver_tags.first().map(String::as_str), &version)
            } else {
                format!("{prefix}{version}")
            });
        }
        ctx.link_compare = ctx.previous_tag.is_some() && ctx.current_tag.as_deref().is_some_and(|t| !t.is_empty());
    }
}

fn guess_next_tag(previous: Option<&str>, version: &str) -> String {
    match previous {
        Some(p) if !p.is_empty() => {
            if p.starts_with('v') && !version.starts_with('v') {
                format!("v{version}")
            } else if !p.starts_with('v') && version.starts_with('v') {
                version.trim_start_matches('v').to_owned()
            } else {
                version.to_owned()
            }
        }
        _ => {
            if version.starts_with('v') {
                version.to_owned()
            } else {
                format!("v{version}")
            }
        }
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
    fn hosted() {
        let h = parse_hosted_git_url("git@github.com:EcophiAI/analysis_cmpc.git").unwrap();
        assert_eq!(h.host, "https://github.com");
        assert_eq!(h.owner.as_deref(), Some("EcophiAI"));
        assert_eq!(h.project.as_deref(), Some("analysis_cmpc"));
        assert_eq!(h.url, "https://github.com/EcophiAI/analysis_cmpc");
        let h = parse_hosted_git_url("https://gitlab.example.com/group/sub/proj.git").unwrap();
        assert_eq!(h.r#type, "gitlab");
        assert_eq!(h.owner.as_deref(), Some("group/sub"));
    }

    #[test]
    fn dates() {
        assert_eq!(
            format_git_date("2026-11-12 00:30:00 +0100").as_deref(),
            Some("2026-11-11")
        );
        assert_eq!(
            format_git_date("2024-02-29 23:30:00 -0100").as_deref(),
            Some("2024-03-01")
        );
    }

    #[test]
    fn collation() {
        assert_eq!(locale_compare("bump", "CI fails"), Ordering::Less);
        assert_eq!(locale_compare("removed", "Resolve"), Ordering::Less);
        assert_eq!(locale_compare("a", "A"), Ordering::Less);
    }
}
