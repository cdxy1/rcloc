//! Comparing two trees: `--diff`.
//!
//! Files are paired by their path relative to the input root, each pair is
//! diffed line by line, and the results are tallied per language as same,
//! modified, added and removed. A file present on only one side contributes
//! all of its lines to added or removed.
//!
//! Pairing is delegated to [`crate::align`], which strips the leading
//! directories before matching so that two releases of the same project line
//! up despite their directory names differing.

use crate::align;
use crate::classify::{self, Classification, ClassifyOptions};
use crate::counter::CountOptions;
use crate::diff::Delta;
use crate::filters::{self, FilterContext};
use crate::{diff, io};
use anyhow::Result;
use cloc_lang::LangDb;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The three line categories, each with its own tally of edits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileDelta {
    pub blank: Delta,
    pub comment: Delta,
    pub code: Delta,
}

impl std::ops::AddAssign for FileDelta {
    fn add_assign(&mut self, rhs: Self) {
        self.blank += rhs.blank;
        self.comment += rhs.comment;
        self.code += rhs.code;
    }
}

/// Per-language totals, plus how many files fell into each disposition.
#[derive(Debug, Clone, Copy, Default)]
pub struct LanguageDelta {
    pub files: Delta,
    pub delta: FileDelta,
}

/// The result of comparing two trees.
#[derive(Debug, Default)]
pub struct DiffReport {
    pub by_language: BTreeMap<String, LanguageDelta>,
    /// Per-file results, for `--by-file`.
    pub by_file: Vec<(PathBuf, String, FileDelta)>,
    /// Pair alignment, for `--diff-alignment`.
    pub alignment: Vec<String>,
}

/// Options affecting how lines are compared.
#[derive(Debug, Clone, Default)]
pub struct DiffOptions {
    pub count: CountOptions,
    /// `--ignore-whitespace`: compare with all whitespace removed.
    pub ignore_whitespace: bool,
    /// `--ignore-case`: compare case-insensitively.
    pub ignore_case: bool,
    pub classify: ClassifyOptions,
}

/// Pair up the files of two trees and diff each pair.
pub fn compare(
    _left_root: &Path,
    left_files: &[PathBuf],
    _right_root: &Path,
    right_files: &[PathBuf],
    db: &LangDb,
    opts: &DiffOptions,
) -> Result<DiffReport> {
    let alignment = align::align(left_files, right_files);
    let mut report = DiffReport::default();

    for (left, right) in &alignment.pairs {
        compare_pair(left, right, left, db, opts, &mut report)?;
    }
    for path in &alignment.removed {
        one_sided(path, path, db, opts, false, &mut report)?;
    }
    for path in &alignment.added {
        one_sided(path, path, db, opts, true, &mut report)?;
    }

    report.by_file.sort_by(|a, b| a.0.cmp(&b.0));
    report.alignment.sort();
    Ok(report)
}

/// A file on only one side: every line is added, or every line removed.
fn one_sided(
    path: &Path,
    key: &Path,
    db: &LangDb,
    opts: &DiffOptions,
    added: bool,
    report: &mut DiffReport,
) -> Result<()> {
    let Some(language) = language_of(path, db, &opts.classify)? else {
        return Ok(());
    };
    let counts = crate::counter::count_file(path, &language, db, &opts.count)?;

    let side = |n: usize| {
        if added {
            Delta { added: n, ..Default::default() }
        } else {
            Delta { removed: n, ..Default::default() }
        }
    };
    let delta = FileDelta {
        blank: side(counts.blank),
        comment: side(counts.comment),
        code: side(counts.code),
    };

    let entry = report.by_language.entry(language.clone()).or_default();
    if added {
        entry.files.added += 1;
    } else {
        entry.files.removed += 1;
    }
    entry.delta += delta;
    report.by_file.push((key.to_path_buf(), language, delta));
    report
        .alignment
        .push(format!("  {} {}", if added { "+" } else { "-" }, key.display()));
    Ok(())
}

/// Two versions of the same file.
fn compare_pair(
    left: &Path,
    right: &Path,
    key: &Path,
    db: &LangDb,
    opts: &DiffOptions,
    report: &mut DiffReport,
) -> Result<()> {
    let Some(language) = language_of(left, db, &opts.classify)? else {
        return Ok(());
    };

    let left_lines = io::read_lines(left)?;
    let right_lines = io::read_lines(right)?;
    let identical = left_lines == right_lines;

    let entry = report.by_language.entry(language.clone()).or_default();
    if identical {
        entry.files.same += 1;
    } else {
        entry.files.modified += 1;
    }
    report.alignment.push(format!(
        "  {} {}",
        if identical { "==" } else { "!=" },
        key.display()
    ));

    // Identical files need no diffing: every line is "same".
    if identical {
        let counts = crate::counter::count_file(left, &language, db, &opts.count)?;
        let same = |n: usize| Delta { same: n, ..Default::default() };
        let delta = FileDelta {
            blank: same(counts.blank),
            comment: same(counts.comment),
            code: same(counts.code),
        };
        report.by_language.entry(language.clone()).or_default().delta += delta;
        report.by_file.push((key.to_path_buf(), language, delta));
        return Ok(());
    }

    let (code_l, comment_l) = classify_lines(left_lines, left, &language, db, opts)?;
    let (code_r, comment_r) = classify_lines(right_lines, right, &language, db, opts)?;

    let normalise = |mut lines: Vec<String>| {
        if opts.ignore_whitespace {
            lines = lines
                .into_iter()
                .map(|l| l.chars().filter(|c| !c.is_whitespace()).collect())
                .collect();
        }
        if opts.ignore_case {
            lines = lines.into_iter().map(|l| l.to_lowercase()).collect();
        }
        lines
    };

    let counts_l = crate::counter::count_file(left, &language, db, &opts.count)?;
    let counts_r = crate::counter::count_file(right, &language, db, &opts.count)?;

    let delta = FileDelta {
        // Blank lines have no identity to diff, so the change in their count
        // is reported as an addition or a removal and the rest as unchanged.
        blank: blank_delta(counts_l.blank, counts_r.blank),
        comment: Delta::between(&normalise(comment_l), &normalise(comment_r)),
        code: Delta::between(&normalise(code_l), &normalise(code_r)),
    };

    report.by_language.entry(language.clone()).or_default().delta += delta;
    report.by_file.push((key.to_path_buf(), language, delta));
    Ok(())
}

/// Blank lines are interchangeable, so for a changed file only the change in
/// how many there are is reported -- never "same", however many both sides
/// have. A file whose contents match exactly is handled separately and does
/// report its blanks as unchanged.
fn blank_delta(left: usize, right: usize) -> Delta {
    Delta {
        same: 0,
        modified: 0,
        added: right.saturating_sub(left),
        removed: left.saturating_sub(right),
    }
}

/// Split a file's lines into code and comments, the way the counter does.
fn classify_lines(
    lines: Vec<String>,
    path: &Path,
    language: &str,
    db: &LangDb,
    opts: &DiffOptions,
) -> Result<(Vec<String>, Vec<String>)> {
    let continuation = db.eol_continuation(language);
    let without_blanks =
        filters::remove_blank_lines_for_language(lines, continuation, language)?;

    let ctx = FilterContext {
        options: &opts.count.filters,
        file: path,
        language,
        eol_continuation: continuation,
    };
    let without_comments = filters::apply_chain(
        without_blanks.clone(),
        db.filters(language).unwrap_or(&[]),
        &ctx,
    )?;

    Ok(diff::split_code_and_comments(&without_blanks, &without_comments))
}

fn language_of(path: &Path, db: &LangDb, opts: &ClassifyOptions) -> Result<Option<String>> {
    Ok(match classify::classify(path, db, opts)? {
        Classification::Language(l) => Some(l),
        Classification::Ignored { .. } => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Build two trees and compare them.
    fn compare_trees(
        tag: &str,
        left: &[(&str, &str)],
        right: &[(&str, &str)],
    ) -> DiffReport {
        let base = std::env::temp_dir().join(format!("cloc-diff-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (lroot, rroot) = (base.join("L"), base.join("R"));
        let mut lfiles = Vec::new();
        let mut rfiles = Vec::new();
        for (root, files, out) in [
            (&lroot, left, &mut lfiles),
            (&rroot, right, &mut rfiles),
        ] {
            for (name, content) in files {
                let path = root.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, content).unwrap();
                out.push(path);
            }
        }
        compare(
            &lroot,
            &lfiles,
            &rroot,
            &rfiles,
            LangDb::default_db(),
            &DiffOptions::default(),
        )
        .unwrap()
    }

    #[test]
    fn identical_trees_are_all_same() {
        let files = [("a.py", "x = 1\n# note\n\ny = 2\n")];
        let r = compare_trees("same", &files, &files);
        let py = &r.by_language["Python"];
        assert_eq!(py.files.same, 1);
        assert_eq!(py.files.modified, 0);
        assert_eq!(py.delta.code.same, 2);
        assert_eq!(py.delta.comment.same, 1);
        assert_eq!(py.delta.blank.same, 1);
    }

    #[test]
    fn a_new_file_is_all_added() {
        let r = compare_trees("added", &[], &[("a.py", "x = 1\ny = 2\n")]);
        let py = &r.by_language["Python"];
        assert_eq!(py.files.added, 1);
        assert_eq!(py.delta.code.added, 2);
    }

    #[test]
    fn a_deleted_file_is_all_removed() {
        let r = compare_trees("removed", &[("a.py", "x = 1\n")], &[]);
        let py = &r.by_language["Python"];
        assert_eq!(py.files.removed, 1);
        assert_eq!(py.delta.code.removed, 1);
    }

    /// A changed line is one modification; the file is counted as modified.
    #[test]
    fn a_changed_line_is_modified() {
        let r = compare_trees(
            "mod",
            &[("a.py", "x = 1\ny = 2\n")],
            &[("a.py", "x = 99\ny = 2\n")],
        );
        let py = &r.by_language["Python"];
        assert_eq!(py.files.modified, 1);
        assert_eq!(py.delta.code.modified, 1);
        assert_eq!(py.delta.code.same, 1);
    }

    /// Code and comments are tallied separately, so a new comment does not
    /// show up as new code.
    #[test]
    fn comments_and_code_are_counted_apart() {
        let r = compare_trees(
            "cats",
            &[("a.py", "x = 1\n")],
            &[("a.py", "# a new note\nx = 1\n")],
        );
        let py = &r.by_language["Python"];
        assert_eq!(py.delta.comment.added, 1);
        assert_eq!(py.delta.code.added, 0);
        assert_eq!(py.delta.code.same, 1);
    }

    /// Blank lines have no identity, so only their count is compared.
    #[test]
    fn blank_lines_are_compared_by_count() {
        let r = compare_trees(
            "blanks",
            &[("a.py", "x = 1\n\ny = 2\n")],
            &[("a.py", "x = 1\n\n\n\ny = 2\n")],
        );
        let py = &r.by_language["Python"];
        // Only the surplus is reported; matched blanks are not "same".
        assert_eq!(py.delta.blank.same, 0);
        assert_eq!(py.delta.blank.added, 2);
    }

    /// Files are paired by their path relative to each root, so the roots
    /// themselves may be named anything.
    #[test]
    fn files_pair_by_relative_path() {
        let r = compare_trees(
            "paths",
            &[("src/a.py", "x = 1\n"), ("gone.py", "z = 3\n")],
            &[("src/a.py", "x = 2\n"), ("new.py", "w = 4\n")],
        );
        let py = &r.by_language["Python"];
        assert_eq!(py.files.modified, 1);
        assert_eq!(py.files.added, 1);
        assert_eq!(py.files.removed, 1);
    }

    #[test]
    fn alignment_records_each_pair() {
        let r = compare_trees(
            "align",
            &[("a.py", "x = 1\n")],
            &[("a.py", "x = 2\n"), ("b.py", "y = 1\n")],
        );
        assert!(r.alignment.iter().any(|l| l.contains("!=") && l.contains("a.py")));
        assert!(r.alignment.iter().any(|l| l.starts_with("  +") && l.contains("b.py")));
    }
}
