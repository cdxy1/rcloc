//! Rendering a report in each supported format.

use crate::report::{Cutoff, Denominator, LanguageTotals, Report};
use std::fmt::Write as _;

/// Output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
    Yaml,
    Csv,
    Markdown,
    Xml,
}

/// Rendering knobs shared by the formats.
#[derive(Debug, Clone, Default)]
pub struct OutputOptions {
    /// `--by-file`: report each file rather than each language.
    pub by_file: bool,
    /// `--hide-rate`: leave the processing rate out of the header.
    pub hide_rate: bool,
    /// `--quiet`: suppress the header entirely.
    pub quiet: bool,
    /// `--csv-delimiter`.
    pub csv_delimiter: Option<String>,
    /// `--thousands-delimiter`: group digits in the text report.
    pub thousands_delimiter: Option<String>,
    /// `--by-percent` / `--percent`: show comment and blank as percentages.
    pub by_percent: Option<Denominator>,
    /// `--sum-one`: print the SUM row even for a single input file.
    pub sum_one: bool,
    /// `--summary-cutoff`: fold small languages into "Other".
    pub cutoff: Option<Cutoff>,
    /// `--fmt=N`: one of five alternate text layouts.
    pub fmt: Option<u8>,
    /// `--xsl`: stylesheet to reference from the XML output.
    pub xsl: Option<String>,
}

const VERSION: &str = env!("CARGO_PKG_VERSION");
const URL: &str = "https://github.com/cdxy1/rcloc";

pub fn render(report: &Report, format: Format, opts: &OutputOptions) -> String {
    match format {
        Format::Text => text(report, opts),
        Format::Json => json(report, opts),
        Format::Yaml => yaml(report, opts),
        Format::Csv => csv(report, opts),
        Format::Markdown => markdown(report, opts),
        Format::Xml => xml(report, opts),
    }
}

/// Group digits with the requested separator, e.g. `1234567` -> `1 234 567`.
fn number(n: usize, opts: &OutputOptions) -> String {
    let Some(sep) = &opts.thousands_delimiter else {
        return n.to_string();
    };
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push_str(sep);
        }
        out.push(c);
    }
    out
}

fn header(report: &Report, opts: &OutputOptions) -> String {
    if opts.quiet {
        return String::new();
    }
    let n = report.files.len();
    if opts.hide_rate || report.elapsed_secs <= 0.0 {
        format!("◉ rcloc {VERSION}\n  Fast source insights, powered by Rust\n\n")
    } else {
        format!(
            "◉ rcloc {VERSION}\n  {} files · {} lines · {:.2}s · {:.0} files/s · {:.0} lines/s\n\n",
            n,
            report.totals().counts.total(),
            report.elapsed_secs,
            n as f64 / report.elapsed_secs,
            report.totals().counts.total() as f64 / report.elapsed_secs,
        )
    }
}

// --- text ----------------------------------------------------------------- {{{1

/// Render one row's three count columns, as numbers or as percentages.
///
/// Under `--by-percent X` every column is a percentage of the same row-wise
/// denominator, so a row shows how it divides between blank, comment and
/// code. Under `--percent` (`X` = `t`) each column is instead a percentage of
/// that column's total across the whole report, so a row shows what share of
/// the project's blanks, comments and code it accounts for.
fn count_columns(
    counts: cloc_core::counter::Counts,
    totals: cloc_core::counter::Counts,
    opts: &OutputOptions,
) -> [String; 3] {
    let Some(denominator) = opts.by_percent else {
        return [
            number(counts.blank, opts),
            number(counts.comment, opts),
            number(counts.code, opts),
        ];
    };

    let pct = |n: usize, divisor: f64| {
        if divisor <= 0.0 {
            "0.00".to_string()
        } else {
            format!("{:.2}", 100.0 * n as f64 / divisor)
        }
    };

    if denominator == Denominator::ColumnTotal {
        return [
            pct(counts.blank, totals.blank as f64),
            pct(counts.comment, totals.comment as f64),
            pct(counts.code, totals.code as f64),
        ];
    }
    let divisor = denominator.of(counts);
    [
        pct(counts.blank, divisor),
        pct(counts.comment, divisor),
        pct(counts.code, divisor),
    ]
}

fn text(report: &Report, opts: &OutputOptions) -> String {
    // --fmt selects between by-language and by-file layouts, and whether a
    // total-lines column appears.
    let by_file = opts.by_file || matches!(opts.fmt, Some(3 | 4 | 5));
    let with_total = matches!(opts.fmt, Some(2 | 4));
    let opts = &OutputOptions {
        by_file,
        ..opts.clone()
    };
    let first_heading = if opts.by_file { "File" } else { "Language" };
    // Widen the first column to fit its longest entry, so long paths under
    // --by-file are not truncated.
    let first_width = report
        .files
        .iter()
        .map(|f| {
            if opts.by_file {
                f.path.to_string_lossy().chars().count()
            } else {
                f.language.chars().count()
            }
        })
        .chain(std::iter::once(first_heading.len()))
        .max()
        .unwrap_or(20)
        .max(20);

    let mut out = header(report, opts);

    // The headings gain a percent sign so the columns cannot be misread.
    let (blank_head, comment_head, code_head) = if opts.by_percent.is_some() {
        ("blank %", "comment %", "code %")
    } else {
        ("blank", "comment", "code")
    };
    let mut widths = vec![first_width];
    if !opts.by_file {
        widths.push(9);
    }
    widths.extend([14, 14, 14]);
    if with_total {
        widths.push(14);
    }

    table_border(&mut out, &widths, '┌', '┬', '┐');
    let mut headings = vec![first_heading.to_string()];
    if !opts.by_file {
        headings.push("files".to_string());
    }
    headings.extend([blank_head, comment_head, code_head].map(str::to_string));
    if with_total {
        headings.push("total".to_string());
    }
    table_row(&mut out, &headings, &widths);
    table_border(&mut out, &widths, '├', '┼', '┤');

    let overall = report.totals().counts;
    let row = |label: &str, totals: LanguageTotals, show_files: bool, out: &mut String| {
        let mut cells = vec![label.to_string()];
        if show_files {
            cells.push(number(totals.files, opts));
        }
        let [blank, comment, code] = count_columns(totals.counts, overall, opts);
        cells.extend([blank, comment, code]);
        if with_total {
            cells.push(number(totals.counts.total(), opts));
        }
        table_row(out, &cells, &widths);
    };

    if opts.by_file {
        for f in report.files_by_code() {
            row(
                &f.path.to_string_lossy(),
                LanguageTotals {
                    files: 1,
                    counts: f.counts,
                },
                false,
                &mut out,
            );
        }
    } else {
        for (lang, totals) in report.languages_by_code_with_cutoff(opts.cutoff) {
            row(&lang, totals, true, &mut out);
        }
    }

    // A single input file makes the SUM row pure repetition, so cloc leaves
    // it out unless asked.
    let show_sum = opts.sum_one || report.files.len() > 1;
    if show_sum {
        table_border(&mut out, &widths, '├', '┼', '┤');
        row("SUM:", report.totals(), !opts.by_file, &mut out);
    }
    table_border(&mut out, &widths, '└', '┴', '┘');
    out
}

/// Draw one horizontal edge of the terminal table.
fn table_border(out: &mut String, widths: &[usize], left: char, join: char, right: char) {
    out.push(left);
    for (index, width) in widths.iter().enumerate() {
        out.push_str(&"─".repeat(width + 2));
        out.push(if index + 1 == widths.len() {
            right
        } else {
            join
        });
    }
    out.push('\n');
}

/// Draw a row. The first cell is left-aligned; measurements are right-aligned.
fn table_row(out: &mut String, cells: &[String], widths: &[usize]) {
    out.push('│');
    for (index, (cell, width)) in cells.iter().zip(widths).enumerate() {
        if index == 0 {
            let _ = write!(out, " {cell:<width$} │", width = width);
        } else {
            let _ = write!(out, " {cell:>width$} │", width = width);
        }
    }
    out.push('\n');
}

// --- json ----------------------------------------------------------------- {{{1

fn json(report: &Report, opts: &OutputOptions) -> String {
    let mut out = String::from("{\n");

    if !opts.quiet {
        let _ = write!(
            out,
            "  \"header\" : {{\n    \"cloc_url\" : \"{URL}\",\n    \
             \"cloc_version\" : \"{VERSION}\",\n    \"elapsed_seconds\" : {},\n    \
             \"n_files\" : {},\n    \"n_lines\" : {}\n  }},\n",
            report.elapsed_secs,
            report.files.len(),
            report.totals().counts.total(),
        );
    }

    if opts.by_file {
        for f in report.files_by_code() {
            let _ = write!(
                out,
                "  {} : {{\n    \"blank\" : {},\n    \"comment\" : {},\n    \
                 \"code\" : {},\n    \"language\" : {}\n  }},\n",
                json_string(&f.path.to_string_lossy()),
                f.counts.blank,
                f.counts.comment,
                f.counts.code,
                json_string(&f.language),
            );
        }
    } else {
        for (lang, t) in report.languages_by_code_with_cutoff(opts.cutoff) {
            let _ = write!(
                out,
                "  {} : {{\n    \"nFiles\" : {},\n    \"blank\" : {},\n    \
                 \"comment\" : {},\n    \"code\" : {}\n  }},\n",
                json_string(&lang),
                t.files,
                t.counts.blank,
                t.counts.comment,
                t.counts.code,
            );
        }
    }

    let t = report.totals();
    let _ = write!(
        out,
        "  \"SUM\" : {{\n    \"blank\" : {},\n    \"comment\" : {},\n    \
         \"code\" : {},\n    \"nFiles\" : {}\n  }}\n}}\n",
        t.counts.blank, t.counts.comment, t.counts.code, t.files,
    );
    out
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// --- yaml ----------------------------------------------------------------- {{{1

fn yaml(report: &Report, opts: &OutputOptions) -> String {
    let mut out = String::from("---\n");
    if !opts.quiet {
        let _ = write!(
            out,
            "header :\n  cloc_url : {URL}\n  cloc_version : {VERSION}\n  \
             elapsed_seconds : {}\n  n_files : {}\n  n_lines : {}\n",
            report.elapsed_secs,
            report.files.len(),
            report.totals().counts.total(),
        );
    }
    if opts.by_file {
        for f in report.files_by_code() {
            let _ = write!(
                out,
                "'{}' :\n  blank : {}\n  comment : {}\n  code : {}\n  language : {}\n",
                f.path.to_string_lossy(),
                f.counts.blank,
                f.counts.comment,
                f.counts.code,
                f.language,
            );
        }
    } else {
        for (lang, t) in report.languages_by_code() {
            let _ = write!(
                out,
                "{lang} :\n  nFiles : {}\n  blank : {}\n  comment : {}\n  code : {}\n",
                t.files, t.counts.blank, t.counts.comment, t.counts.code,
            );
        }
    }
    let t = report.totals();
    let _ = write!(
        out,
        "SUM :\n  blank : {}\n  comment : {}\n  code : {}\n  nFiles : {}\n",
        t.counts.blank, t.counts.comment, t.counts.code, t.files,
    );
    out
}

// --- csv ------------------------------------------------------------------ {{{1

fn csv(report: &Report, opts: &OutputOptions) -> String {
    let d = opts.csv_delimiter.as_deref().unwrap_or(",");
    let mut out = String::new();
    if opts.by_file {
        let _ = writeln!(out, "language{d}filename{d}blank{d}comment{d}code");
        for f in report.files_by_code() {
            let _ = writeln!(
                out,
                "{}{d}{}{d}{}{d}{}{d}{}",
                csv_field(&f.language, d),
                csv_field(&f.path.to_string_lossy(), d),
                f.counts.blank,
                f.counts.comment,
                f.counts.code,
            );
        }
    } else {
        let _ = writeln!(out, "files{d}language{d}blank{d}comment{d}code");
        for (lang, t) in report.languages_by_code() {
            let _ = writeln!(
                out,
                "{}{d}{}{d}{}{d}{}{d}{}",
                t.files,
                csv_field(lang, d),
                t.counts.blank,
                t.counts.comment,
                t.counts.code,
            );
        }
    }
    let t = report.totals();
    let _ = writeln!(
        out,
        "{}{d}SUM{d}{}{d}{}{d}{}",
        t.files, t.counts.blank, t.counts.comment, t.counts.code
    );
    out
}

/// Quote a CSV field if it contains the delimiter or a quote.
fn csv_field(s: &str, delimiter: &str) -> String {
    if s.contains(delimiter) || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// --- markdown ------------------------------------------------------------- {{{1

fn markdown(report: &Report, opts: &OutputOptions) -> String {
    let mut out = header(report, opts);
    if !out.is_empty() {
        out.push('\n');
    }
    let first = if opts.by_file { "File" } else { "Language" };
    if opts.by_file {
        let _ = writeln!(out, "{first} | blank | comment | code");
        let _ = writeln!(out, ":------|------:|--------:|-----:");
        for f in report.files_by_code() {
            let _ = writeln!(
                out,
                "{} | {} | {} | {}",
                f.path.to_string_lossy(),
                f.counts.blank,
                f.counts.comment,
                f.counts.code
            );
        }
    } else {
        let _ = writeln!(out, "{first} | files | blank | comment | code");
        let _ = writeln!(out, ":------|------:|------:|--------:|-----:");
        for (lang, t) in report.languages_by_code() {
            let _ = writeln!(
                out,
                "{lang} | {} | {} | {} | {}",
                t.files, t.counts.blank, t.counts.comment, t.counts.code
            );
        }
    }
    let t = report.totals();
    let _ = writeln!(
        out,
        "SUM: | {} | {} | {} | {}",
        t.files, t.counts.blank, t.counts.comment, t.counts.code
    );
    out
}

// --- xml ------------------------------------------------------------------ {{{1

fn xml(report: &Report, opts: &OutputOptions) -> String {
    let mut out = String::from("<?xml version=\"1.0\"?>\n");
    // The stylesheet reference has to sit between the declaration and the
    // root element, which is why it is written here rather than appended.
    if let Some(sheet) = &opts.xsl {
        let _ = writeln!(
            out,
            "<?xml-stylesheet type=\"text/xsl\" href=\"{}\"?>",
            xml_escape(sheet)
        );
    }
    out.push_str("<results>\n");
    if !opts.quiet {
        let _ = write!(
            out,
            "<header>\n  <cloc_url>{URL}</cloc_url>\n  <cloc_version>{VERSION}</cloc_version>\n  \
             <elapsed_seconds>{}</elapsed_seconds>\n  <n_files>{}</n_files>\n  \
             <n_lines>{}</n_lines>\n</header>\n",
            report.elapsed_secs,
            report.files.len(),
            report.totals().counts.total(),
        );
    }
    if opts.by_file {
        out.push_str("<files>\n");
        for f in report.files_by_code() {
            let _ = writeln!(
                out,
                "  <file name=\"{}\" blank=\"{}\" comment=\"{}\" code=\"{}\" language=\"{}\" />",
                xml_escape(&f.path.to_string_lossy()),
                f.counts.blank,
                f.counts.comment,
                f.counts.code,
                xml_escape(&f.language),
            );
        }
    } else {
        out.push_str("<languages>\n");
        for (lang, t) in report.languages_by_code() {
            let _ = writeln!(
                out,
                "  <language name=\"{}\" files_count=\"{}\" blank=\"{}\" comment=\"{}\" code=\"{}\" />",
                xml_escape(lang),
                t.files,
                t.counts.blank,
                t.counts.comment,
                t.counts.code,
            );
        }
    }
    let t = report.totals();
    let _ = writeln!(
        out,
        "  <total sum_files=\"{}\" blank=\"{}\" comment=\"{}\" code=\"{}\" />",
        t.files, t.counts.blank, t.counts.comment, t.counts.code
    );
    out.push_str(if opts.by_file {
        "</files>\n</results>\n"
    } else {
        "</languages>\n</results>\n"
    });
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::FileEntry;
    use cloc_core::counter::Counts;
    use std::path::PathBuf;

    fn sample() -> Report {
        Report {
            files: vec![
                FileEntry {
                    path: PathBuf::from("src/main.rs"),
                    language: "Rust".to_string(),
                    counts: Counts {
                        blank: 2,
                        comment: 3,
                        code: 10,
                    },
                },
                FileEntry {
                    path: PathBuf::from("setup.py"),
                    language: "Python".to_string(),
                    counts: Counts {
                        blank: 1,
                        comment: 1,
                        code: 4,
                    },
                },
            ],
            ignored: vec![],
            elapsed_secs: 0.0,
        }
    }

    #[test]
    fn totals_sum_the_files() {
        let t = sample().totals();
        assert_eq!(t.files, 2);
        assert_eq!(
            t.counts,
            Counts {
                blank: 3,
                comment: 4,
                code: 14
            }
        );
    }

    /// Languages are ordered by code descending.
    #[test]
    fn language_order_is_by_code() {
        let r = sample();
        let order: Vec<&str> = r.languages_by_code().into_iter().map(|(l, _)| l).collect();
        assert_eq!(order, vec!["Rust", "Python"]);
    }

    #[test]
    fn text_report_lines_up() {
        let out = text(
            &sample(),
            &OutputOptions {
                quiet: true,
                ..Default::default()
            },
        );
        assert!(out.contains("Language"));
        assert!(out.contains("Rust"));
        assert!(out.contains("SUM:"));
        // Every rule and row must be the same width.
        let widths: Vec<usize> = out.lines().map(|l| l.chars().count()).collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "ragged: {widths:?}"
        );
    }

    #[test]
    fn json_is_parseable_and_carries_the_sum() {
        let out = json(
            &sample(),
            &OutputOptions {
                quiet: true,
                ..Default::default()
            },
        );
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
        assert_eq!(v["SUM"]["code"], 14);
        assert_eq!(v["Rust"]["nFiles"], 1);
    }

    #[test]
    fn json_by_file_keys_on_paths() {
        let opts = OutputOptions {
            by_file: true,
            quiet: true,
            ..Default::default()
        };
        let out = json(&sample(), &opts);
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
        assert_eq!(v["src/main.rs"]["language"], "Rust");
        assert_eq!(v["src/main.rs"]["code"], 10);
    }

    #[test]
    fn csv_has_a_header_and_a_sum() {
        let out = csv(&sample(), &OutputOptions::default());
        assert!(out.starts_with("files,language,blank,comment,code\n"));
        assert!(out.trim_end().ends_with("2,SUM,3,4,14"));
    }

    /// A delimiter appearing in a field must not break the columns.
    #[test]
    fn csv_quotes_fields_containing_the_delimiter() {
        assert_eq!(csv_field("a,b", ","), "\"a,b\"");
        assert_eq!(csv_field("plain", ","), "plain");
    }

    /// `--xsl` inserts the stylesheet reference after the declaration and
    /// before the root element, where a processor will look for it.
    #[test]
    fn xsl_reference_precedes_the_root_element() {
        let opts = OutputOptions {
            quiet: true,
            xsl: Some("cloc.xsl".to_string()),
            ..Default::default()
        };
        let out = xml(&sample(), &opts);
        let decl = out.find("<?xml version").unwrap();
        let sheet = out.find("<?xml-stylesheet").unwrap();
        let root = out.find("<results>").unwrap();
        assert!(decl < sheet && sheet < root);
        assert!(out.contains("href=\"cloc.xsl\""));
    }

    #[test]
    fn xml_escapes_special_characters() {
        assert_eq!(xml_escape("a<b & c\""), "a&lt;b &amp; c&quot;");
    }

    /// A single input file makes SUM pure repetition, so it is omitted
    /// unless --sum-one asks for it.
    #[test]
    fn sum_row_appears_only_for_multiple_files_unless_forced() {
        let mut one = sample();
        one.files.truncate(1);
        let plain = text(
            &one,
            &OutputOptions {
                quiet: true,
                ..Default::default()
            },
        );
        assert!(!plain.contains("SUM:"));

        let forced = text(
            &one,
            &OutputOptions {
                quiet: true,
                sum_one: true,
                ..Default::default()
            },
        );
        assert!(forced.contains("SUM:"));

        let two = text(
            &sample(),
            &OutputOptions {
                quiet: true,
                ..Default::default()
            },
        );
        assert!(two.contains("SUM:"));
    }

    /// `--percent` shows each column as a share of that column's total, so
    /// the SUM row reads 100 across the board.
    #[test]
    fn percent_columns_total_to_one_hundred() {
        let opts = OutputOptions {
            quiet: true,
            by_percent: Some(Denominator::ColumnTotal),
            ..Default::default()
        };
        let out = text(&sample(), &opts);
        assert!(out.contains("blank %"));
        assert!(out.lines().last().is_some());
        let sum_line = out.lines().find(|l| l.contains("│ SUM:")).unwrap();
        assert_eq!(sum_line.matches("100.00").count(), 3);
    }

    /// `--by-percent cmb` is row-wise instead: a row's three percentages
    /// account for that row's own lines.
    #[test]
    fn by_percent_is_row_wise() {
        let opts = OutputOptions {
            quiet: true,
            by_percent: Some(Denominator::All),
            ..Default::default()
        };
        let out = text(&sample(), &opts);
        let rust = out.lines().find(|l| l.contains("│ Rust")).unwrap();
        // 2 blank, 3 comment, 10 code out of 15.
        assert!(rust.contains("13.33"));
        assert!(rust.contains("20.00"));
        assert!(rust.contains("66.67"));
    }

    /// `--fmt 2` adds a total-lines column; `--fmt 3` switches to by-file.
    #[test]
    fn fmt_selects_the_layout() {
        let with_total = text(
            &sample(),
            &OutputOptions {
                quiet: true,
                fmt: Some(2),
                ..Default::default()
            },
        );
        assert!(with_total.contains("total"));
        assert!(with_total.contains("Language"));

        let by_file = text(
            &sample(),
            &OutputOptions {
                quiet: true,
                fmt: Some(3),
                ..Default::default()
            },
        );
        assert!(by_file.contains("File"));
        assert!(by_file.contains("src/main.rs"));
    }

    /// `--summary-cutoff` folds small languages into one "Other" row without
    /// changing the totals.
    #[test]
    fn cutoff_folds_small_languages_into_other() {
        let opts = OutputOptions {
            quiet: true,
            cutoff: Cutoff::parse("c:5"),
            ..Default::default()
        };
        let out = text(&sample(), &opts);
        assert!(out.contains("Other"));
        assert!(!out.contains("Python"));
        assert!(out.contains("Rust"));
        // The SUM is unchanged: folding moves rows, it does not drop them.
        let sum = out.lines().find(|l| l.contains("│ SUM:")).unwrap();
        assert!(sum.contains("14"));
    }

    #[test]
    fn thousands_delimiter_groups_digits() {
        let opts = OutputOptions {
            thousands_delimiter: Some(",".to_string()),
            ..Default::default()
        };
        assert_eq!(number(1234567, &opts), "1,234,567");
        assert_eq!(number(999, &opts), "999");
        assert_eq!(number(1000, &opts), "1,000");
    }
}

// --- diff reports --------------------------------------------------------- {{{1

use cloc_core::diff::Delta;
use cloc_core::diffmode::DiffReport;

/// Render a `--diff` report.
///
/// Every format shows the same four dispositions -- same, modified, added,
/// removed -- as four blocks, because a diff has no single number to put in a
/// column the way a plain count does.
pub fn render_diff(report: &DiffReport, format: Format, opts: &OutputOptions) -> String {
    match format {
        Format::Json => diff_json(report, opts),
        Format::Csv => diff_csv(report, opts),
        _ => diff_text(report, opts),
    }
}

/// The four dispositions, in the order cloc prints them.
const DISPOSITIONS: [&str; 4] = ["same", "modified", "added", "removed"];

fn pick(delta: Delta, which: &str) -> usize {
    match which {
        "same" => delta.same,
        "modified" => delta.modified,
        "added" => delta.added,
        _ => delta.removed,
    }
}

fn diff_text(report: &DiffReport, opts: &OutputOptions) -> String {
    let width = report
        .by_language
        .keys()
        .map(|l| l.chars().count())
        .chain(std::iter::once("Language".len()))
        .max()
        .unwrap_or(20)
        .max(20);
    let rule = "-".repeat(width + 9 + 14 * 3);

    let mut out = String::new();
    if !opts.quiet {
        out.push_str(&format!("{URL} v {VERSION}\n"));
    }
    out.push_str(&rule);
    out.push('\n');
    let _ = writeln!(
        out,
        "{:<width$}{:>9}{:>14}{:>14}{:>14}",
        "Language", "files", "blank", "comment", "code"
    );
    out.push_str(&rule);
    out.push('\n');

    // A language heads its own block and the four dispositions sit under it,
    // which reads better than four tables each listing every language.
    let mut totals: [(usize, usize, usize, usize); 4] = Default::default();

    for (language, d) in &report.by_language {
        let _ = writeln!(out, "{language}");
        for (n, which) in DISPOSITIONS.iter().enumerate() {
            let row = (
                pick(d.files, which),
                pick(d.delta.blank, which),
                pick(d.delta.comment, which),
                pick(d.delta.code, which),
            );
            let _ = writeln!(
                out,
                " {:<w$}{:>9}{:>14}{:>14}{:>14}",
                which,
                number(row.0, opts),
                number(row.1, opts),
                number(row.2, opts),
                number(row.3, opts),
                w = width - 1
            );
            totals[n].0 += row.0;
            totals[n].1 += row.1;
            totals[n].2 += row.2;
            totals[n].3 += row.3;
        }
    }

    out.push_str(&rule);
    out.push('\n');
    let _ = writeln!(out, "SUM:");
    for (n, which) in DISPOSITIONS.iter().enumerate() {
        let _ = writeln!(
            out,
            " {:<w$}{:>9}{:>14}{:>14}{:>14}",
            which,
            number(totals[n].0, opts),
            number(totals[n].1, opts),
            number(totals[n].2, opts),
            number(totals[n].3, opts),
            w = width - 1
        );
    }
    out.push_str(&rule);
    out.push('\n');
    out
}

fn diff_json(report: &DiffReport, opts: &OutputOptions) -> String {
    let mut out = String::from("{\n");
    let mut blocks: Vec<String> = Vec::new();

    for which in DISPOSITIONS {
        let mut rows: Vec<String> = Vec::new();
        let mut totals = (0usize, 0usize, 0usize, 0usize);

        if opts.by_file {
            for (path, _language, delta) in &report.by_file {
                let row = (
                    0,
                    pick(delta.blank, which),
                    pick(delta.comment, which),
                    pick(delta.code, which),
                );
                if row.1 == 0 && row.2 == 0 && row.3 == 0 {
                    continue;
                }
                rows.push(format!(
                    "      {} : {{\n        \"blank\" : {},\n        \"comment\" : {},\n        \"code\" : {}\n      }}",
                    json_string(&path.to_string_lossy()),
                    row.1,
                    row.2,
                    row.3
                ));
                totals.1 += row.1;
                totals.2 += row.2;
                totals.3 += row.3;
            }
        } else {
            for (language, d) in &report.by_language {
                let row = (
                    pick(d.files, which),
                    pick(d.delta.blank, which),
                    pick(d.delta.comment, which),
                    pick(d.delta.code, which),
                );
                if row == (0, 0, 0, 0) {
                    continue;
                }
                rows.push(format!(
                    "      {} : {{\n        \"nFiles\" : {},\n        \"blank\" : {},\n        \"comment\" : {},\n        \"code\" : {}\n      }}",
                    json_string(language),
                    row.0,
                    row.1,
                    row.2,
                    row.3
                ));
                totals.0 += row.0;
                totals.1 += row.1;
                totals.2 += row.2;
                totals.3 += row.3;
            }
        }

        rows.push(format!(
            "      \"SUM\" : {{\n        \"nFiles\" : {},\n        \"blank\" : {},\n        \"comment\" : {},\n        \"code\" : {}\n      }}",
            totals.0, totals.1, totals.2, totals.3
        ));
        blocks.push(format!("  \"{which}\" : {{\n{}\n  }}", rows.join(",\n")));
    }

    out.push_str(&blocks.join(",\n"));
    out.push_str("\n}\n");
    out
}

fn diff_csv(report: &DiffReport, opts: &OutputOptions) -> String {
    let d = opts.csv_delimiter.as_deref().unwrap_or(",");
    let mut out = String::new();
    let _ = writeln!(
        out,
        "disposition{d}language{d}files{d}blank{d}comment{d}code"
    );
    for which in DISPOSITIONS {
        for (language, counts) in &report.by_language {
            let row = (
                pick(counts.files, which),
                pick(counts.delta.blank, which),
                pick(counts.delta.comment, which),
                pick(counts.delta.code, which),
            );
            if row == (0, 0, 0, 0) {
                continue;
            }
            let _ = writeln!(
                out,
                "{which}{d}{}{d}{}{d}{}{d}{}{d}{}",
                csv_field(language, d),
                row.0,
                row.1,
                row.2,
                row.3
            );
        }
    }
    out
}
