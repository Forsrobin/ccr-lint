//! Port of conventional-commits-parser 7.1.4 (`CommitParser`, `regex.ts`, `utils.ts`), with the parser
//! options of conventional-changelog-conventionalcommits 10.4.

use std::collections::{BTreeMap, HashSet};

use fancy_regex::Regex;

use crate::js::{self, D, DOT, W, WS};

const SCISSOR: &str = "------------------------ >8 ------------------------";

/// Parser options. `Default` is what commitlint and the changelog use with the conventionalcommits preset,
/// minus the per-use differences (`field_pattern`, `comment_char`, `reference_actions`).
#[derive(Debug, Clone)]
pub struct ParserOptions {
    pub note_keywords: Vec<String>,
    pub issue_prefixes: Vec<String>,
    pub issue_prefixes_case_sensitive: bool,
    pub reference_actions: Vec<String>,
    /// `fieldPattern` is on in the changelog and bump (`-hash-` markers), off in commitlint.
    pub field_pattern: bool,
    pub comment_char: Option<String>,
}

pub fn default_reference_actions() -> Vec<String> {
    [
        "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

impl Default for ParserOptions {
    fn default() -> Self {
        ParserOptions {
            note_keywords: vec!["BREAKING CHANGE".into(), "BREAKING-CHANGE".into()],
            issue_prefixes: vec!["#".into()],
            issue_prefixes_case_sensitive: false,
            reference_actions: default_reference_actions(),
            field_pattern: true,
            comment_char: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub raw: String,
    pub action: Option<String>,
    pub owner: Option<String>,
    pub repository: Option<String>,
    pub prefix: String,
    pub issue: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Revert {
    pub header: Option<String>,
    pub hash: Option<String>,
}

/// A parsed commit. String fields are `None` where JS has `null`/`undefined`; `Some("")` is kept apart
/// because the tools test truthiness in some places and `=== null` in others.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commit {
    pub merge: Option<String>,
    pub header: Option<String>,
    pub body: Option<String>,
    pub footer: Option<String>,
    pub r#type: Option<String>,
    pub scope: Option<String>,
    pub subject: Option<String>,
    pub notes: Vec<Note>,
    pub references: Vec<Reference>,
    pub revert: Option<Revert>,
    /// Values of `-field-` markers (`hash`, `gitTags`, `committerDate`, ...).
    pub fields: BTreeMap<String, String>,
}

impl Commit {
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    /// The string property a `-name-` marker writes to, if it is one of the parsed ones.
    fn field_mut(&mut self, name: &str) -> Option<&mut Option<String>> {
        match name {
            "merge" => Some(&mut self.merge),
            "header" => Some(&mut self.header),
            "body" => Some(&mut self.body),
            "footer" => Some(&mut self.footer),
            "type" => Some(&mut self.r#type),
            "scope" => Some(&mut self.scope),
            "subject" => Some(&mut self.subject),
            _ => None,
        }
    }
}

/// `trimNewLines`.
pub fn trim_new_lines(input: &str) -> String {
    let Some(first) = input.find(|c| c != '\r' && c != '\n') else {
        return String::new();
    };
    input[first..].trim_end_matches(['\r', '\n']).to_owned()
}

/// `appendLine`: a falsy source is replaced by the line.
fn append_line(src: Option<&str>, line: &str) -> String {
    match src {
        Some(s) if !s.is_empty() => format!("{s}\n{line}"),
        _ => line.to_owned(),
    }
}

fn append_to(slot: &mut Option<String>, line: &str) {
    *slot = Some(append_line(slot.as_deref(), line));
}

struct Regexes {
    header: Regex,
    breaking_header: Regex,
    notes: Regex,
    reference_parts: Regex,
    references: Regex,
    footer_references: Regex,
    footer_token: Regex,
    url: Regex,
    field: Regex,
    revert: Regex,
    gpg: Regex,
}

/// JS `\b` (ASCII word boundary).
pub const JS_B: &str = r"(?:(?<=[A-Za-z0-9_])(?![A-Za-z0-9_])|(?<![A-Za-z0-9_])(?=[A-Za-z0-9_]))";

fn regexes(opts: &ParserOptions) -> Regexes {
    let notes_sel = js::join_or(&opts.note_keywords);
    let prefixes = js::join_or(&opts.issue_prefixes);
    let kw = js::join_or(&opts.reference_actions);
    let repo_char = r"[A-Za-z0-9_\-./]";
    let ci = if opts.issue_prefixes_case_sensitive { "" } else { "(?i)" };
    let never = "(?!)";
    let notes = if opts.note_keywords.is_empty() {
        never.to_owned()
    } else {
        format!(r"(?i)^(?:\*{WS}+)?({notes_sel}):{WS}*({DOT}*)")
    };
    let reference_parts = if opts.issue_prefixes.is_empty() {
        never.to_owned()
    } else {
        format!(
            r"{ci}{DOT}*?(?<!{repo_char})(?:({repo_char}+)(?!{repo_char}))?({prefixes})((?=[A-Za-z0-9_-]*{D})[A-Za-z0-9_-]+)(?={WS}|$|[,;.)\]])"
        )
    };
    let references_with = |sep: &str| {
        if opts.reference_actions.is_empty() {
            format!("(?i)()({DOT}+)")
        } else {
            let next = format!(r"(?<!{W})(?:{kw})(?!{W})");
            format!(r"(?i)(?<!{W})({kw}){sep}(?:{WS}+({DOT}*?))(?={next}|$)")
        }
    };
    let issue_sep = if opts.issue_prefixes.is_empty() {
        String::new()
    } else {
        format!(r"|{WS}+(?:{prefixes})")
    };
    Regexes {
        header: js::re(&format!(r"^({W}*)(?:\(({DOT}*)\))?!?: ({DOT}*)$")),
        breaking_header: js::re(&format!(r"^({W}*)(?:\(({DOT}*)\))?!: ({DOT}*)$")),
        notes: js::re(&notes),
        reference_parts: js::re(&reference_parts),
        references: js::re(&references_with("")),
        footer_references: js::re(&references_with(":?")),
        footer_token: js::re(&format!(
            r"(?i)^(?:BREAKING CHANGE|[A-Za-z0-9_-]+)(?::{WS}+{issue_sep}){DOT}+"
        )),
        url: js::re(&format!(
            r"{JS_B}(?:https?)://(?:www\.)?([-a-zA-Z0-9@:%_+.~#?&/=])+{JS_B}"
        )),
        field: js::re(&format!(r"^-(?={DOT}*{W})({DOT}*?)-$")),
        revert: js::re(&format!(
            r#"(?i)^(?:Revert|revert:){WS}"?((?s:.)+?)"?{WS}*This reverts commit ({W}*)\."#
        )),
        gpg: js::re(&format!(r"^{WS}*gpg:")),
    }
}

pub struct CommitParser {
    opts: ParserOptions,
    re: Regexes,
}

struct State<'a> {
    lines: Vec<&'a str>,
    index: usize,
    commit: Commit,
}

impl<'a> State<'a> {
    fn current(&self) -> Option<&'a str> {
        self.lines.get(self.index).copied()
    }
    fn next(&mut self) -> Option<&'a str> {
        let line = self.current();
        self.index += 1;
        line
    }
    fn available(&self) -> bool {
        self.index < self.lines.len()
    }
}

fn group(caps: &fancy_regex::Captures<'_, str>, i: usize) -> Option<String> {
    caps.get(i).map(|m| m.as_str().to_owned()).filter(|s| !s.is_empty())
}

impl CommitParser {
    pub fn new(opts: ParserOptions) -> Self {
        let re = regexes(&opts);
        CommitParser { opts, re }
    }

    /// `parseReference` driven by a sticky regex: `pos` is `lastIndex`.
    fn parse_reference(&self, input: &str, action: Option<&str>, pos: &mut usize) -> Option<Reference> {
        // A reference needs an issue prefix; without one the regex cannot match (fast path).
        if !self.has_prefix(input) {
            *pos = 0;
            return None;
        }
        if js::test(&self.re.url, input) {
            return None;
        }
        let caps = match self.re.reference_parts.captures_from_pos(input, *pos) {
            Ok(Some(c)) if c.get(0).is_some_and(|m| m.start() == *pos) => c,
            _ => {
                *pos = 0;
                return None;
            }
        };
        let whole = caps.get(0)?;
        *pos = whole.end();
        let mut repository = caps.get(1).map(|m| m.as_str().to_owned());
        let mut owner = None;
        if let Some(repo) = repository.clone()
            && let Some(slash) = repo.find('/')
        {
            owner = Some(repo[..slash].to_owned());
            repository = Some(repo[slash + 1..].to_owned());
        }
        Some(Reference {
            raw: whole.as_str().to_owned(),
            action: action.map(str::to_owned),
            owner,
            repository,
            prefix: caps.get(2).map(|m| m.as_str().to_owned()).unwrap_or_default(),
            issue: caps.get(3).map(|m| m.as_str().to_owned()).unwrap_or_default(),
        })
    }

    fn has_prefix(&self, input: &str) -> bool {
        if self.opts.issue_prefixes.is_empty() {
            return false;
        }
        if self.opts.issue_prefixes_case_sensitive {
            self.opts.issue_prefixes.iter().any(|p| input.contains(js::trim(p)))
        } else {
            let lower = input.to_lowercase();
            self.opts
                .issue_prefixes
                .iter()
                .any(|p| lower.contains(&js::trim(p).to_lowercase()))
        }
    }

    pub fn parse_references(&self, input: &str, footer_token: bool) -> Vec<Reference> {
        if !self.has_prefix(input) {
            return Vec::new();
        }
        let regex = if footer_token {
            &self.re.footer_references
        } else {
            &self.re.references
        };
        let mut sentences: Vec<(Option<String>, &str)> = Vec::new();
        let mut last = 0;
        let lower = input.to_lowercase();
        let has_action = self.opts.reference_actions.is_empty()
            || self
                .opts
                .reference_actions
                .iter()
                .any(|a| lower.contains(&js::trim(a).to_lowercase()));
        for caps in regex.captures_iter(if has_action { input } else { "" }).flatten() {
            let Some(m) = caps.get(0) else { continue };
            sentences.push((None, input.get(last..m.start()).unwrap_or("")));
            sentences.push((group(&caps, 1), caps.get(2).map_or("", |g| g.as_str())));
            last = m.end();
        }
        sentences.push((None, input.get(last..).unwrap_or("")));
        let mut refs = Vec::new();
        for (action, sentence) in sentences {
            let mut pos = 0;
            while let Some(r) = self.parse_reference(sentence, action.as_deref(), &mut pos) {
                refs.push(r);
            }
        }
        refs
    }

    fn parse_meta(&self, st: &mut State<'_>) -> bool {
        if !self.opts.field_pattern || !st.available() {
            return false;
        }
        let mut field: Option<String> = None;
        let mut parsed = false;
        while let Some(line) = st.current() {
            if let Ok(Some(caps)) = self.re.field.captures(line) {
                field = group(&caps, 1);
                st.next();
                continue;
            }
            if let Some(name) = &field {
                parsed = true;
                append_field(&mut st.commit, name, line);
                st.next();
            } else {
                break;
            }
        }
        parsed
    }

    fn parse_notes(&self, st: &mut State<'_>) -> bool {
        let Some(line) = st.current() else { return false };
        let Ok(Some(caps)) = self.re.notes.captures(line) else {
            return false;
        };
        let idx = st.commit.notes.len();
        st.commit.notes.push(Note {
            title: caps.get(1).map(|m| m.as_str().to_owned()).unwrap_or_default(),
            text: caps.get(2).map(|m| m.as_str().to_owned()).unwrap_or_default(),
        });
        append_to(&mut st.commit.footer, line);
        st.next();
        while st.available() {
            if self.parse_meta(st) {
                return true;
            }
            if self.parse_notes(st) {
                return true;
            }
            // `parseMeta` can consume trailing `-field-` lines without a value, leaving `undefined`,
            // which JS tests as the string "undefined" and appends as ''.
            let line2 = st.current().unwrap_or("");
            let tok = js::test(&self.re.footer_token, line2);
            let refs = self.parse_references(line2, tok);
            st.commit.references.extend(refs);
            if !tok && let Some(note) = st.commit.notes.get_mut(idx) {
                note.text = append_line(Some(&note.text), line2);
            }
            append_to(&mut st.commit.footer, line2);
            st.next();
            if tok {
                break;
            }
        }
        true
    }

    fn parse_body_and_footer(&self, st: &mut State<'_>, is_body: bool) -> bool {
        let Some(line) = st.current() else { return is_body };
        let tok = js::test(&self.re.footer_token, line);
        let still = !tok && is_body;
        let refs = self.parse_references(line, tok);
        st.commit.references.extend(refs);
        if still {
            append_to(&mut st.commit.body, line);
        } else {
            append_to(&mut st.commit.footer, line);
        }
        st.next();
        still
    }

    /// `CommitParser#parse`; `None` where it throws (`input.trim()` empty).
    pub fn parse(&self, input: &str) -> Option<Commit> {
        if js::trim(input).is_empty() {
            return None;
        }
        let trimmed = trim_new_lines(input);
        let raw_lines = js::split_lines(&trimmed);
        let gpg_ok = |l: &&str| !js::test(&self.re.gpg, l);
        let lines: Vec<&str> = match self.opts.comment_char.as_deref().filter(|c| !c.is_empty()) {
            Some(cc) => {
                let scissor = format!("{cc} {SCISSOR}");
                let end = raw_lines.iter().position(|l| *l == scissor).unwrap_or(raw_lines.len());
                raw_lines
                    .iter()
                    .take(end)
                    .copied()
                    .filter(|l| !l.starts_with(cc))
                    .filter(gpg_ok)
                    .collect()
            }
            None => raw_lines.iter().copied().filter(gpg_ok).collect(),
        };
        let mut st = State {
            lines,
            index: 0,
            commit: Commit::default(),
        };

        // parseHeader (no merge pattern in this preset)
        let header = st.next();
        if let Some(h) = header.filter(|h| !h.is_empty()) {
            let caps = match self.re.breaking_header.captures(h) {
                Ok(Some(c)) => Some(c),
                _ => self.re.header.captures(h).ok().flatten(),
            };
            st.commit.header = Some(h.to_owned());
            if let Some(caps) = caps {
                st.commit.r#type = group(&caps, 1);
                st.commit.scope = group(&caps, 2);
                st.commit.subject = group(&caps, 3);
            }
        }
        if let Some(h) = st.commit.header.clone() {
            st.commit.references = self.parse_references(&h, false);
        }

        let mut is_body = true;
        while st.available() {
            self.parse_meta(&mut st);
            if self.parse_notes(&mut st) {
                is_body = false;
            }
            if !self.parse_body_and_footer(&mut st, is_body) {
                is_body = false;
            }
        }

        // parseBreakingHeader
        if st.commit.notes.is_empty()
            && let Some(h) = st.commit.header.as_deref()
            && let Ok(Some(caps)) = self.re.breaking_header.captures(h)
        {
            st.commit.notes.push(Note {
                title: "BREAKING CHANGE".into(),
                text: caps.get(3).map(|m| m.as_str().to_owned()).unwrap_or_default(),
            });
        }

        // parseRevert on the raw input
        if let Ok(Some(caps)) = self.re.revert.captures(input) {
            st.commit.revert = Some(Revert {
                header: group(&caps, 1),
                hash: group(&caps, 2),
            });
        }

        // cleanupCommit
        let c = &mut st.commit;
        if let Some(b) = c.body.as_mut().filter(|b| !b.is_empty()) {
            *b = trim_new_lines(b);
        }
        if let Some(f) = c.footer.as_mut().filter(|f| !f.is_empty()) {
            *f = trim_new_lines(f);
        }
        for note in &mut c.notes {
            note.text = trim_new_lines(&note.text);
        }
        let mut seen = HashSet::new();
        c.references.retain(|r| {
            let uid = format!("{} {}", r.action.as_deref().unwrap_or("null"), r.raw).to_lowercase();
            seen.insert(uid)
        });
        Some(st.commit)
    }
}

/// `commit[field] = appendLine(commit[field], line)`.
fn append_field(commit: &mut Commit, name: &str, line: &str) {
    if let Some(slot) = commit.field_mut(name) {
        append_to(slot, line);
    } else {
        let v = append_line(commit.fields.get(name).map(String::as_str), line);
        commit.fields.insert(name.to_owned(), v);
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

    fn lint_parser() -> CommitParser {
        CommitParser::new(ParserOptions {
            field_pattern: false,
            ..Default::default()
        })
    }

    #[test]
    fn header_body_footer() {
        let c = lint_parser()
            .parse("feat(api)!: add x\n\nbody line\n\nBREAKING CHANGE: gone\nCloses #12")
            .unwrap();
        assert_eq!(c.r#type.as_deref(), Some("feat"));
        assert_eq!(c.scope.as_deref(), Some("api"));
        assert_eq!(c.subject.as_deref(), Some("add x"));
        assert_eq!(c.body.as_deref(), Some("body line"));
        assert_eq!(c.footer.as_deref(), Some("BREAKING CHANGE: gone\nCloses #12"));
        assert_eq!(
            c.notes,
            vec![Note {
                title: "BREAKING CHANGE".into(),
                text: "gone".into()
            }]
        );
        assert_eq!(c.references.len(), 1);
        assert_eq!(c.references[0].action.as_deref(), Some("Closes"));
        assert_eq!(c.references[0].issue, "12");
    }

    #[test]
    fn fields() {
        let p = CommitParser::new(ParserOptions::default());
        let c = p
            .parse("fix: y\n\n-hash-\nabc\n-gitTags-\n (tag: v1.0.0)\n-committerDate-\n2024-01-01\n")
            .unwrap();
        assert_eq!(c.field("hash"), Some("abc"));
        assert_eq!(c.field("gitTags"), Some(" (tag: v1.0.0)"));
        assert_eq!(c.body.as_deref(), Some(""));
    }

    #[test]
    fn owner_repo_references() {
        let refs = lint_parser().parse_references("fixes org/repo#3, #4 and see #5", false);
        let issues: Vec<_> = refs
            .iter()
            .map(|r| (r.owner.clone(), r.repository.clone(), r.issue.clone()))
            .collect();
        assert_eq!(
            issues,
            vec![
                (Some("org".into()), Some("repo".into()), "3".into()),
                (None, None, "4".into()),
                (None, None, "5".into())
            ]
        );
    }
}
