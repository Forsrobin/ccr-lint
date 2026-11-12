//! ANSI styling as picocolors (commitlint) and chalk 6 (commit-and-tag-version) apply it, including when
//! each of them turns colors on.

use std::env;
use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavor {
    Pico,
    Chalk,
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    enabled: bool,
    flavor: Flavor,
}

fn env_truthy(name: &str) -> bool {
    env::var_os(name).is_some_and(|v| !v.is_empty())
}

fn has_arg(flag: &str) -> bool {
    let args: Vec<String> = env::args().collect();
    let pos = args.iter().position(|a| a == flag);
    let term = args.iter().position(|a| a == "--");
    pos.is_some_and(|p| term.is_none_or(|t| p < t))
}

impl Palette {
    pub fn off() -> Self {
        Palette {
            enabled: false,
            flavor: Flavor::Pico,
        }
    }

    /// picocolors' `isColorSupported`.
    pub fn picocolors() -> Self {
        let args: Vec<String> = env::args().collect();
        let has = |f: &str| args.iter().any(|a| a == f);
        let enabled = !(env_truthy("NO_COLOR") || has("--no-color"))
            && (env_truthy("FORCE_COLOR")
                || has("--color")
                || cfg!(windows)
                || (std::io::stdout().is_terminal() && env::var("TERM").ok().as_deref() != Some("dumb"))
                || env_truthy("CI"));
        Palette {
            enabled,
            flavor: Flavor::Pico,
        }
    }

    /// chalk's `supportsColor` level for stdout, > 0 meaning colors (only the basic 16 are used).
    pub fn chalk() -> Self {
        Palette {
            enabled: chalk_level() > 0,
            flavor: Flavor::Chalk,
        }
    }

    fn paint(&self, open: &str, close: &str, replace: &str, s: &str) -> String {
        if !self.enabled {
            return s.to_owned();
        }
        match self.flavor {
            Flavor::Pico => {
                // `string.indexOf(close, open.length)`
                let from = open.len().min(s.len());
                match s.get(from..).and_then(|t| t.find(close)) {
                    Some(i) => {
                        let idx = from + i;
                        let mut out = String::from(open);
                        out.push_str(s.get(..idx).unwrap_or(""));
                        let rest = s.get(idx..).unwrap_or("");
                        out.push_str(&rest.replace(close, replace));
                        out.push_str(close);
                        out
                    }
                    None => format!("{open}{s}{close}"),
                }
            }
            Flavor::Chalk => {
                if s.is_empty() {
                    return String::new();
                }
                let mut s = if s.contains('\u{1b}') {
                    s.replace(close, open)
                } else {
                    s.to_owned()
                };
                if s.contains('\n') {
                    let mut out = String::new();
                    let mut first = true;
                    for line in s.split('\n') {
                        if !first {
                            // close before the line break (and its \r) and reopen after it
                            let had_cr = out.ends_with('\r');
                            if had_cr {
                                out.pop();
                            }
                            out.push_str(close);
                            out.push_str(if had_cr { "\r\n" } else { "\n" });
                            out.push_str(open);
                        }
                        out.push_str(line);
                        first = false;
                    }
                    s = out;
                }
                format!("{open}{s}{close}")
            }
        }
    }

    pub fn bold(&self, s: &str) -> String {
        self.paint("\u{1b}[1m", "\u{1b}[22m", "\u{1b}[22m\u{1b}[1m", s)
    }
    pub fn red(&self, s: &str) -> String {
        self.paint("\u{1b}[31m", "\u{1b}[39m", "\u{1b}[31m", s)
    }
    pub fn green(&self, s: &str) -> String {
        self.paint("\u{1b}[32m", "\u{1b}[39m", "\u{1b}[32m", s)
    }
    pub fn yellow(&self, s: &str) -> String {
        self.paint("\u{1b}[33m", "\u{1b}[39m", "\u{1b}[33m", s)
    }
    pub fn blue(&self, s: &str) -> String {
        self.paint("\u{1b}[34m", "\u{1b}[39m", "\u{1b}[34m", s)
    }
    pub fn white(&self, s: &str) -> String {
        self.paint("\u{1b}[37m", "\u{1b}[39m", "\u{1b}[37m", s)
    }
    pub fn gray(&self, s: &str) -> String {
        self.paint("\u{1b}[90m", "\u{1b}[39m", "\u{1b}[90m", s)
    }
}

/// chalk 6 vendored supports-color, for stdout.
fn chalk_level() -> u8 {
    let mut flag_force: Option<u8> = None;
    if has_arg("--no-color") || has_arg("--no-colors") || has_arg("--color=false") || has_arg("--color=never") {
        flag_force = Some(0);
    } else if has_arg("--color") || has_arg("--colors") || has_arg("--color=true") || has_arg("--color=always") {
        flag_force = Some(1);
    }
    let force_env = env::var("FORCE_COLOR").ok();
    let numeric = force_env
        .as_deref()
        .is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()));
    let env_force: Option<u8> = match force_env.as_deref() {
        None => None,
        Some("false") => Some(0),
        Some("true" | "") => Some(1),
        Some(v) if numeric => Some(v.parse::<u32>().map_or(3, |n| u8::try_from(n.min(3)).unwrap_or(3))),
        Some(_) => None,
    };
    if env_force.is_some() {
        flag_force = env_force;
    }
    if flag_force == Some(0) {
        return 0;
    }
    for f in ["--color=16m", "--color=full", "--color=truecolor"] {
        if has_arg(f) {
            return 3;
        }
    }
    if has_arg("--color=256") {
        return 2;
    }
    if let Some(f) = flag_force
        && numeric
    {
        return f;
    }
    let set = |k: &str| env::var_os(k).is_some();
    if set("TF_BUILD") && set("AGENT_NAME") {
        return 1;
    }
    if !std::io::stdout().is_terminal() && flag_force.is_none() {
        return 0;
    }
    let min = flag_force.unwrap_or(0);
    let term = env::var("TERM").unwrap_or_default();
    if term == "dumb" {
        return min;
    }
    if cfg!(windows) {
        return 1;
    }
    if set("CI") {
        if ["GITHUB_ACTIONS", "GITEA_ACTIONS", "CIRCLECI"].iter().any(|k| set(k)) {
            return 3;
        }
        if ["TRAVIS", "APPVEYOR", "GITLAB_CI", "BUILDKITE", "DRONE"]
            .iter()
            .any(|k| set(k))
            || env::var("CI_NAME").ok().as_deref() == Some("codeship")
        {
            return 1;
        }
        return min;
    }
    if set("TEAMCITY_VERSION") {
        return 0;
    }
    if env::var("COLORTERM").ok().as_deref() == Some("truecolor") {
        return 3;
    }
    if matches!(term.as_str(), "xterm-kitty" | "xterm-ghostty" | "wezterm") {
        return 3;
    }
    if let Ok(p) = env::var("TERM_PROGRAM") {
        match p.as_str() {
            "iTerm.app" => return 2,
            "Apple_Terminal" => return 2,
            _ => {}
        }
    }
    let t = term.to_lowercase();
    if t.ends_with("-256") || t.ends_with("-256color") {
        return 2;
    }
    if ["screen", "xterm", "vt100", "vt220", "rxvt"]
        .iter()
        .any(|p| t.starts_with(p))
        || ["color", "ansi", "cygwin", "linux"].iter().any(|p| t.contains(p))
    {
        return 1;
    }
    if set("COLORTERM") {
        return 1;
    }
    min
}

/// `figures` glyphs (with the fallbacks of a terminal without Unicode, i.e. the Linux console).
pub struct Figures {
    pub tick: &'static str,
    pub info: &'static str,
    pub cross: &'static str,
}

pub fn figures() -> Figures {
    let unicode = if cfg!(windows) {
        false
    } else {
        env::var("TERM").ok().as_deref() != Some("linux")
    };
    if unicode {
        Figures {
            tick: "✔",
            info: "ℹ",
            cross: "✘",
        }
    } else {
        Figures {
            tick: "√",
            info: "i",
            cross: "×",
        }
    }
}
