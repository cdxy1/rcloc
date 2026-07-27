//! Language definition tables for cloc.
//!
//! The tables are lifted verbatim from the Perl implementation's
//! `set_constants()` by `tools/extract_lang_defs.pl` and embedded here as
//! JSON. Nothing in this crate counts anything; it only answers "what
//! language is this file, and how are its comments written".

use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub mod filter;

pub use filter::{CommentDialect, Filter, RawFilter};

/// The extracted tables, as they appear on disk.
#[derive(Debug, Deserialize)]
struct RawDb {
    language_by_extension: HashMap<String, String>,
    language_by_script: HashMap<String, String>,
    language_by_file_type: HashMap<String, String>,
    language_by_prefix: HashMap<String, String>,
    filters_by_language: HashMap<String, Vec<RawFilter>>,
    not_code_extension: BTreeSet<String>,
    not_code_filename: BTreeSet<String>,
    scale_factor: HashMap<String, f64>,
    known_binary_archives: BTreeSet<String>,
    eol_continuation_re: HashMap<String, String>,
    extension_collision: BTreeMap<String, Vec<String>>,
}

/// Language definitions: how to name a file's language and how to strip its
/// comments.
#[derive(Debug)]
pub struct LangDb {
    /// `rs` -> `Rust`. Lower-cased lookups happen at the call site.
    language_by_extension: HashMap<String, String>,
    /// Shebang interpreter -> language, e.g. `python3` -> `Python`.
    language_by_script: HashMap<String, String>,
    /// Whole file name -> language, e.g. `Makefile` -> `make`.
    language_by_file_type: HashMap<String, String>,
    /// File name prefix -> language, e.g. `Dockerfile` -> `Dockerfile`.
    language_by_prefix: HashMap<String, String>,
    /// Language -> its ordered chain of comment filters.
    filters_by_language: HashMap<String, Vec<Filter>>,
    /// Extensions that are never code (`jpg`, `zip`, ...).
    not_code_extension: BTreeSet<String>,
    /// File names that are never code (`README`, `Makefile.in`, ...).
    not_code_filename: BTreeSet<String>,
    /// Language -> COCOMO-style scale factor.
    scale_factor: HashMap<String, f64>,
    /// Archive extensions cloc knows how to unpack.
    known_binary_archives: BTreeSet<String>,
    /// Language -> regex matching an end-of-line continuation marker.
    eol_continuation_re: HashMap<String, String>,
    /// Pseudo-language -> the extensions that are ambiguous between its
    /// members, e.g. `Lisp/Julia` -> `["jl"]`.
    extension_collision: BTreeMap<String, Vec<String>>,
    /// Reverse of the above: `jl` -> `Lisp/Julia`.
    collision_by_extension: HashMap<String, String>,
    /// The distinct values of `language_by_script`, i.e. every language a
    /// `#!` line can name.
    script_languages: BTreeSet<String>,
}

/// The JSON is embedded so the binary is self-contained: cloc is often copied
/// around as a single file, and a port that needs a sidecar data file next to
/// it would lose that property.
const EMBEDDED: &str = include_str!("../../../data/languages.json");

static DEFAULT: Lazy<LangDb> =
    Lazy::new(|| LangDb::from_json(EMBEDDED).expect("embedded languages.json is malformed"));

impl LangDb {
    /// The definitions compiled into this binary.
    pub fn default_db() -> &'static LangDb {
        &DEFAULT
    }

    /// Parse definitions from JSON in the extractor's schema.
    pub fn from_json(json: &str) -> Result<Self> {
        let raw: RawDb = serde_json::from_str(json).context("parsing language definitions")?;

        let mut filters_by_language = HashMap::with_capacity(raw.filters_by_language.len());
        for (language, raws) in &raw.filters_by_language {
            let resolved = raws
                .iter()
                .map(|r| Filter::from_raw(language, r))
                .collect::<Result<Vec<_>, _>>()?;
            filters_by_language.insert(language.clone(), resolved);
        }

        let mut collision_by_extension = HashMap::new();
        for (pseudo, exts) in &raw.extension_collision {
            for ext in exts {
                collision_by_extension.insert(ext.clone(), pseudo.clone());
            }
        }

        let script_languages = raw.language_by_script.values().cloned().collect();

        // An extension that names a language is never "not code", even when
        // both tables list it. The original prunes the overlap at startup, so
        // `csv` counts as CSV rather than being skipped as data.
        let mut not_code_extension = raw.not_code_extension;
        not_code_extension.retain(|ext| !raw.language_by_extension.contains_key(ext));

        Ok(Self {
            language_by_extension: raw.language_by_extension,
            language_by_script: raw.language_by_script,
            language_by_file_type: raw.language_by_file_type,
            language_by_prefix: raw.language_by_prefix,
            filters_by_language,
            not_code_extension,
            not_code_filename: raw.not_code_filename,
            scale_factor: raw.scale_factor,
            known_binary_archives: raw.known_binary_archives,
            eol_continuation_re: raw.eol_continuation_re,
            extension_collision: raw.extension_collision,
            collision_by_extension,
            script_languages,
        })
    }

    pub fn language_for_extension(&self, ext: &str) -> Option<&str> {
        self.language_by_extension.get(ext).map(String::as_str)
    }

    pub fn language_for_script(&self, interpreter: &str) -> Option<&str> {
        self.language_by_script.get(interpreter).map(String::as_str)
    }

    /// Whether a language can be named by a `#!` line. Such languages get to
    /// count their shebang as code even if a filter would have removed it.
    pub fn is_script_language(&self, language: &str) -> bool {
        self.script_languages.contains(language)
    }

    pub fn language_for_file_name(&self, name: &str) -> Option<&str> {
        self.language_by_file_type.get(name).map(String::as_str)
    }

    /// Match a file name against the prefix table (`Dockerfile.foo`).
    pub fn language_for_prefix(&self, name: &str) -> Option<&str> {
        self.language_by_prefix
            .iter()
            .find(|(prefix, _)| name.starts_with(prefix.as_str()))
            .map(|(_, lang)| lang.as_str())
    }

    pub fn filters(&self, language: &str) -> Option<&[Filter]> {
        self.filters_by_language.get(language).map(Vec::as_slice)
    }

    pub fn is_not_code_extension(&self, ext: &str) -> bool {
        self.not_code_extension.contains(ext)
    }

    pub fn is_not_code_filename(&self, name: &str) -> bool {
        self.not_code_filename.contains(name)
    }

    pub fn scale_factor(&self, language: &str) -> Option<f64> {
        self.scale_factor.get(language).copied()
    }

    pub fn is_known_binary_archive(&self, ext: &str) -> bool {
        self.known_binary_archives.contains(ext)
    }

    /// Archive extensions, used when hunting for archives *nested* inside an
    /// already-extracted tree. Deliberately a different set from the one that
    /// decides whether a command-line argument is an archive.
    pub fn binary_archive_extensions(&self) -> &BTreeSet<String> {
        &self.known_binary_archives
    }

    pub fn eol_continuation(&self, language: &str) -> Option<&str> {
        self.eol_continuation_re.get(language).map(String::as_str)
    }

    /// The pseudo-language an ambiguous extension maps to, if any.
    pub fn collision_for_extension(&self, ext: &str) -> Option<&str> {
        self.collision_by_extension.get(ext).map(String::as_str)
    }

    pub fn is_collision(&self, language: &str) -> bool {
        self.extension_collision.contains_key(language)
    }

    /// Every language that has a filter chain, sorted.
    pub fn languages(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.filters_by_language.keys().map(String::as_str).collect();
        v.sort_unstable();
        v
    }

    /// Every known extension, sorted.
    pub fn extensions(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .language_by_extension
            .keys()
            .map(String::as_str)
            .collect();
        v.sort_unstable();
        v
    }

    pub fn extension_collisions(&self) -> &BTreeMap<String, Vec<String>> {
        &self.extension_collision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_db_loads() {
        let db = LangDb::default_db();
        assert_eq!(db.language_for_extension("rs"), Some("Rust"));
        assert_eq!(db.language_for_extension("cpp"), Some("C++"));
        assert_eq!(db.language_for_script("python3"), Some("Python"));
        assert_eq!(db.language_for_file_name("Makefile"), Some("make"));
        assert_eq!(db.language_for_prefix("Dockerfile.dev"), Some("Dockerfile"));
    }

    /// The whole point of extracting rather than retyping: guard against a
    /// silently truncated table.
    #[test]
    fn table_sizes_match_the_perl_source() {
        let db = LangDb::default_db();
        assert_eq!(db.extensions().len(), 1012);
        assert_eq!(db.languages().len(), 422);
        assert_eq!(db.extension_collision.len(), 22);
        // Extensions that also name a language are pruned from the
        // not-code set, so this is below the 66 the Perl table lists.
        assert_eq!(db.not_code_extension.len(), 64);
        assert_eq!(db.not_code_filename.len(), 23);
        assert_eq!(db.known_binary_archives.len(), 11);
        assert_eq!(db.eol_continuation_re.len(), 70);
        assert_eq!(db.scale_factor.len(), 463);
    }

    /// Every filter in the table must resolve; `from_json` returns Err
    /// otherwise, so simply loading the embedded db proves this. Assert the
    /// shape of a couple of chains to catch argument-order regressions.
    #[test]
    fn filter_chains_resolve() {
        let db = LangDb::default_db();
        let cpp = db.filters("C++").expect("C++ has filters");
        assert_eq!(cpp.len(), 3);
        assert!(matches!(
            &cpp[0],
            Filter::RmCommentsInStrings { string_marker, start_comment, end_comment, multiline }
                if string_marker == "\"" && start_comment == "/*"
                    && end_comment == "*/" && !multiline
        ));
        assert!(matches!(
            &cpp[2],
            Filter::CallRegexpCommon { dialect: CommentDialect::Cpp }
        ));
    }

    /// `replace_between_regex` defaults multiline to true, the opposite of
    /// `rm_comments_in_strings`. Getting this backwards miscounts Java.
    #[test]
    fn replace_between_regex_multiline_defaults_true() {
        let db = LangDb::default_db();
        let java = db.filters("Java").expect("Java has filters");
        let rbr = java
            .iter()
            .find(|f| matches!(f, Filter::ReplaceBetweenRegex { .. }))
            .expect("Java uses replace_between_regex");
        assert!(matches!(
            rbr,
            Filter::ReplaceBetweenRegex { multiline: true, .. }
        ));
    }

    /// `csv` appears in both tables; naming a language must win, or CSV
    /// files are skipped as data.
    #[test]
    fn an_extension_that_names_a_language_is_code() {
        let db = LangDb::default_db();
        assert_eq!(db.language_for_extension("csv"), Some("CSV"));
        assert!(!db.is_not_code_extension("csv"));
        // Something that only appears in the not-code table is unaffected.
        assert!(db.is_not_code_extension("jpg"));
    }

    /// An ambiguous extension resolves to its pseudo-language, which the
    /// classifier then disambiguates by inspecting file content.
    #[test]
    fn ambiguous_extensions_map_to_collisions() {
        let db = LangDb::default_db();
        assert_eq!(
            db.collision_for_extension("m"),
            Some("MATLAB/Mathematica/Objective-C/MUMPS/Mercury")
        );
        assert_eq!(db.collision_for_extension("jl"), Some("Lisp/Julia"));
        assert!(db.is_collision("Perl/Prolog"));
        assert!(!db.is_collision("Rust"));
    }
}
