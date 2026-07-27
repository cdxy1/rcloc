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

/// What the comment and blank columns are shown as a percentage of, per
/// `--by-percent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denominator {
    /// `t`: each column as a percentage of its own total, i.e. `--percent`.
    ColumnTotal,
    /// `c`: lines of code.
    Code,
    /// `cm`: code plus comments.
    CodeComment,
    /// `cb`: code plus blanks.
    CodeBlank,
    /// `cmb`: code, comments and blanks.
    All,
}

impl Denominator {
    pub fn parse(spec: &str) -> Option<Self> {
        Some(match spec {
            "t" => Self::ColumnTotal,
            "c" => Self::Code,
            "cm" => Self::CodeComment,
            "cb" => Self::CodeBlank,
            "cmb" => Self::All,
            _ => return None,
        })
    }

    /// The divisor for one row's percentages.
    pub fn of(self, counts: Counts) -> f64 {
        let value = match self {
            Self::ColumnTotal => return 0.0,
            Self::Code => counts.code,
            Self::CodeComment => counts.code + counts.comment,
            Self::CodeBlank => counts.code + counts.blank,
            Self::All => counts.total(),
        };
        value as f64
    }
}

/// `--summary-cutoff=X:N[%]`: fold small languages into an "Other" row.
#[derive(Debug, Clone, Copy)]
pub struct Cutoff {
    key: CutoffKey,
    value: f64,
    by_percent: bool,
}

#[derive(Debug, Clone, Copy)]
enum CutoffKey {
    Code,
    Files,
    Comment,
    CodeComment,
}

impl Cutoff {
    /// Parse `c:100`, `f:5`, `m:2%` and friends.
    pub fn parse(spec: &str) -> Option<Self> {
        let (key, value) = spec.split_once(':')?;
        let key = match key {
            "c" => CutoffKey::Code,
            "f" => CutoffKey::Files,
            "m" => CutoffKey::Comment,
            "cm" => CutoffKey::CodeComment,
            _ => return None,
        };
        let by_percent = value.ends_with('%');
        let number = value.trim_end_matches('%').parse().ok()?;
        Some(Self {
            key,
            value: number,
            by_percent,
        })
    }

    fn measure(&self, t: LanguageTotals) -> f64 {
        match self.key {
            CutoffKey::Code => t.counts.code as f64,
            CutoffKey::Files => t.files as f64,
            CutoffKey::Comment => t.counts.comment as f64,
            CutoffKey::CodeComment => (t.counts.code + t.counts.comment) as f64,
        }
    }
}

impl Report {
    /// Per-language totals with small languages folded into "Other".
    pub fn by_language_with_cutoff(&self, cutoff: Option<Cutoff>) -> BTreeMap<String, LanguageTotals> {
        let plain: BTreeMap<String, LanguageTotals> = self
            .by_language()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let Some(cutoff) = cutoff else { return plain };

        // A percentage threshold is relative to the whole run, so the
        // absolute value has to be worked out before any row is judged.
        let threshold = if cutoff.by_percent {
            let overall = self.totals();
            cutoff.value * cutoff.measure(overall) / 100.0
        } else {
            cutoff.value
        };

        let mut out: BTreeMap<String, LanguageTotals> = BTreeMap::new();
        for (language, totals) in plain {
            let name = if cutoff.measure(totals) <= threshold {
                "Other".to_string()
            } else {
                language
            };
            let entry = out.entry(name).or_default();
            entry.files += totals.files;
            entry.counts += totals.counts;
        }
        out
    }

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

    /// As [`Report::languages_by_code`], but after applying a cutoff.
    pub fn languages_by_code_with_cutoff(
        &self,
        cutoff: Option<Cutoff>,
    ) -> Vec<(String, LanguageTotals)> {
        let mut v: Vec<(String, LanguageTotals)> =
            self.by_language_with_cutoff(cutoff).into_iter().collect();
        v.sort_by(|a, b| b.1.counts.code.cmp(&a.1.counts.code).then(a.0.cmp(&b.0)));
        v
    }

    /// Files ordered by code descending, then path.
    pub fn files_by_code(&self) -> Vec<&FileEntry> {
        let mut v: Vec<&FileEntry> = self.files.iter().collect();
        v.sort_by(|a, b| b.counts.code.cmp(&a.counts.code).then(a.path.cmp(&b.path)));
        v
    }
}
