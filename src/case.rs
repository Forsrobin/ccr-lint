//! Case checks of @commitlint/ensure 21 (`ensureCase`, `toCase`), with the es-toolkit 1.53 `compat`
//! functions they call (`startCase`, `camelCase`, `upperFirst`, `deburr`, lodash `words`).

use std::sync::LazyLock;

use fancy_regex::Regex;

use crate::js;
use crate::parser::JS_B;

/// `deburr` of es-toolkit compat for U+00C0..=U+017F, indexed by `code - 0xC0`.
const DEBURRED: [&str; 192] = [
    "A", "A", "A", "A", "A", "A", "Ae", "C", "E", "E", "E", "E", "I", "I", "I", "I", "D", "N", "O", "O", "O", "O", "O",
    "×", "O", "U", "U", "U", "U", "Y", "Th", "ss", "a", "a", "a", "a", "a", "a", "ae", "c", "e", "e", "e", "e", "i",
    "i", "i", "i", "d", "n", "o", "o", "o", "o", "o", "÷", "o", "u", "u", "u", "u", "y", "th", "y", "A", "a", "A", "a",
    "A", "a", "C", "c", "C", "c", "C", "c", "C", "c", "D", "d", "D", "d", "E", "e", "E", "e", "E", "e", "E", "e", "E",
    "e", "G", "g", "G", "g", "G", "g", "G", "g", "H", "h", "H", "h", "I", "i", "I", "i", "I", "i", "I", "i", "I", "i",
    "IJ", "ij", "J", "j", "K", "k", "k", "L", "l", "L", "l", "L", "l", "L", "l", "L", "l", "N", "n", "N", "n", "N",
    "n", "'n", "N", "n", "O", "o", "O", "o", "O", "o", "Oe", "oe", "R", "r", "R", "r", "R", "r", "S", "s", "S", "s",
    "S", "s", "S", "s", "T", "t", "T", "t", "T", "t", "U", "u", "U", "u", "U", "u", "U", "u", "U", "u", "U", "u", "W",
    "w", "Y", "y", "Y", "Z", "z", "Z", "z", "Z", "z", "s",
];

fn is_combining(c: char) -> bool {
    matches!(u32::from(c), 768..=879 | 8400..=8447 | 65056..=65071)
}

fn deburr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let code = u32::from(c);
        if (192..=383).contains(&code) {
            let idx = usize::try_from(code - 192).unwrap_or(0);
            out.push_str(DEBURRED.get(idx).copied().unwrap_or(""));
        } else if !is_combining(c) {
            out.push(c);
        }
    }
    out
}

fn normalize_for_case(s: &str) -> String {
    s.replace(['\'', '\u{2019}'], "")
}

/// lodash's word patterns. JS runs them on UTF-16 code units; astral characters (surrogate pairs there)
/// are spelled as code point ranges here.
static HAS_UNICODE_WORD: LazyLock<Regex> =
    LazyLock::new(|| js::re("[a-z][A-Z]|[A-Z]{2}[a-z]|[0-9][a-zA-Z]|[a-zA-Z][0-9]|[^a-zA-Z0-9 ]"));
static ASCII_WORD: LazyLock<Regex> = LazyLock::new(|| js::re(r"[^\x00-\x2f\x3a-\x40\x5b-\x60\x7b-\x7f]+"));
static UNICODE_WORD: LazyLock<Regex> = LazyLock::new(|| {
    let astral = r"\x{10000}-\x{10FFFF}";
    let combo = r"\x{300}-\x{36f}\x{fe20}-\x{fe2f}\x{20d0}-\x{20ff}";
    let dingbat = r"\x{2700}-\x{27bf}";
    let lower_r = r"a-z\x{df}-\x{f6}\x{f8}-\x{ff}";
    let upper_r = r"A-Z\x{c0}-\x{d6}\x{d8}-\x{de}";
    let var_r = r"\x{fe0e}\x{fe0f}";
    let brk = r"\x{ac}\x{b1}\x{d7}\x{f7}\x{00}-\x{2f}\x{3a}-\x{40}\x{5b}-\x{60}\x{7b}-\x{bf}\x{2000}-\x{206f} \t\x{0b}\x{0c}\x{a0}\x{feff}\n\r\x{2028}\x{2029}\x{1680}\x{180e}\x{2000}\x{2001}\x{2002}\x{2003}\x{2004}\x{2005}\x{2006}\x{2007}\x{2008}\x{2009}\x{200a}\x{202f}\x{205f}\x{3000}";
    let apos = "['\u{2019}]";
    let rs_break = format!("[{brk}]");
    let rs_combo = format!("[{combo}]");
    let digits = "[0-9]+";
    let rs_dingbat = format!("[{dingbat}]");
    let rs_lower = format!("[{lower_r}]");
    // `[^...\d+...]` in lodash: the `+` is a literal inside the class.
    let rs_misc = format!("[^{astral}{brk}0-9+{dingbat}{lower_r}{upper_r}]");
    let rs_modifier = format!(r"(?:{rs_combo}|[\x{{1F3FB}}-\x{{1F3FF}}])");
    let rs_non_astral = format!("[^{astral}]");
    let rs_regional = r"(?:[\x{1F1E6}-\x{1F1FF}]){2}";
    let rs_surr_pair = format!("[{astral}]");
    let rs_upper = format!("[{upper_r}]");
    let zwj = r"\x{200d}";
    let misc_lower = format!("(?:{rs_lower}|{rs_misc})");
    let misc_upper = format!("(?:{rs_upper}|{rs_misc})");
    let contr_lower = format!("(?:{apos}(?:d|ll|m|re|s|t|ve))?");
    let contr_upper = format!("(?:{apos}(?:D|LL|M|RE|S|T|VE))?");
    let opt_mod = format!("{rs_modifier}?");
    let opt_var = format!("[{var_r}]?");
    let opt_join = format!("(?:{zwj}(?:{rs_non_astral}|{rs_regional}|{rs_surr_pair}){opt_var}{opt_mod})*");
    let ord_lower = format!("[0-9]*(?:1st|2nd|3rd|(?![123])[0-9]th)(?={JS_B}|[A-Z_])");
    let ord_upper = format!("[0-9]*(?:1ST|2ND|3RD|(?![123])[0-9]TH)(?={JS_B}|[a-z_])");
    let seq = format!("{opt_var}{opt_mod}{opt_join}");
    let emoji = format!("(?:{rs_dingbat}|{rs_regional}|{rs_surr_pair}){seq}");
    js::re(
        &[
            format!("{rs_upper}?{rs_lower}+{contr_lower}(?={rs_break}|{rs_upper}|$)"),
            format!("{misc_upper}+{contr_upper}(?={rs_break}|{rs_upper}{misc_lower}|$)"),
            format!("{rs_upper}?{misc_lower}+{contr_lower}"),
            format!("{rs_upper}+{contr_upper}"),
            ord_upper,
            ord_lower,
            digits.to_owned(),
            emoji,
        ]
        .join("|"),
    )
});

fn words(s: &str) -> Vec<String> {
    let re = if js::test(&HAS_UNICODE_WORD, s) {
        &*UNICODE_WORD
    } else {
        &*ASCII_WORD
    };
    re.find_iter(s).flatten().map(|m| m.as_str().to_owned()).collect()
}

/// `String#toUpperCase` / `toLowerCase` (full Unicode mappings, like Rust's).
fn upper(s: &str) -> String {
    s.to_uppercase()
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// Split off the first UTF-16 code unit's worth: the first char, unless it is astral (then nothing, since
/// a lone surrogate does not change case; the caller keeps the whole char).
fn first_unit(s: &str) -> (&str, &str) {
    match s.chars().next() {
        Some(c) if u32::from(c) <= 0xFFFF => s.split_at(c.len_utf8()),
        _ => ("", s),
    }
}

fn upper_first(s: &str) -> String {
    let (first, rest) = first_unit(s);
    format!("{}{rest}", upper(first))
}

/// es-toolkit `capitalize`: `charAt(0).toUpperCase() + slice(1).toLowerCase()`.
fn capitalize(s: &str) -> String {
    let (first, rest) = first_unit(s);
    format!("{}{}", upper(first), lower(rest))
}

fn start_case(s: &str) -> String {
    let normalized = normalize_for_case(&deburr(s));
    let words = words(js::trim(&normalized));
    let mut out = String::new();
    for w in words {
        if !out.is_empty() {
            out.push(' ');
        }
        // the first code point, astral or not
        let mut chars = w.chars();
        if let Some(c) = chars.next() {
            out.push_str(&upper(&c.to_string()));
            out.push_str(chars.as_str());
        }
    }
    out
}

fn camel_case(s: &str) -> String {
    let words = words(&normalize_for_case(&deburr(s)));
    let mut it = words.iter();
    let Some(first) = it.next() else { return String::new() };
    let mut out = lower(first);
    for w in it {
        out.push_str(&capitalize(w));
    }
    out
}

fn kebab_like(s: &str, sep: &str) -> String {
    words(&normalize_for_case(&deburr(s)))
        .iter()
        .map(|w| lower(w))
        .collect::<Vec<_>>()
        .join(sep)
}

/// `toCase` of @commitlint/ensure; `None` for an unknown case name (it throws).
pub fn to_case(input: &str, target: &str) -> Option<String> {
    Some(match target {
        "camel-case" => camel_case(input),
        "kebab-case" => kebab_like(input, "-"),
        "snake-case" => kebab_like(input, "_"),
        "pascal-case" => upper_first(&camel_case(input)),
        "start-case" => start_case(input),
        "upper-case" | "uppercase" => upper(input),
        "sentence-case" | "sentencecase" => upper_first(input),
        "lower-case" | "lowercase" | "lowerCase" => lower(input),
        _ => return None,
    })
}

static QUOTED: LazyLock<Regex> = LazyLock::new(|| js::re(&format!(r#"`{d}*?`|"{d}*?"|'{d}*?'"#, d = js::DOT)));
static STARTS_WITH_DIGIT: LazyLock<Regex> = LazyLock::new(|| js::re("^[0-9]"));

/// `ensureCase(raw, target)`.
pub fn ensure_case(raw: &str, target: &str) -> Option<bool> {
    let replaced = QUOTED.replace_all(raw, "");
    let input = js::trim(&replaced);
    let transformed = to_case(input, target)?;
    if transformed.is_empty() || js::test(&STARTS_WITH_DIGIT, &transformed) {
        return Some(true);
    }
    Some(transformed == input)
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
    fn es_toolkit_cases() {
        assert_eq!(start_case("hello world"), "Hello World");
        assert_eq!(start_case("HELLO WORLD"), "HELLO WORLD");
        assert_eq!(start_case("hello-world"), "Hello World");
        assert_eq!(camel_case("HTTPRequest"), "httpRequest");
        assert_eq!(camel_case("some whitespace"), "someWhitespace");
        assert_eq!(upper_first(&camel_case("hyphen-text")), "HyphenText");
        assert_eq!(words("fred, barney, & pebbles"), vec!["fred", "barney", "pebbles"]);
        assert_eq!(deburr("Crème brûlée"), "Creme brulee");
    }

    #[test]
    fn ensure() {
        assert_eq!(ensure_case("Add thing", "sentence-case"), Some(true));
        assert_eq!(ensure_case("add thing", "sentence-case"), Some(false));
        assert_eq!(ensure_case("`Foo` bar", "sentence-case"), Some(false));
        assert_eq!(ensure_case("ADD", "upper-case"), Some(true));
    }
}
