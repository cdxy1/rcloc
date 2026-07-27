//! Aggregating counts and rendering them.

use cloc_core::counter::Counts;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// What one file contributed.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub language: String,
    pub counts: Counts,
}

/// A whole run's results.
#[derive(Debug, Default)]
pub struct Report {
    pub files: Vec<FileEntry>,
    pub ignored: Vec<(PathBuf, String)>,
    pub elapsed_secs: f64,
}

/// Per-language totals, plus the number of files in each.
#[derive(Debug, Clone, Copy, Default)]
pub struct LanguageTotals {
    pub files: usize,
    pub counts: Counts,
}

impl Report {
    /// Totals per language, keyed by name.
    pub fn by_language(&self) -> BTreeMap<&str, LanguageTotals> {
        let mut map: BTreeMap<&str, LanguageTotals> = BTreeMap::new();
        for f in &self.files {
            let e = map.entry(f.language.as_str()).or_default();
            e.files += 1;
            e.counts += f.counts;
        }
        map
    }

    pub fn totals(&self) -> LanguageTotals {
        let mut t = LanguageTotals::default();
        for f in &self.files {
            t.files += 1;
            t.counts += f.counts;
        }
        t
    }

    /// Languages ordered as cloc orders them: most code first, ties broken by
    /// name so the output is stable.
    pub fn languages_by_code(&self) -> Vec<(&str, LanguageTotals)> {
        let mut v: Vec<(&str, LanguageTotals)> = self.by_language().into_iter().collect();
        v.sort_by(|a, b| b.1.counts.code.cmp(&a.1.counts.code).then(a.0.cmp(b.0)));
        v
    }

    /// Files ordered by code descending, then path.
    pub fn files_by_code(&self) -> Vec<&FileEntry> {
        let mut v: Vec<&FileEntry> = self.files.iter().collect();
        v.sort_by(|a, b| b.counts.code.cmp(&a.counts.code).then(a.path.cmp(&b.path)));
        v
    }
}
