//! Port of commitlint 21.2: `@commitlint/lint`, `is-ignored`, `rules`, `ensure` and `format`, with the
//! rules of `@commitlint/config-conventional` as the default configuration.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use fancy_regex::Regex;
use serde_json::{Value, json};

use crate::case::ensure_case;
use crate::color::Palette;
use crate::js::{self, DOT, ML_END, ML_START, W, WS};
use crate::parser::{Commit, CommitParser, ParserOptions};
use crate::semver;

pub const DEFAULT_HELP_URL: &str = "https://github.com/conventional-changelog/commitlint/#what-is-commitlint";

/// One configured rule, `[level, when, value]` as in a commitlint config.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleConfig {
    pub name: String,
    pub level: u8,
    pub when: Option<String>,
    pub value: Option<Value>,
}

/// The rules of @commitlint/config-conventional 21.2.4, in its order.
pub fn default_rules() -> Vec<RuleConfig> {
    let types = json!([
        "build", "chore", "ci", "docs", "feat", "fix", "perf", "refactor", "revert", "style", "test"
    ]);
    let r = |name: &str, level: u8, when: &str, value: Option<Value>| RuleConfig {
        name: name.to_owned(),
        level,
        when: Some(when.to_owned()),
        value,
    };
    vec![
        r("body-leading-blank", 1, "always", None),
        r("body-max-line-length", 2, "always", Some(json!(100))),
        r("footer-leading-blank", 1, "always", None),
        r("footer-max-line-length", 2, "always", Some(json!(100))),
        r("header-max-length", 2, "always", Some(json!(100))),
        r("header-trim", 2, "always", None),
        r(
            "subject-case",
            2,
            "never",
            Some(json!(["sentence-case", "start-case", "pascal-case", "upper-case"])),
        ),
        r("subject-empty", 2, "never", None),
        r("subject-full-stop", 2, "never", Some(json!("."))),
        r("type-case", 2, "always", Some(json!("lower-case"))),
        r("type-empty", 2, "never", None),
        r("type-enum", 2, "always", Some(types)),
    ]
}

pub const RULE_NAMES: &[&str] = &[
    "body-case",
    "body-empty",
    "body-full-stop",
    "body-leading-blank",
    "body-max-length",
    "body-max-line-length",
    "body-min-length",
    "breaking-change-exclamation-mark",
    "footer-empty",
    "footer-leading-blank",
    "footer-max-length",
    "footer-max-line-length",
    "footer-min-length",
    "header-case",
    "header-full-stop",
    "header-max-length",
    "header-min-length",
    "header-trim",
    "references-empty",
    "scope-case",
    "scope-delimiter-style",
    "scope-empty",
    "scope-enum",
    "scope-max-length",
    "scope-min-length",
    "signed-off-by",
    "subject-case",
    "subject-empty",
    "subject-exclamation-mark",
    "subject-full-stop",
    "subject-max-length",
    "subject-min-length",
    "trailer-exists",
    "type-case",
    "type-empty",
    "type-enum",
    "type-max-length",
    "type-min-length",
];

/// Lint options besides the rules.
#[derive(Debug, Clone)]
pub struct LintOptions {
    pub default_ignores: bool,
    /// Extra ignore patterns (regexes tested against the message, like commitlint `ignores` functions).
    pub ignores: Vec<Regex>,
    pub help_url: String,
    /// `core.commentChar` in `--edit` mode.
    pub comment_char: Option<String>,
}

impl Default for LintOptions {
    fn default() -> Self {
        LintOptions {
            default_ignores: true,
            ignores: Vec::new(),
            help_url: DEFAULT_HELP_URL.to_owned(),
            comment_char: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub level: u8,
    pub name: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LintResult {
    pub valid: bool,
    pub errors: Vec<Problem>,
    pub warnings: Vec<Problem>,
    pub input: String,
}

// ---------------------------------------------------------------- is-ignored

static WILDCARDS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    let nl = r"(?:\r?\n)";
    vec![
        // the `m` flag: `^`/`$` at JS line terminators
        js::re(&format!(
            r"{ML_START}((Merge pull request)|(Merge ({DOT}*?) into ({DOT}*?)|(Merge branch ({DOT}*?))){nl}*{ML_END})"
        )),
        js::re(&format!(r"{ML_START}(Merge tag ({DOT}*?)){nl}*{ML_END}")),
        js::re(&format!(r"^(R|r)evert ({DOT}*)")),
        js::re(&format!(r"^(R|r)eapply ({DOT}*)")),
        js::re(r"^(amend|fixup|squash)!"),
        js::re(&format!(
            r"^(Merged ({DOT}*?)(in|into) ({DOT}*)|Merged PR ({DOT}*): ({DOT}*))"
        )),
        js::re(&format!(r"^Merge remote-tracking branch({WS}*)({DOT}*)")),
        js::re(&format!(r"^Automatic merge({DOT}*)")),
        js::re(&format!(r"^Auto-merged ({DOT}*?) into ({DOT}*)")),
    ]
});

static SEMVER_CHORE: LazyLock<Regex> = LazyLock::new(|| js::re(r"^chore(\([^)]+\))?:"));
static SKIP_CI_BRACKETS: LazyLock<Regex> = LazyLock::new(|| js::re(&format!(r"(?i)\[(skip|ci)(-|{WS})(ci|skip)\]")));
static SKIP_CI_PARENS: LazyLock<Regex> = LazyLock::new(|| js::re(&format!(r"(?i)\((skip|ci)(-|{WS})(ci|skip)\)")));

fn is_semver_message(c: &str) -> bool {
    let first = c.split('\n').next().unwrap_or("");
    let s = SEMVER_CHORE.replace(first, "");
    let s = SKIP_CI_BRACKETS.replace(&s, "");
    let s = SKIP_CI_PARENS.replace(&s, "");
    semver::valid(js::trim(&s)).is_some()
}

pub fn is_ignored(commit: &str, opts: &LintOptions) -> bool {
    if opts.default_ignores {
        for (i, re) in WILDCARDS.iter().enumerate() {
            if js::test(re, commit) {
                return true;
            }
            if i == 4 && is_semver_message(commit) {
                return true;
            }
        }
    }
    opts.ignores.iter().any(|re| js::test(re, commit))
}

// ---------------------------------------------------------------- rules

struct Parsed<'a> {
    commit: &'a Commit,
    raw: &'a str,
}

fn len16(s: &str) -> usize {
    js::len16(s)
}

fn message(parts: &[Option<&str>]) -> String {
    parts
        .iter()
        .flatten()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

fn truthy(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.is_empty())
}

fn num(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0)
}

/// `${value}` of a number the way JS prints it.
fn num_str(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        None => "0".to_owned(),
        Some(v) => v.to_string(),
    }
}

fn max_length(s: Option<&str>, max: f64) -> bool {
    s.is_some_and(|s| f64::from(u32::try_from(len16(s)).unwrap_or(u32::MAX)) <= max)
}

fn min_length(s: Option<&str>, min: f64) -> bool {
    s.is_some_and(|s| f64::from(u32::try_from(len16(s)).unwrap_or(u32::MAX)) >= min)
}

static URL_IN_LINE: LazyLock<Regex> = LazyLock::new(|| {
    js::re(&format!(
        r"{}https?://[^{}]+",
        crate::parser::JS_B,
        &WS[1..WS.len() - 1]
    ))
});

fn max_line_length(s: &str, max: f64) -> bool {
    js::split_lines(s)
        .iter()
        .all(|l| js::test(&URL_IN_LINE, l) || max_length(Some(l), max))
}

fn str_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// Case checks: strings, or `{ when, case }` objects.
fn case_checks(value: Option<&Value>) -> Vec<(String, String)> {
    let items: Vec<&Value> = match value {
        Some(Value::Array(a)) => a.iter().collect(),
        Some(v) => vec![v],
        None => Vec::new(),
    };
    items
        .into_iter()
        .map(|v| match v {
            Value::String(s) => ("always".to_owned(), s.clone()),
            Value::Object(o) => (
                o.get("when").and_then(Value::as_str).unwrap_or("always").to_owned(),
                o.get("case").and_then(Value::as_str).unwrap_or("").to_owned(),
            ),
            other => ("always".to_owned(), other.to_string()),
        })
        .collect()
}

type RuleOutcome = Result<(bool, Option<String>), String>;

/// The shared shape of the `*-case` rules.
fn case_rule(subject: &str, input: &[&str], when: &str, value: Option<&Value>) -> RuleOutcome {
    let checks = case_checks(value);
    let negated = |w: &str| w == "never";
    let mut matches = Vec::new();
    for (w, c) in &checks {
        let mut r = true;
        for seg in input {
            if !ensure_case(seg, c).ok_or_else(|| format!("to-case: Unknown target case \"{c}\""))? {
                r = false;
                break;
            }
        }
        if if negated(w) { !r } else { r } {
            matches.push(c.clone());
        }
    }
    let result = !matches.is_empty();
    let reported: Vec<String> = if negated(when) && result {
        matches
    } else {
        checks.iter().map(|(_, c)| c.clone()).collect()
    };
    let list = reported.join(", ");
    Ok((
        if negated(when) { !result } else { result },
        Some(message(&[
            Some(&format!("{subject} must")),
            negated(when).then_some("not"),
            Some(&format!("be {list}")),
        ])),
    ))
}

static STARTS_WITH_CASED_LETTER: LazyLock<Regex> = LazyLock::new(|| js::re(r"(?i)^[\p{Ll}\p{Lu}\p{Lt}]"));
static STARTS_WITH_ASCII_LETTER: LazyLock<Regex> = LazyLock::new(|| js::re("(?i)^[a-z]"));
static BREAKING_HEADER: LazyLock<Regex> = LazyLock::new(|| js::re(&format!(r"^({W}*)(?:\(({DOT}*)\))?!: ({DOT}*)$")));
static BREAKING_FOOTER: LazyLock<Regex> = LazyLock::new(|| js::re(&format!("{ML_START}BREAKING[ -]CHANGE:")));
static CHERRY_PICK: LazyLock<Regex> = LazyLock::new(|| js::re(r"(?i)^\(cherry picked from commit [0-9a-f]{7,64}\)$"));
static SCOPE_DELIMS: LazyLock<Regex> = LazyLock::new(|| js::re("[^A-Za-z0-9-_]+"));

fn delimiter_regex(delims: &[String]) -> Regex {
    let pats: Vec<String> = delims
        .iter()
        .map(|d| if d == "," { ", ?".to_owned() } else { js::escape(d) })
        .collect();
    js::re(&pats.join("|"))
}

fn split_by<'a>(re: &Regex, s: &'a str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut last = 0;
    for m in re.find_iter(s).flatten() {
        if m.end() == m.start() {
            continue;
        }
        out.push(s.get(last..m.start()).unwrap_or(""));
        last = m.end();
    }
    out.push(s.get(last..).unwrap_or(""));
    out
}

fn run_rule(name: &str, p: &Parsed<'_>, when: Option<&str>, value: Option<&Value>) -> RuleOutcome {
    let c = p.commit;
    let header = c.header.as_deref();
    let body = c.body.as_deref();
    let footer = c.footer.as_deref();
    let ok = |b: bool, m: String| Ok((b, Some(m)));
    let pass = || Ok((true, None));
    let w = when.unwrap_or("always");
    let full_stop_value = value.and_then(Value::as_str).unwrap_or(".");
    match name {
        "body-case" => match truthy(body) {
            None => pass(),
            Some(b) => case_rule("body", &[b], w, value),
        },
        "body-empty" | "footer-empty" | "subject-empty" | "type-empty" => {
            let (field, v) = match name {
                "body-empty" => ("body", body),
                "footer-empty" => ("footer", footer),
                "subject-empty" => ("subject", c.subject.as_deref()),
                _ => ("type", c.r#type.as_deref()),
            };
            let negated = w == "never";
            let not_empty = !v.unwrap_or("").is_empty();
            ok(
                if negated { not_empty } else { !not_empty },
                message(&[
                    Some(field),
                    Some(if negated { "may not" } else { "must" }),
                    Some("be empty"),
                ]),
            )
        }
        "body-full-stop" => match truthy(body) {
            None => pass(),
            Some(b) => {
                let negated = w == "never";
                let has = last_unit_is(b, full_stop_value);
                ok(
                    if negated { !has } else { has },
                    message(&[
                        Some("body"),
                        Some(if negated { "may not" } else { "must" }),
                        Some("end with full stop"),
                    ]),
                )
            }
        },
        "body-leading-blank" | "footer-leading-blank" => {
            let (field, v) = if name == "body-leading-blank" {
                ("body", body)
            } else {
                ("footer", footer)
            };
            if truthy(v).is_none() {
                return pass();
            }
            let negated = when == Some("never");
            let raw_lines = js::split_lines(p.raw);
            let leading = if field == "body" {
                raw_lines.get(1).copied()
            } else {
                let first = js::split_lines(v.unwrap_or("")).first().copied().unwrap_or("");
                let offset = raw_lines
                    .iter()
                    .position(|l| *l == first)
                    .map_or(-1, |i| isize::try_from(i).unwrap_or(isize::MAX));
                js::slice_from(&raw_lines, offset - 1).first().copied()
            };
            let succeeds = leading == Some("");
            ok(
                if negated { !succeeds } else { succeeds },
                message(&[
                    Some(field),
                    Some(if negated { "may not" } else { "must" }),
                    Some("have leading blank line"),
                ]),
            )
        }
        "body-max-length" | "footer-max-length" | "scope-max-length" | "subject-max-length" | "type-max-length" => {
            let (field, v) = field_of(name, c);
            match truthy(v) {
                None => pass(),
                Some(s) => ok(
                    max_length(Some(s), num(value)),
                    format!("{field} must not be longer than {} characters", num_str(value)),
                ),
            }
        }
        "body-min-length" | "footer-min-length" | "scope-min-length" | "subject-min-length" | "type-min-length" => {
            let (field, v) = field_of(name, c);
            match truthy(v) {
                None => pass(),
                Some(s) => ok(
                    min_length(Some(s), num(value)),
                    format!("{field} must not be shorter than {} characters", num_str(value)),
                ),
            }
        }
        "body-max-line-length" | "footer-max-line-length" => {
            let (field, v) = if name.starts_with("body") {
                ("body's", body)
            } else {
                ("footer's", footer)
            };
            match truthy(v) {
                None => pass(),
                Some(s) => ok(
                    max_line_length(s, num(value)),
                    format!("{field} lines must not be longer than {} characters", num_str(value)),
                ),
            }
        }
        "breaking-change-exclamation-mark" => {
            if truthy(header).is_none() && truthy(footer).is_none() {
                return pass();
            }
            let excl = truthy(header).is_some_and(|h| js::test(&BREAKING_HEADER, h));
            let brk = truthy(footer).is_some_and(|f| js::test(&BREAKING_FOOTER, f));
            let negated = w == "never";
            let check = excl == brk;
            ok(
                if negated { !check } else { check },
                message(&[
                    Some("breaking changes"),
                    Some(if negated { "must not" } else { "must" }),
                    Some("have both an exclamation mark in the header"),
                    Some("and BREAKING CHANGE in the footer"),
                    Some("to identify a breaking change"),
                ]),
            )
        }
        "header-case" => match header {
            Some(h) if js::test(&STARTS_WITH_ASCII_LETTER, h) => case_rule("header", &[h], w, value),
            _ => pass(),
        },
        "header-full-stop" => {
            let negated = w == "never";
            let has = header.is_some_and(|h| last_unit_is(h, full_stop_value));
            ok(
                if negated { !has } else { has },
                message(&[
                    Some("header"),
                    Some(if negated { "may not" } else { "must" }),
                    Some("end with full stop"),
                ]),
            )
        }
        "header-max-length" | "header-min-length" => {
            let len = header.map_or_else(|| "undefined".to_owned(), |h| len16(h).to_string());
            if name == "header-max-length" {
                ok(
                    max_length(header, num(value)),
                    format!(
                        "header must not be longer than {} characters, current length is {len}",
                        num_str(value)
                    ),
                )
            } else {
                ok(
                    min_length(header, num(value)),
                    format!(
                        "header must not be shorter than {} characters, current length is {len}",
                        num_str(value)
                    ),
                )
            }
        }
        "header-trim" => {
            let Some(h) = truthy(header) else { return pass() };
            let starts = js::trim_start(h).len() < h.len();
            let ends = js::trim_end(h).len() < h.len();
            match (starts, ends) {
                (true, true) => ok(false, "header must not be surrounded by whitespace".into()),
                (true, false) => ok(false, "header must not start with whitespace".into()),
                (false, true) => ok(false, "header must not end with whitespace".into()),
                _ => pass(),
            }
        }
        "references-empty" => {
            let negated = when.unwrap_or("never") == "always";
            let not_empty = !c.references.is_empty();
            ok(
                if negated { !not_empty } else { not_empty },
                message(&[
                    Some("references"),
                    Some(if negated { "must" } else { "may not" }),
                    Some("be empty"),
                ]),
            )
        }
        "scope-case" => {
            let Some(scope) = truthy(c.scope.as_deref()) else {
                return pass();
            };
            let (cases, delims) = match value {
                Some(Value::Object(o)) => (o.get("cases").cloned(), str_list(o.get("delimiters"))),
                v => (v.cloned(), Vec::new()),
            };
            let delims = if delims.is_empty() {
                vec!["/".into(), "\\".into(), ",".into()]
            } else {
                delims
            };
            let re = delimiter_regex(&delims);
            let segs: Vec<&str> = split_by(&re, scope).into_iter().collect();
            // segments matching the delimiter regex itself count as valid
            let checked: Vec<&str> = segs.into_iter().filter(|s| !js::test(&re, s)).collect();
            case_rule("scope", &checked, w, cases.as_ref())
        }
        "scope-delimiter-style" => {
            let Some(scope) = truthy(c.scope.as_deref()) else {
                return pass();
            };
            let delims = str_list(value);
            let delims = if delims.is_empty() {
                vec!["/".into(), "\\".into(), ",".into()]
            } else {
                delims
            };
            let mut found: Vec<String> = Vec::new();
            for m in SCOPE_DELIMS.find_iter(scope).flatten() {
                let d = if js::trim(m.as_str()) == "," {
                    ",".to_owned()
                } else {
                    m.as_str().to_owned()
                };
                if !found.contains(&d) {
                    found.push(d);
                }
            }
            let all = found.iter().all(|d| delims.contains(d));
            let never = w == "never";
            ok(
                if never { !all } else { all },
                format!(
                    "scope delimiters must {}be one of [{}]",
                    if never { "not " } else { "" },
                    delims.join(", ")
                ),
            )
        }
        "scope-empty" => {
            let negated = when.unwrap_or("never") == "always";
            let not_empty = !c.scope.as_deref().unwrap_or("").is_empty();
            ok(
                if negated { !not_empty } else { not_empty },
                message(&[
                    Some("scope"),
                    Some(if negated { "must" } else { "may not" }),
                    Some("be empty"),
                ]),
            )
        }
        "scope-enum" => {
            let (scopes, delims) = match value {
                Some(Value::Object(o)) => (str_list(o.get("scopes")), str_list(o.get("delimiters"))),
                v => (str_list(v), Vec::new()),
            };
            let Some(scope) = truthy(c.scope.as_deref()) else {
                return Ok((true, Some(String::new())));
            };
            if scopes.is_empty() {
                return Ok((true, Some(String::new())));
            }
            let delims = if delims.is_empty() {
                vec!["/".into(), "\\".into(), ",".into()]
            } else {
                delims
            };
            let parts = split_by(&delimiter_regex(&delims), scope);
            let in_enum = |s: &str| scopes.iter().any(|x| x == s);
            let list = format!("be one of [{}]", scopes.join(", "));
            if w == "never" {
                let valid = !parts.iter().any(|s| in_enum(s)) && !in_enum(scope);
                ok(valid, message(&[Some("scope must"), Some("not"), Some(&list)]))
            } else {
                let valid = parts.iter().all(|s| in_enum(s)) || in_enum(scope);
                ok(valid, message(&[Some("scope must"), Some(&list)]))
            }
        }
        "signed-off-by" | "trailer-exists" => {
            let v = value.and_then(Value::as_str).unwrap_or("");
            let negated = w == "never";
            let has = if name == "signed-off-by" {
                let lines: Vec<&str> = js::split_lines(p.raw)
                    .into_iter()
                    .filter(|l| !l.starts_with('#') && !js::test(&CHERRY_PICK, js::trim(l)) && !l.is_empty())
                    .collect();
                lines.last().is_some_and(|l| l.starts_with(v))
            } else {
                interpret_trailers(p.raw).iter().filter(|l| l.starts_with(v)).count() > 0
            };
            let msg = if name == "signed-off-by" {
                message(&[
                    Some("message"),
                    Some(if negated { "must not" } else { "must" }),
                    Some("be signed off"),
                ])
            } else {
                message(&[
                    Some("message"),
                    Some(if negated { "must not" } else { "must" }),
                    Some(&format!("have `{v}` trailer")),
                ])
            };
            ok(if negated { !has } else { has }, msg)
        }
        "subject-case" => match c.subject.as_deref() {
            Some(s) if js::test(&STARTS_WITH_CASED_LETTER, s) => case_rule("subject", &[s], w, value),
            _ => pass(),
        },
        "subject-exclamation-mark" => {
            let Some(h) = truthy(header) else {
                return Ok((true, Some(String::new())));
            };
            let negated = w == "never";
            let has = js::test(&BREAKING_HEADER, h);
            ok(
                if negated { !has } else { has },
                message(&[
                    Some("subject"),
                    Some(if negated { "must not" } else { "must" }),
                    Some("have an exclamation mark in the subject to identify a breaking change"),
                ]),
            )
        }
        "subject-full-stop" => {
            if let Some(h) = header {
                // `header.indexOf(":") || 0`, in UTF-16 units
                let colon = h.find(':').map(|i| len16(&h[..i]));
                if let Some(ci) = colon
                    && ci > 0
                    && ci == len16(h) - 1
                {
                    return pass();
                }
            }
            let negated = w == "never";
            let mut has = header.is_some_and(|h| last_unit_is(h, full_stop_value));
            if header.is_some_and(|h| h.ends_with("...")) {
                has = false;
            }
            ok(
                if negated { !has } else { has },
                message(&[
                    Some("subject"),
                    Some(if negated { "may not" } else { "must" }),
                    Some("end with full stop"),
                ]),
            )
        }
        "type-case" => match truthy(c.r#type.as_deref()) {
            None => pass(),
            Some(t) => case_rule("type", &[t], w, value),
        },
        "type-enum" => {
            let Some(t) = truthy(c.r#type.as_deref()) else {
                return pass();
            };
            let list = str_list(value);
            let negated = w == "never";
            let result = list.iter().any(|x| x == t);
            ok(
                if negated { !result } else { result },
                message(&[
                    Some("type must"),
                    negated.then_some("not"),
                    Some(&format!("be one of [{}]", list.join(", "))),
                ]),
            )
        }
        _ => Err(format!("Could not find rule implementation for {name}")),
    }
}

/// `input[input.length - 1] === value`: the last UTF-16 unit equals the (one-unit) value.
fn last_unit_is(s: &str, value: &str) -> bool {
    let last = s.encode_utf16().last();
    let mut v = value.encode_utf16();
    match (last, v.next(), v.next()) {
        (Some(l), Some(x), None) => l == x,
        _ => false,
    }
}

fn field_of<'a>(name: &str, c: &'a Commit) -> (&'static str, Option<&'a str>) {
    if name.starts_with("body") {
        ("body", c.body.as_deref())
    } else if name.starts_with("footer") {
        ("footer", c.footer.as_deref())
    } else if name.starts_with("scope") {
        ("scope", c.scope.as_deref())
    } else if name.starts_with("subject") {
        ("subject", c.subject.as_deref())
    } else {
        ("type", c.r#type.as_deref())
    }
}

fn interpret_trailers(raw: &str) -> Vec<String> {
    let child = Command::new("git")
        .args(["interpret-trailers", "--parse"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return Vec::new() };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(raw.as_bytes());
    }
    let out = child
        .wait_with_output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    js::split_lines(&out).into_iter().map(str::to_owned).collect()
}

/// Validate the rule configs like `lint()` does before running them.
pub fn validate_rules(rules: &[RuleConfig]) -> Result<(), String> {
    let missing: Vec<&str> = rules
        .iter()
        .map(|r| r.name.as_str())
        .filter(|n| !RULE_NAMES.contains(n))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "Found rules without implementation: {}.\nSupported rules are: {}.",
            missing.join(", "),
            RULE_NAMES.join(", ")
        ));
    }
    let mut errors = Vec::new();
    for r in rules {
        if r.level == 0 && r.when.is_none() {
            continue;
        }
        if r.level > 2 {
            errors.push(format!(
                "level for rule {} must be between 0 and 2, received {}",
                r.name, r.level
            ));
            continue;
        }
        match r.when.as_deref() {
            None => errors.push(format!("config for rule {} must be 2 or 3 items long", r.name)),
            Some("always" | "never") => {}
            Some(w) => errors.push(format!(
                "condition for rule {} must be \"always\" or \"never\", received '{w}'",
                r.name
            )),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

/// `buildCommitMessage`.
fn build_commit_message(c: &Commit) -> String {
    let mut m = c.header.clone().unwrap_or_default();
    let header_missing = c.header.is_none();
    if let Some(b) = truthy(c.body.as_deref()) {
        m = format!("{}\n\n{b}", if header_missing { "null" } else { &m });
    }
    if let Some(f) = truthy(c.footer.as_deref()) {
        let base = if header_missing && truthy(c.body.as_deref()).is_none() {
            "null".to_owned()
        } else {
            m
        };
        m = format!("{base}\n\n{f}");
    }
    m
}

/// commitlint's parser options (`fieldPattern: null`), compiled once per comment char.
fn lint_parser(comment_char: Option<&str>) -> std::sync::Arc<CommitParser> {
    use std::sync::{Arc, Mutex};
    static CACHE: Mutex<Vec<(Option<String>, Arc<CommitParser>)>> = Mutex::new(Vec::new());
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, p)) = cache.iter().find(|(c, _)| c.as_deref() == comment_char) {
        return Arc::clone(p);
    }
    let p = Arc::new(CommitParser::new(ParserOptions {
        field_pattern: false,
        comment_char: comment_char.map(str::to_owned),
        ..ParserOptions::default()
    }));
    cache.push((comment_char.map(str::to_owned), Arc::clone(&p)));
    p
}

/// `lint(message, rules, opts)`.
pub fn lint(message: &str, rules: &[RuleConfig], opts: &LintOptions) -> Result<LintResult, String> {
    let valid_result = || LintResult {
        valid: true,
        errors: Vec::new(),
        warnings: Vec::new(),
        input: message.to_owned(),
    };
    if is_ignored(js::trim_end(message), opts) {
        return Ok(valid_result());
    }
    let parser = lint_parser(opts.comment_char.as_deref());
    let commit = if message.is_empty() {
        Commit::default()
    } else {
        parser
            .parse(message)
            .ok_or_else(|| "TypeError: Expected a raw commit".to_owned())?
    };
    if commit.header.is_none() && commit.body.is_none() && commit.footer.is_none() {
        return Ok(valid_result());
    }
    validate_rules(rules)?;
    let parsed = Parsed {
        commit: &commit,
        raw: message,
    };
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for r in rules.iter().filter(|r| r.level > 0) {
        let (valid, msg) = run_rule(&r.name, &parsed, r.when.as_deref(), r.value.as_ref())?;
        if !valid {
            let p = Problem {
                level: r.level,
                name: r.name.clone(),
                message: msg.unwrap_or_else(|| "undefined".to_owned()),
            };
            if r.level == 2 { errors.push(p) } else { warnings.push(p) }
        }
    }
    Ok(LintResult {
        valid: errors.is_empty(),
        errors,
        warnings,
        input: build_commit_message(&commit),
    })
}

// ---------------------------------------------------------------- format

pub struct FormatOptions<'a> {
    pub verbose: bool,
    pub help_url: Option<&'a str>,
    pub colors: Palette,
}

fn format_input(r: &LintResult, o: &FormatOptions<'_>) -> Vec<String> {
    let pc = o.colors;
    if r.input.is_empty() {
        return vec![String::new()];
    }
    let has_problems = !r.errors.is_empty() || !r.warnings.is_empty();
    if !(o.verbose || has_problems) {
        return Vec::new();
    }
    vec![format!("{}   --- input ---\n{}", pc.gray("⧗"), pc.bold(&r.input))]
}

fn format_result(r: &LintResult, o: &FormatOptions<'_>) -> Vec<String> {
    let pc = o.colors;
    let problems: Vec<String> = r
        .errors
        .iter()
        .chain(&r.warnings)
        .map(|p| {
            let deco = match p.level {
                2 => pc.red("✖"),
                1 => pc.yellow("⚠"),
                _ => pc.white(" "),
            };
            format!("{deco}   {} {}", p.message, pc.gray(&format!("[{}]", p.name)))
        })
        .collect();
    let (sign, color): (&str, fn(&Palette, &str) -> String) = if !r.errors.is_empty() {
        ("✖", Palette::red)
    } else if !r.warnings.is_empty() {
        ("⚠", Palette::yellow)
    } else {
        ("✔", Palette::green)
    };
    let has_problems = !problems.is_empty();
    let mut out = problems;
    if has_problems {
        out.push(String::new());
    }
    if o.verbose || has_problems {
        let summary = format!(
            "{}   found {} problems, {} warnings",
            color(&pc, sign),
            r.errors.len(),
            r.warnings.len()
        );
        out.push(pc.bold(&summary));
    }
    if has_problems && let Some(url) = o.help_url.filter(|u| !u.is_empty()) {
        out.push(format!("ⓘ   Get help: {url}"));
    }
    if has_problems {
        out.push(String::new());
    }
    out
}

/// `format(report, options)` of @commitlint/format.
pub fn format(results: &[LintResult], o: &FormatOptions<'_>) -> String {
    results
        .iter()
        .flat_map(|r| {
            let mut v = format_input(r, o);
            v.extend(format_result(r, o));
            v
        })
        .collect::<Vec<_>>()
        .join("\n")
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

    fn run(msg: &str) -> LintResult {
        lint(msg, &default_rules(), &LintOptions::default()).unwrap()
    }

    #[test]
    fn conventional() {
        assert!(run("feat: add a thing").valid);
        let r = run("Add a thing");
        let names: Vec<_> = r.errors.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["subject-empty", "type-empty"]);
        let r = run("feat: Add a thing.");
        let names: Vec<_> = r.errors.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["subject-case", "subject-full-stop"]);
    }

    #[test]
    fn ignores() {
        let o = LintOptions::default();
        assert!(is_ignored("Merge branch 'x' into main", &o));
        assert!(is_ignored("chore(release): 1.2.3", &o));
        assert!(is_ignored("fixup! feat: x", &o));
        assert!(!is_ignored("feat: x", &o));
    }
}
