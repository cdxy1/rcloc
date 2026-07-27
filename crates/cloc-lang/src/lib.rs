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
pub mod langdef;

pub use filter::{CommentDialect, Filter, RawFilter};
pub use langdef::{LangDef, LangDefs};

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
    /// `--force-lang=LANG` with no extension: count everything as this.
    force_all_language: Option<String>,
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

    /// The embedded JSON, for callers that need their own mutable copy.
    pub fn embedded_json() -> &'static str {
        EMBEDDED
    }

    /// Count every file as `language`, per a bare `--force-lang=LANG`.
    pub fn force_all(&mut self, language: &str) {
        self.force_all_language = Some(language.to_string());
    }

    /// The language every file is being forced to, if any.
    pub fn forced_language(&self) -> Option<&str> {
        self.force_all_language.as_deref()
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
            force_all_language: None,
        })
    }

    /// Fold user-supplied definitions in, as `--read-lang-def` does.
    ///
    /// A language the built-ins already know keeps its filters and scale
    /// factor; only its extensions, file names and interpreters are added to.
    /// A language they do not know is taken whole. That asymmetry is the
    /// original's: the common use is teaching cloc a new extension for a
    /// language it can already count.
    pub fn merge_definitions(&mut self, defs: &LangDefs) -> Result<()> {
        for (language, def) in defs {
            let known = self.scale_factor.contains_key(language);
            if !known {
                let filters = def
                    .filters
                    .iter()
                    .map(|r| Filter::from_raw(language, r))
                    .collect::<Result<Vec<_>, _>>()?;
                self.filters_by_language.insert(language.clone(), filters);
                if let Some(scale) = def.scale_factor {
                    self.scale_factor.insert(language.clone(), scale);
                }
                if let Some(eol) = &def.eol_continuation {
                    self.eol_continuation_re
                        .insert(language.clone(), eol.clone());
                }
            }
            self.add_names(language, def);
        }
        Ok(())
    }

    /// Discard the built-ins and use only these definitions, as
    /// `--force-lang-def` does.
    pub fn replace_definitions(&mut self, defs: &LangDefs) -> Result<()> {
        self.language_by_extension.clear();
        self.language_by_script.clear();
        self.language_by_file_type.clear();
        self.filters_by_language.clear();
        self.scale_factor.clear();
        self.eol_continuation_re.clear();
        self.script_languages.clear();
        self.extension_collision.clear();
        self.collision_by_extension.clear();

        for (language, def) in defs {
            let filters = def
                .filters
                .iter()
                .map(|r| Filter::from_raw(language, r))
                .collect::<Result<Vec<_>, _>>()?;
            self.filters_by_language.insert(language.clone(), filters);
            if let Some(scale) = def.scale_factor {
                self.scale_factor.insert(language.clone(), scale);
            }
            if let Some(eol) = &def.eol_continuation {
                self.eol_continuation_re
                    .insert(language.clone(), eol.clone());
            }
            self.add_names(language, def);
        }
        Ok(())
    }

    fn add_names(&mut self, language: &str, def: &LangDef) {
        for ext in &def.extensions {
            self.language_by_extension
                .insert(ext.clone(), language.to_string());
            // A user-declared extension is code by definition.
            self.not_code_extension.remove(ext);
        }
        for name in &def.filenames {
            self.language_by_file_type
                .insert(name.clone(), language.to_string());
        }
        for exe in &def.script_exes {
            self.language_by_script
                .insert(exe.clone(), language.to_string());
            self.script_languages.insert(language.to_string());
        }
    }

    /// Force `extension` to be counted as `language`, per `--force-lang`.
    pub fn force_extension(&mut self, extension: &str, language: &str) {
        self.language_by_extension
            .insert(extension.to_string(), language.to_string());
        self.not_code_extension.remove(extension);
        self.collision_by_extension.remove(extension);
    }

    /// Map a `#!` interpreter to a language, per `--script-lang`.
    pub fn force_script(&mut self, interpreter: &str, language: &str) {
        self.language_by_script
            .insert(interpreter.to_string(), language.to_string());
        self.script_languages.insert(language.to_string());
    }

    /// Look up a language name case-insensitively, as the options do.
    pub fn canonical_language(&self, name: &str) -> Option<&str> {
        let wanted = name.to_lowercase();
        self.filters_by_language
            .keys()
            .find(|k| k.to_lowercase() == wanted)
            .map(String::as_str)
    }

    /// Render the current definitions in the text format.
    ///
    /// Languages whose extension is shared with another are skipped unless
    /// `include_duplicates`, since writing the shared extension under both
    /// names produces a file that cannot be read back.
    pub fn to_definitions(&self, include_duplicates: bool) -> LangDefs {
        let mut defs = LangDefs::new();
        for (language, filters) in &self.filters_by_language {
            if language.contains("Brain") || language == "(unknown)" {
                continue;
            }
            if self.extension_collision.contains_key(language) {
                continue;
            }
            let mut def = LangDef {
                filters: filters.iter().map(Filter::to_raw).collect(),
                scale_factor: self.scale_factor.get(language).copied(),
                eol_continuation: self.eol_continuation_re.get(language).cloned(),
                ..Default::default()
            };
            for (ext, owner) in &self.language_by_extension {
                if owner == language {
                    def.extensions.push(ext.clone());
                }
            }
            if def.extensions.is_empty() && include_duplicates {
                for (pseudo, exts) in &self.extension_collision {
                    if pseudo.split('/').any(|part| part == language) {
                        def.extensions.extend(exts.iter().cloned());
                    }
                }
            }
            for (name, owner) in &self.language_by_file_type {
                if owner == language {
                    def.filenames.push(name.clone());
                }
            }
            for (exe, owner) in &self.language_by_script {
                if owner == language {
                    def.script_exes.push(exe.clone());
                }
            }
            def.extensions.sort();
            def.filenames.sort();
            def.script_exes.sort();
            defs.insert(language.clone(), def);
        }
        defs
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

    /// `--read-lang-def` teaches an extension to a language cloc already
    /// knows, without disturbing that language's filters.
    #[test]
    fn merging_adds_names_but_keeps_known_filters() {
        let mut db = LangDb::from_json(LangDb::embedded_json()).unwrap();
        let before = db.filters("Python").unwrap().len();
        let defs = langdef::parse("Python\n    extension pyx2\n    filter remove_matches ^X\n")
            .unwrap();
        db.merge_definitions(&defs).unwrap();
        assert_eq!(db.language_for_extension("pyx2"), Some("Python"));
        assert_eq!(db.filters("Python").unwrap().len(), before);
    }

    /// A language the built-ins do not know is taken whole.
    #[test]
    fn merging_adds_unknown_languages_entirely() {
        let mut db = LangDb::from_json(LangDb::embedded_json()).unwrap();
        let defs = langdef::parse(
            "Widget\n    filter remove_matches ^;\n    extension wdg\n    3rd_gen_scale 1.00\n",
        )
        .unwrap();
        db.merge_definitions(&defs).unwrap();
        assert_eq!(db.language_for_extension("wdg"), Some("Widget"));
        assert_eq!(db.filters("Widget").unwrap().len(), 1);
        assert_eq!(db.scale_factor("Widget"), Some(1.0));
    }

    /// `--force-lang-def` discards the built-ins rather than adding to them.
    #[test]
    fn replacing_discards_the_built_ins() {
        let mut db = LangDb::from_json(LangDb::embedded_json()).unwrap();
        let defs =
            langdef::parse("Widget\n    extension wdg\n    3rd_gen_scale 1.00\n").unwrap();
        db.replace_definitions(&defs).unwrap();
        assert_eq!(db.language_for_extension("wdg"), Some("Widget"));
        assert_eq!(db.language_for_extension("rs"), None);
        assert_eq!(db.languages(), vec!["Widget"]);
    }

    /// What --write-lang-def emits must be readable by --force-lang-def, so
    /// the two options compose.
    #[test]
    fn written_definitions_can_be_read_back() {
        let db = LangDb::default_db();
        let text = langdef::write(&db.to_definitions(false));
        let parsed = langdef::parse(&text).expect("round trip parses");

        let mut rebuilt = LangDb::from_json(LangDb::embedded_json()).unwrap();
        rebuilt.replace_definitions(&parsed).unwrap();
        assert_eq!(rebuilt.language_for_extension("rs"), Some("Rust"));
        assert_eq!(rebuilt.language_for_extension("py"), Some("Python"));
        assert_eq!(rebuilt.language_for_file_name("Makefile"), Some("make"));
        assert_eq!(rebuilt.language_for_script("python3"), Some("Python"));
    }

    /// Language names are matched case-insensitively by the options.
    #[test]
    fn language_lookup_ignores_case() {
        let db = LangDb::default_db();
        assert_eq!(db.canonical_language("python"), Some("Python"));
        assert_eq!(db.canonical_language("BOURNE SHELL"), Some("Bourne Shell"));
        assert_eq!(db.canonical_language("nosuch"), None);
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
