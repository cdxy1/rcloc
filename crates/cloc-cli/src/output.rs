//! Rendering a report in each supported format.

use crate::report::{LanguageTotals, Report};
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
}

const VERSION: &str = env!("CARGO_PKG_VERSION");
const URL: &str = "github.com/AlDanial/cloc";

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
        format!("{URL} v {VERSION}\n")
    } else {
        format!(
            "{URL} v {VERSION}  T={:.2} s ({:.1} files/s, {:.1} lines/s)\n",
            report.elapsed_secs,
            n as f64 / report.elapsed_secs,
            report.totals().counts.total() as f64 / report.elapsed_secs,
        )
    }
}

// --- text ----------------------------------------------------------------- {{{1

fn text(report: &Report, opts: &OutputOptions) -> String {
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

    let cols = [("files", 9usize), ("blank", 14), ("comment", 14), ("code", 14)];
    let total_width = first_width + cols.iter().map(|c| c.1).sum::<usize>();
    let rule = "-".repeat(total_width);

    let mut out = header(report, opts);
    out.push_str(&rule);
    out.push('\n');

    // --by-file has no per-row file count, so that column is dropped.
    let _ = write!(out, "{:<width$}", first_heading, width = first_width);
    if !opts.by_file {
        let _ = write!(out, "{:>9}", "files");
    }
    let _ = writeln!(out, "{:>14}{:>14}{:>14}", "blank", "comment", "code");
    out.push_str(&rule);
    out.push('\n');

    let row = |label: &str, totals: LanguageTotals, show_files: bool, out: &mut String| {
        let _ = write!(out, "{:<width$}", label, width = first_width);
        if show_files {
            let _ = write!(out, "{:>9}", number(totals.files, opts));
        }
        let _ = writeln!(
            out,
            "{:>14}{:>14}{:>14}",
            number(totals.counts.blank, opts),
            number(totals.counts.comment, opts),
            number(totals.counts.code, opts),
        );
    };

    if opts.by_file {
        for f in report.files_by_code() {
            row(
                &f.path.to_string_lossy(),
                LanguageTotals { files: 1, counts: f.counts },
                false,
                &mut out,
            );
        }
    } else {
        for (lang, totals) in report.languages_by_code() {
            row(lang, totals, true, &mut out);
        }
    }

    out.push_str(&rule);
    out.push('\n');
    row("SUM:", report.totals(), !opts.by_file, &mut out);
    out.push_str(&rule);
    out.push('\n');
    out
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
        for (lang, t) in report.languages_by_code() {
            let _ = write!(
                out,
                "  {} : {{\n    \"nFiles\" : {},\n    \"blank\" : {},\n    \
                 \"comment\" : {},\n    \"code\" : {}\n  }},\n",
                json_string(lang),
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
    let mut out = String::from("<?xml version=\"1.0\"?>\n<results>\n");
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
                    counts: Counts { blank: 2, comment: 3, code: 10 },
                },
                FileEntry {
                    path: PathBuf::from("setup.py"),
                    language: "Python".to_string(),
                    counts: Counts { blank: 1, comment: 1, code: 4 },
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
        assert_eq!(t.counts, Counts { blank: 3, comment: 4, code: 14 });
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
        let out = text(&sample(), &OutputOptions { quiet: true, ..Default::default() });
        assert!(out.contains("Language"));
        assert!(out.contains("Rust"));
        assert!(out.contains("SUM:"));
        // Every rule and row must be the same width.
        let widths: Vec<usize> = out.lines().map(|l| l.chars().count()).collect();
        assert!(widths.windows(2).all(|w| w[0] == w[1]), "ragged: {widths:?}");
    }

    #[test]
    fn json_is_parseable_and_carries_the_sum() {
        let out = json(&sample(), &OutputOptions { quiet: true, ..Default::default() });
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
        assert_eq!(v["SUM"]["code"], 14);
        assert_eq!(v["Rust"]["nFiles"], 1);
    }

    #[test]
    fn json_by_file_keys_on_paths() {
        let opts = OutputOptions { by_file: true, quiet: true, ..Default::default() };
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

    #[test]
    fn xml_escapes_special_characters() {
        assert_eq!(xml_escape("a<b & c\""), "a&lt;b &amp; c&quot;");
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
