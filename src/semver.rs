//! The parts of node-semver 7 the ported tools use: strict parsing (`valid`, `clean`), comparison and
//! `inc` for the release types commit-and-tag-version computes.

use std::cmp::Ordering;
use std::fmt;
use std::sync::LazyLock;

use fancy_regex::Regex;

use crate::js;

const MAX_LENGTH: usize = 256;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

static FULL: LazyLock<Regex> = LazyLock::new(|| {
    let num = r"0|[1-9][0-9]*";
    let non_num = r"[0-9]*[a-zA-Z-][a-zA-Z0-9-]*";
    let pre_id = format!("(?:{num}|{non_num})");
    js::re(&format!(
        r"^v?({num})\.({num})\.({num})(?:-({pre_id}(?:\.{pre_id})*))?(?:\+([a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)*))?$"
    ))
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ident {
    Num(u64),
    Str(String),
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ident::Num(n) => write!(f, "{n}"),
            Ident::Str(s) => f.write_str(s),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: Vec<Ident>,
    pub build: Vec<String>,
}

impl fmt::Display for Version {
    /// `SemVer#version`: build metadata is not part of it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.prerelease.is_empty() {
            let pre: Vec<String> = self.prerelease.iter().map(ToString::to_string).collect();
            write!(f, "-{}", pre.join("."))?;
        }
        Ok(())
    }
}

fn component(s: &str) -> Option<u64> {
    let n: u64 = s.parse().ok()?;
    (n <= MAX_SAFE_INTEGER).then_some(n)
}

/// `new SemVer(version)` (strict), `None` where it throws.
pub fn parse(version: &str) -> Option<Version> {
    if js::len16(version) > MAX_LENGTH {
        return None;
    }
    let caps = FULL.captures(js::trim(version)).ok()??;
    let get = |i: usize| caps.get(i).map(|m| m.as_str());
    let prerelease = match get(4) {
        Some(pre) if !pre.is_empty() => pre
            .split('.')
            .map(|id| match id.parse::<u64>() {
                Ok(n) if id.bytes().all(|b| b.is_ascii_digit()) && n < MAX_SAFE_INTEGER => Ident::Num(n),
                _ => Ident::Str(id.to_owned()),
            })
            .collect(),
        _ => Vec::new(),
    };
    Some(Version {
        major: component(get(1)?)?,
        minor: component(get(2)?)?,
        patch: component(get(3)?)?,
        prerelease,
        build: get(5)
            .filter(|b| !b.is_empty())
            .map(|b| b.split('.').map(str::to_owned).collect())
            .unwrap_or_default(),
    })
}

/// `semver.valid`.
pub fn valid(version: &str) -> Option<String> {
    parse(version).map(|v| v.to_string())
}

/// `semver.clean`.
pub fn clean(version: &str) -> Option<String> {
    let stripped = js::trim(version).trim_start_matches(['=', 'v']);
    valid(stripped)
}

fn compare_ident(a: &Ident, b: &Ident) -> Ordering {
    match (a, b) {
        (Ident::Num(x), Ident::Num(y)) => x.cmp(y),
        (Ident::Num(_), Ident::Str(_)) => Ordering::Less,
        (Ident::Str(_), Ident::Num(_)) => Ordering::Greater,
        (Ident::Str(x), Ident::Str(y)) => x.cmp(y),
    }
}

/// `SemVer#compare` (build metadata ignored).
pub fn compare(a: &Version, b: &Version) -> Ordering {
    (a.major, a.minor, a.patch)
        .cmp(&(b.major, b.minor, b.patch))
        .then_with(|| match (a.prerelease.is_empty(), b.prerelease.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (x, y) in a.prerelease.iter().zip(&b.prerelease) {
                    let o = compare_ident(x, y);
                    if o != Ordering::Equal {
                        return o;
                    }
                }
                a.prerelease.len().cmp(&b.prerelease.len())
            }
        })
}

/// `semver.lt(a, b)` on strings, `None` when either is invalid (node-semver throws).
pub fn lt(a: &str, b: &str) -> Option<bool> {
    Some(compare(&parse(a)?, &parse(b)?) == Ordering::Less)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    Major,
    Minor,
    Patch,
}

impl Release {
    pub fn from_level(level: u8) -> Release {
        match level {
            0 => Release::Major,
            1 => Release::Minor,
            _ => Release::Patch,
        }
    }

    pub fn parse(s: &str) -> Option<Release> {
        match s.to_lowercase().as_str() {
            "major" => Some(Release::Major),
            "minor" => Some(Release::Minor),
            "patch" => Some(Release::Patch),
            _ => None,
        }
    }
}

/// `semver.inc(version, release)` for major/minor/patch.
pub fn inc(version: &str, release: Release) -> Option<String> {
    let mut v = parse(version)?;
    match release {
        Release::Major => {
            if v.minor != 0 || v.patch != 0 || v.prerelease.is_empty() {
                v.major += 1;
            }
            v.minor = 0;
            v.patch = 0;
        }
        Release::Minor => {
            if v.patch != 0 || v.prerelease.is_empty() {
                v.minor += 1;
            }
            v.patch = 0;
        }
        Release::Patch => {
            if v.prerelease.is_empty() {
                v.patch += 1;
            }
        }
    }
    v.prerelease.clear();
    Some(v.to_string())
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
    fn parses_like_node_semver() {
        assert_eq!(valid("v1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(valid(" 1.2.3-rc.1+build.5 ").as_deref(), Some("1.2.3-rc.1"));
        assert_eq!(valid("01.2.3"), None);
        assert_eq!(valid("1.2"), None);
        assert_eq!(valid("=1.2.3"), None);
        assert_eq!(clean("=v1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(valid("1.2.3-01"), None);
    }

    #[test]
    fn increments() {
        assert_eq!(inc("1.2.3", Release::Minor).as_deref(), Some("1.3.0"));
        assert_eq!(inc("1.2.3", Release::Major).as_deref(), Some("2.0.0"));
        assert_eq!(inc("2.0.0-rc.1", Release::Major).as_deref(), Some("2.0.0"));
        assert_eq!(inc("1.2.0-rc.1", Release::Minor).as_deref(), Some("1.2.0"));
        assert_eq!(inc("1.2.3-rc.1", Release::Minor).as_deref(), Some("1.3.0"));
        assert_eq!(inc("1.2.3-rc.1", Release::Patch).as_deref(), Some("1.2.3"));
    }

    #[test]
    fn compares() {
        assert_eq!(lt("0.9.0", "1.0.0"), Some(true));
        assert_eq!(lt("1.0.0-rc.1", "1.0.0"), Some(true));
        assert_eq!(lt("1.0.0-2", "1.0.0-rc"), Some(true));
    }
}
