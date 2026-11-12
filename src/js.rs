//! JavaScript semantics the ported tools rely on: whitespace and line terminators as JS defines them,
//! string lengths in UTF-16 code units, and regex building blocks with JS meaning (`\w`, `\d`, `\s`, `.` are
//! ASCII / JS-specific there, Unicode-aware in Rust).

use fancy_regex::Regex;

/// JS `\s` (WhiteSpace + LineTerminator).
pub const WS: &str =
    r"[\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";
/// JS `.` without the `s` flag: anything but a line terminator.
pub const DOT: &str = r"[^\n\r\x{2028}\x{2029}]";
/// JS `^` with the `m` flag.
pub const ML_START: &str = r"(?:(?<![\s\S])|(?<=[\n\r\x{2028}\x{2029}]))";
/// JS `$` with the `m` flag.
pub const ML_END: &str = r"(?:(?![\s\S])|(?=[\n\r\x{2028}\x{2029}]))";
/// JS `\w`.
pub const W: &str = r"[A-Za-z0-9_]";
/// JS `\W`... not needed; JS `\d`.
pub const D: &str = r"[0-9]";

/// Compile a pattern that is a constant of this crate. A failure is a bug in the pattern, caught by the
/// unit tests that exercise every pattern, so it is reported as an empty-never-matching regex instead of a panic.
pub fn re(pattern: &str) -> Regex {
    match Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ccr-lint: internal regex error in {pattern:?}: {e}");
            #[allow(clippy::expect_used)] // `(?!)` always compiles
            Regex::new("(?!)").expect("never-matching regex")
        }
    }
}

/// `regex.test(s)`, treating engine errors (backtrack limit) as no match like a JS engine never would;
/// the patterns here are linear enough that this does not happen in practice.
pub fn test(r: &Regex, s: &str) -> bool {
    r.is_match(s).unwrap_or(false)
}

pub fn is_js_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// `String.prototype.trim`.
pub fn trim(s: &str) -> &str {
    s.trim_matches(is_js_ws)
}

pub fn trim_start(s: &str) -> &str {
    s.trim_start_matches(is_js_ws)
}

pub fn trim_end(s: &str) -> &str {
    s.trim_end_matches(is_js_ws)
}

/// `string.length`.
pub fn len16(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `s.split(/\r?\n/)`.
pub fn split_lines(s: &str) -> Vec<&str> {
    s.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect()
}

/// `Array.prototype.slice(start)` on a list, with JS negative-index semantics.
pub fn slice_from<T>(v: &[T], start: isize) -> &[T] {
    let len = isize::try_from(v.len()).unwrap_or(isize::MAX);
    let from = if start < 0 {
        (len + start).max(0)
    } else {
        start.min(len)
    };
    v.get(usize::try_from(from).unwrap_or(0)..).unwrap_or(&[])
}

/// Escape a string for use inside a regex, as `escape()` in conventional-commits-parser does.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if ".*+?^${}()|[]\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `joinOr`: trimmed, escaped alternatives.
pub fn join_or(parts: &[String]) -> String {
    parts
        .iter()
        .map(|p| escape(trim(p)))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("|")
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
    fn slice_follows_js() {
        let v = [1, 2, 3];
        assert_eq!(slice_from(&v, 1), &[2, 3]);
        assert_eq!(slice_from(&v, -1), &[3]);
        assert_eq!(slice_from(&v, -5), &[1, 2, 3]);
        assert_eq!(slice_from(&v, 7), &[] as &[i32]);
    }

    #[test]
    fn lengths_are_utf16() {
        assert_eq!(len16("a😀"), 3);
        assert_eq!(trim("\u{FEFF} x \n"), "x");
    }
}
