//! `--sql`: results as SQL statements, for loading into SQLite or Oracle.
//!
//! One row per file rather than per language, so the database can be asked
//! questions the text report cannot answer.

use crate::report::Report;
use cloc_lang::LangDb;
use std::fmt::Write as _;

/// Which dialect to emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlStyle {
    /// SQLite and friends: `begin transaction` ... `commit`.
    Default,
    /// As above, but each INSERT names its columns.
    NamedColumns,
    /// Oracle: `TO_TIMESTAMP`, `VARCHAR2`, no transaction wrapper.
    Oracle,
}

impl SqlStyle {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.to_lowercase().as_str() {
            "" | "default" | "sqlite" => Self::Default,
            "named_columns" | "named-columns" => Self::NamedColumns,
            "oracle" => Self::Oracle,
            _ => return None,
        })
    }
}

/// Options for the SQL writer.
#[derive(Debug, Clone)]
pub struct SqlOptions {
    pub style: SqlStyle,
    /// `--sql-append`: add rows to an existing file, so no schema is emitted.
    pub append: bool,
    /// `--sql-project`: the identifier rows are tagged with.
    pub project: String,
    /// Seconds the run took, recorded in the metadata row.
    pub elapsed_secs: f64,
    /// Unix timestamp used as the run's id.
    pub id: i64,
    /// Human-readable timestamp for the metadata row.
    pub timestamp: String,
}

/// Render a report as SQL.
pub fn render(report: &Report, db: &LangDb, opts: &SqlOptions) -> String {
    let mut out = String::new();

    // With --sql-append the tables already exist; emitting the schema again
    // would fail the load.
    if !opts.append {
        out.push_str(schema(opts.style));
    }

    match opts.style {
        SqlStyle::Oracle => {
            let _ = writeln!(
                out,
                "insert into metadata values({}, TO_TIMESTAMP('{}','yyyy-mm-dd hh24:mi:ss'), '{}', {:.6});",
                opts.id,
                opts.timestamp,
                quote(&opts.project),
                opts.elapsed_secs,
            );
        }
        SqlStyle::NamedColumns => out.push_str("begin transaction;\n"),
        SqlStyle::Default => {
            out.push_str("begin transaction;\n");
            let _ = writeln!(
                out,
                "insert into metadata values({}, '{}', '{}', {:.6});",
                opts.id,
                opts.timestamp,
                quote(&opts.project),
                opts.elapsed_secs,
            );
        }
    }

    let insert = match opts.style {
        SqlStyle::NamedColumns => {
            "insert into t (id, Project, Language, File, File_dirname, \
             File_basename, nBlank, nComment, nCode, nScaled ) values"
        }
        _ => "insert into t values",
    };

    for (n, entry) in report.files_by_code().iter().enumerate() {
        let path = entry.path.to_string_lossy();
        let dirname = entry
            .path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| ".".to_string());
        let basename = entry
            .path
            .file_name()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        // The scaled figure is code weighted by the language's COCOMO-style
        // factor, which is what makes languages comparable.
        let scaled = entry.counts.code as f64 * db.scale_factor(&entry.language).unwrap_or(1.0);

        let _ = writeln!(
            out,
            "{insert}({}, '{}', '{}', '{}', '{}', '{}', {}, {}, {}, {:.6});",
            opts.id,
            quote(&opts.project),
            quote(&entry.language),
            quote(&path),
            quote(&dirname),
            quote(&basename),
            entry.counts.blank,
            entry.counts.comment,
            entry.counts.code,
            scaled,
        );

        // Long runs are broken into transactions so a failure does not
        // discard everything.
        if opts.style != SqlStyle::Oracle && (n + 1) % 10_000 == 0 {
            out.push_str("commit;\nbegin transaction;\n");
        }
    }

    if opts.style != SqlStyle::Oracle {
        out.push_str("commit;\n");
    }
    out
}

/// Double any embedded single quote, which is how SQL escapes it.
fn quote(s: &str) -> String {
    s.replace('\'', "''")
}

fn schema(style: SqlStyle) -> &'static str {
    match style {
        SqlStyle::Oracle => {
            "
CREATE TABLE metadata
(
  id          INTEGER PRIMARY KEY,
  timestamp   TIMESTAMP,
  project     VARCHAR2(500 CHAR),
  elapsed_s   NUMBER(10, 6)
)
/

CREATE TABLE t
(
  id             INTEGER           ,
  project        VARCHAR2(500 CHAR),
  language       VARCHAR2(500 CHAR),
  file_fullname  VARCHAR2(500 CHAR),
  file_dirname   VARCHAR2(500 CHAR),
  file_basename  VARCHAR2(500 CHAR),
  nblank         INTEGER,
  ncomment       INTEGER,
  ncode          INTEGER,
  nscaled        NUMBER(10, 6),
FOREIGN KEY (id)
    REFERENCES metadata (id)
)
/

"
        }
        _ => {
            "
create table metadata (
                id        integer primary key,
                timestamp varchar(500),
                Project   varchar(500),
                elapsed_s real);
create table t        (
                id            integer        ,
                Project       varchar(500)   ,
                Language      varchar(500)   ,
                File          varchar(500)   ,
                File_dirname  varchar(500)   ,
                File_basename varchar(500)   ,
                nBlank        integer        ,
                nComment      integer        ,
                nCode         integer        ,
                nScaled       real           ,
        foreign key (id)
            references metadata (id));
"
        }
    }
}

/// A project identifier defaulting to the working directory, as cloc does.
pub fn default_project() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::FileEntry;
    use cloc_core::counter::Counts;
    use std::path::PathBuf;

    fn sample() -> Report {
        Report {
            files: vec![FileEntry {
                path: PathBuf::from("src/main.rs"),
                language: "Rust".to_string(),
                counts: Counts { blank: 2, comment: 3, code: 10 },
            }],
            ignored: vec![],
            elapsed_secs: 0.5,
        }
    }

    fn opts(style: SqlStyle, append: bool) -> SqlOptions {
        SqlOptions {
            style,
            append,
            project: "proj".to_string(),
            elapsed_secs: 0.5,
            id: 1700000000,
            timestamp: "2026-07-27 12:00:00".to_string(),
        }
    }

    #[test]
    fn default_style_wraps_inserts_in_a_transaction() {
        let out = render(&sample(), LangDb::default_db(), &opts(SqlStyle::Default, false));
        assert!(out.contains("create table metadata"));
        assert!(out.contains("begin transaction;"));
        assert!(out.contains("insert into t values("));
        assert!(out.trim_end().ends_with("commit;"));
    }

    /// --sql-append adds rows to an existing database, so re-emitting the
    /// schema would break the load.
    #[test]
    fn append_omits_the_schema() {
        let out = render(&sample(), LangDb::default_db(), &opts(SqlStyle::Default, true));
        assert!(!out.contains("create table"));
        assert!(out.contains("insert into t values("));
    }

    #[test]
    fn named_columns_lists_the_columns() {
        let out = render(
            &sample(),
            LangDb::default_db(),
            &opts(SqlStyle::NamedColumns, false),
        );
        assert!(out.contains("insert into t (id, Project, Language, File,"));
    }

    #[test]
    fn oracle_uses_its_own_schema_and_no_transactions() {
        let out = render(&sample(), LangDb::default_db(), &opts(SqlStyle::Oracle, false));
        assert!(out.contains("VARCHAR2(500 CHAR)"));
        assert!(out.contains("TO_TIMESTAMP"));
        assert!(!out.contains("begin transaction"));
    }

    /// A quote in a path would otherwise end the string literal early.
    #[test]
    fn single_quotes_are_doubled() {
        assert_eq!(quote("it's"), "it''s");
        let mut report = sample();
        report.files[0].path = PathBuf::from("src/o'brien.rs");
        let out = render(&report, LangDb::default_db(), &opts(SqlStyle::Default, false));
        assert!(out.contains("o''brien.rs"));
    }

    /// The scaled column is code weighted by the language's factor.
    #[test]
    fn scaled_column_applies_the_language_factor() {
        let db = LangDb::default_db();
        let factor = db.scale_factor("Rust").unwrap();
        let out = render(&sample(), db, &opts(SqlStyle::Default, false));
        assert!(out.contains(&format!("{:.6}", 10.0 * factor)));
    }
}
