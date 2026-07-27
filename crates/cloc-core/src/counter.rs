//! Counting one file.
//!
//! The arithmetic is the original's: blanks are what the blank pass removed,
//! code is what survives the whole filter chain, and comments are whatever is
//! left over. Deriving comments by subtraction rather than counting them
//! directly is why the chain re-runs its blank pass after every filter — a
//! comment that leaves an empty line behind must not be counted twice.

use crate::filters::{self, FilterContext, FilterOptions};
use crate::{io, regex_cache};
use anyhow::Result;
use cloc_lang::LangDb;
use std::path::Path;

/// Line counts for a single file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub blank: usize,
    pub comment: usize,
    pub code: usize,
}

impl Counts {
    pub fn total(&self) -> usize {
        self.blank + self.comment + self.code
    }
}

impl std::ops::AddAssign for Counts {
    fn add_assign(&mut self, rhs: Self) {
        self.blank += rhs.blank;
        self.comment += rhs.comment;
        self.code += rhs.code;
    }
}

/// Options affecting the count of a file.
#[derive(Debug, Clone, Default)]
pub struct CountOptions {
    pub filters: FilterOptions,
    /// `--skip-leading=N[,ext,...]`: drop N leading lines, optionally only
    /// for the listed extensions.
    pub skip_leading: Option<(usize, Vec<String>)>,
    /// `--ignore-regex`: patterns whose matching lines are struck from the
    /// count entirely, rather than counted as comments.
    pub ignore_regex: Vec<String>,
}

/// Count `path`, already known to be `language`.
pub fn count_file(
    path: &Path,
    language: &str,
    db: &LangDb,
    opts: &CountOptions,
) -> Result<Counts> {
    let lines = io::read_lines(path)?;
    count_lines(lines, path, language, db, opts)
}

/// Count already-read lines. Split out so tests and the diff mode can count
/// content that never lived in a file.
pub fn count_lines(
    lines: Vec<String>,
    path: &Path,
    language: &str,
    db: &LangDb,
    opts: &CountOptions,
) -> Result<Counts> {
    let lines = match &opts.skip_leading {
        Some((n, exts)) if applies_to(path, exts) => remove_first_n(lines, *n),
        _ => lines,
    };

    let mut total = lines.len();
    let continuation = db.eol_continuation(language);

    // COBOL decides blankness by column, not by whitespace alone.
    let after_blanks = if language == "COBOL" {
        filters::remove_cobol_blanks(lines)
    } else {
        filters::remove_blank_lines(lines, continuation)?
    };
    let blank = total - after_blanks.len();

    let filter_chain = db.filters(language).unwrap_or(&[]);
    let ctx = FilterContext {
        options: &opts.filters,
        file: path,
        language,
        eol_continuation: continuation,
    };
    let mut remaining = filters::apply_chain(after_blanks, filter_chain, &ctx)?;

    // Lines struck by --ignore-regex leave the tally altogether.
    if !opts.ignore_regex.is_empty() {
        let before = remaining.len();
        remaining = apply_ignores(remaining, &opts.ignore_regex)?;
        total -= before - remaining.len();
    }

    Ok(Counts {
        blank,
        comment: total.saturating_sub(blank).saturating_sub(remaining.len()),
        code: remaining.len(),
    })
}

/// Whether `--skip-leading` applies to this file: an empty extension list
/// means every file.
fn applies_to(path: &Path, exts: &[String]) -> bool {
    if exts.is_empty() {
        return true;
    }
    path.extension()
        .map(|e| e.to_string_lossy().into_owned())
        .is_some_and(|e| exts.iter().any(|x| *x == e))
}

fn remove_first_n(lines: Vec<String>, n: usize) -> Vec<String> {
    // The original returns nothing when asked to skip at least as many lines
    // as the file has.
    if lines.len() > n {
        lines.into_iter().skip(n).collect()
    } else {
        Vec::new()
    }
}

fn apply_ignores(lines: Vec<String>, patterns: &[String]) -> Result<Vec<String>> {
    let mut compiled = Vec::with_capacity(patterns.len());
    for p in patterns {
        compiled.push(regex_cache::cached(p)?);
    }
    let mut out = Vec::with_capacity(lines.len());
    'line: for line in lines {
        for re in &compiled {
            if re.is_match(&line)? {
                continue 'line;
            }
        }
        out.push(line);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(language: &str, source: &str) -> Counts {
        count_with(language, source, CountOptions::default())
    }

    fn count_with(language: &str, source: &str, opts: CountOptions) -> Counts {
        let lines = io::lines_from_bytes(source.as_bytes());
        count_lines(
            lines,
            Path::new("test.txt"),
            language,
            LangDb::default_db(),
            &opts,
        )
        .unwrap()
    }

    #[test]
    fn the_three_counts_partition_the_file() {
        let src = "/* c */\n\nint x;\n";
        let c = count("C", src);
        assert_eq!(c, Counts { blank: 1, comment: 1, code: 1 });
        assert_eq!(c.total(), src.lines().count());
    }

    #[test]
    fn a_mixed_line_counts_as_code() {
        let c = count("C", "int x; /* note */\n");
        assert_eq!(c, Counts { blank: 0, comment: 0, code: 1 });
    }

    /// A comment that spans lines contributes one comment line per line.
    #[test]
    fn multi_line_comments_count_each_line() {
        let c = count("C", "/*\n * a\n * b\n */\nint x;\n");
        assert_eq!(c, Counts { blank: 0, comment: 4, code: 1 });
    }

    #[test]
    fn empty_file_counts_nothing() {
        assert_eq!(count("C", ""), Counts::default());
    }

    #[test]
    fn blank_only_file_is_all_blank() {
        assert_eq!(
            count("C", "\n\n\n"),
            Counts { blank: 3, comment: 0, code: 0 }
        );
    }

    #[test]
    fn python_counts() {
        let c = count("Python", "# c\n\ndef f():\n    \"\"\"doc\"\"\"\n    return 1\n");
        assert_eq!(c, Counts { blank: 1, comment: 2, code: 2 });
    }

    #[test]
    fn skip_leading_drops_a_licence_header() {
        let opts = CountOptions {
            skip_leading: Some((2, vec![])),
            ..Default::default()
        };
        let c = count_with("C", "// lic 1\n// lic 2\nint x;\n", opts);
        assert_eq!(c, Counts { blank: 0, comment: 0, code: 1 });
    }

    /// `--ignore-regex` removes lines from the total, so the parts still sum
    /// to it.
    #[test]
    fn ignore_regex_shrinks_the_total() {
        let opts = CountOptions {
            ignore_regex: vec!["^int".to_string()],
            ..Default::default()
        };
        let c = count_with("C", "/* c */\nint x;\nfloat y;\n", opts);
        assert_eq!(c, Counts { blank: 0, comment: 1, code: 1 });
        assert_eq!(c.total(), 2);
    }
}
