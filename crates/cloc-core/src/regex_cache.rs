//! Perl regex sources, compiled for Rust.
//!
//! The language table stores regexes exactly as the Perl script wrote them.
//! Most are portable, but a few use constructs Rust's engines spell
//! differently (octal escapes) or do not accept at all. [`translate`] handles
//! the former; the latter are caught by a test that compiles every regex in
//! the table.
//!
//! Compilation is deferred and cached: a run that touches three languages
//! should not pay to compile the other 419.

use anyhow::{Context, Result};
use fancy_regex::Regex;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Rewrite a Perl regex into the dialect Rust's engines accept.
///
/// Only differences that actually appear in cloc's tables are handled; this
/// is deliberately not a general Perl-to-Rust regex translator.
pub fn translate(perl: &str) -> String {
    let mut out = String::with_capacity(perl.len());
    let bytes: Vec<char> = perl.chars().collect();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];
        if c != '\\' || i + 1 >= bytes.len() {
            out.push(c);
            i += 1;
            continue;
        }

        let next = bytes[i + 1];
        // Perl writes octal escapes as \NNN, most visibly \47 for a single
        // quote (used to avoid quoting headaches in the table itself).
        // Rust's engines read \47 as a backreference to group 47.
        if next.is_digit(8) {
            let mut j = i + 1;
            let mut digits = String::new();
            while j < bytes.len() && bytes[j].is_digit(8) && digits.len() < 3 {
                digits.push(bytes[j]);
                j += 1;
            }
            // A single digit is a backreference (\1), not an octal escape.
            // Two or more digits starting with 0-7 is octal in the tables.
            if digits.len() >= 2 {
                if let Ok(code) = u32::from_str_radix(&digits, 8) {
                    if let Some(ch) = char::from_u32(code) {
                        escape_literal_char(&mut out, ch);
                        i = j;
                        continue;
                    }
                }
            }
        }

        out.push(c);
        out.push(next);
        i += 2;
    }

    out
}

/// Append `ch` to a regex as a literal, escaping it if it is a metacharacter.
fn escape_literal_char(out: &mut String, ch: char) {
    if "\\.+*?()|[]{}^$#&-~".contains(ch) {
        out.push('\\');
    }
    out.push(ch);
}

/// Compile a Perl regex source, translating it first.
pub fn compile(perl: &str) -> Result<Regex> {
    let translated = translate(perl);
    Regex::new(&translated)
        .with_context(|| format!("compiling regex {perl:?} (translated to {translated:?})"))
}

/// Process-wide cache of compiled regexes, keyed by Perl source.
///
/// Filter chains are shared across every file of a language, so the same
/// handful of patterns is requested repeatedly. Regexes are immutable once
/// built, so handing out `&'static Regex` from a leaked box lets callers hold
/// them without a lock.
pub fn cached(perl: &str) -> Result<&'static Regex> {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    if let Some(re) = cache.lock().unwrap().get(perl) {
        return Ok(re);
    }

    let re: &'static Regex = Box::leak(Box::new(compile(perl)?));
    cache.lock().unwrap().insert(perl.to_string(), re);
    Ok(re)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloc_lang::{Filter, LangDb};

    #[test]
    fn octal_escapes_become_literals() {
        // \47 is a single quote, the most common octal escape in the tables.
        assert_eq!(translate(r"^\s*\47"), r"^\s*'");
        // A lone digit stays a backreference.
        assert_eq!(translate(r#"(["'])(.*?)\1"#), r#"(["'])(.*?)\1"#);
    }

    #[test]
    fn asp_comment_regex_matches_a_quote() {
        let re = compile(r"^\s*\47").unwrap();
        assert!(re.is_match("   ' a comment").unwrap());
        assert!(!re.is_match("   code").unwrap());
    }

    /// Every regex the language table can hand the engine must compile.
    /// This is the check that turns "some obscure language panics at runtime"
    /// into a test failure.
    #[test]
    fn every_table_regex_compiles() {
        let db = LangDb::default_db();
        let mut failures = Vec::new();

        for language in db.languages() {
            for filter in db.filters(language).unwrap_or(&[]) {
                let sources: Vec<&str> = match filter {
                    Filter::RemoveMatches { re }
                    | Filter::RemoveInline { re }
                    | Filter::RemoveAbove { re }
                    | Filter::RemoveBelow { re } => vec![re],
                    Filter::RemoveBelowAbove { below, above } => vec![below, above],
                    Filter::RemoveBetweenRegex { start, end } => vec![start, end],
                    Filter::ReplaceBetweenRegex { start, end, .. } => vec![start, end],
                    Filter::ReplaceRegex { re, .. } => vec![re],
                    _ => vec![],
                };
                for src in sources {
                    if let Err(e) = compile(src) {
                        failures.push(format!("{language}: {src:?}: {e}"));
                    }
                }
            }
        }

        assert!(
            failures.is_empty(),
            "{} table regex(es) failed to compile:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// The EOL-continuation patterns are applied to every line of the
    /// languages that define them, so they must compile too.
    #[test]
    fn every_continuation_regex_compiles() {
        let db = LangDb::default_db();
        let mut failures = Vec::new();
        for language in db.languages() {
            if let Some(src) = db.eol_continuation(language) {
                if let Err(e) = compile(src) {
                    failures.push(format!("{language}: {src:?}: {e}"));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
