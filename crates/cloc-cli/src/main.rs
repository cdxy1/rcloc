//! `cloc-rs` — count lines of code.

mod output;
mod report;
mod sql;

use anyhow::{bail, Context, Result};
use clap::Parser;
use cloc_core::archive::{self, ArchiveOptions};
use cloc_core::classify::{self, Classification, ClassifyOptions};
use cloc_core::counter::{self, CountOptions};
use cloc_core::diffmode::{self, DiffOptions};
use cloc_core::git;
use cloc_core::dedupe;
use cloc_core::filters::FilterOptions;
use cloc_core::vcs;
use cloc_core::walk::{self, WalkOptions};
use cloc_lang::LangDb;
use output::{Format, OutputOptions};
use rayon::prelude::*;
use report::{FileEntry, Report};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "cloc-rs",
    version,
    about = "Count blank lines, comment lines, and lines of source code"
)]
struct Cli {
    /// Files and directories to count.
    #[arg(value_name = "PATH")]
    inputs: Vec<PathBuf>,

    // --- reporting ---
    /// Report counts for each file rather than each language.
    #[arg(long = "by-file", alias = "by_file")]
    by_file: bool,
    /// Suppress the header.
    #[arg(long)]
    quiet: bool,
    /// Omit the processing rate from the header.
    #[arg(long = "hide-rate", alias = "hide_rate")]
    hide_rate: bool,
    /// Write the report to this file instead of standard output.
    #[arg(long = "report-file", alias = "out", value_name = "FILE")]
    report_file: Option<PathBuf>,

    // --- formats ---
    /// Write JSON.
    #[arg(long)]
    json: bool,
    /// Write YAML.
    #[arg(long)]
    yaml: bool,
    /// Write CSV.
    #[arg(long)]
    csv: bool,
    /// Write Markdown.
    #[arg(long)]
    md: bool,
    /// Write XML.
    #[arg(long)]
    xml: bool,
    /// Delimiter for CSV output.
    #[arg(long = "csv-delimiter", value_name = "CHAR")]
    csv_delimiter: Option<String>,
    /// Group digits in the text report.
    #[arg(long = "thousands-delimiter", alias = "ksep", value_name = "CHAR")]
    thousands_delimiter: Option<String>,
    /// Show comment and blank counts as percentages of X: t, c, cm, cb, cmb.
    #[arg(long = "by-percent", alias = "by_percent", value_name = "X")]
    by_percent: Option<String>,
    /// Same as --by-percent t.
    #[arg(long)]
    percent: bool,
    /// Show the SUM row even for a single input file.
    #[arg(long = "sum-one", alias = "sum_one")]
    sum_one: bool,
    /// Fold languages below a threshold into "Other", as X:N or X:N%.
    #[arg(long = "summary-cutoff", alias = "summary_cutoff", value_name = "X:N")]
    summary_cutoff: Option<String>,
    /// Alternate text layout, 1 to 5.
    #[arg(long, value_name = "N")]
    fmt: Option<u8>,
    /// Write SQL statements to FILE, or to standard output for `-`.
    #[arg(long, value_name = "FILE")]
    sql: Option<String>,
    /// Emit only INSERTs, for adding to an existing database.
    #[arg(long = "sql-append", alias = "sql_append")]
    sql_append: bool,
    /// Project identifier recorded with each row.
    #[arg(long = "sql-project", alias = "sql_project", value_name = "NAME")]
    sql_project: Option<String>,
    /// SQL dialect: default, named_columns or oracle.
    #[arg(long = "sql-style", alias = "sql_style", value_name = "STYLE")]
    sql_style: Option<String>,

    // --- selecting files ---
    /// Comma-separated directory names to skip.
    #[arg(long = "exclude-dir", alias = "exclude_dir", value_name = "D1,D2")]
    exclude_dir: Option<String>,
    /// Comma-separated extensions to skip.
    #[arg(long = "exclude-ext", alias = "exclude_ext", value_name = "E1,E2")]
    exclude_ext: Option<String>,
    /// Comma-separated languages to skip.
    #[arg(long = "exclude-lang", alias = "exclude_lang", value_name = "L1,L2")]
    exclude_lang: Option<String>,
    /// Count only these comma-separated languages.
    #[arg(long = "include-lang", alias = "include_lang", value_name = "L1,L2")]
    include_lang: Option<String>,
    /// Count only these comma-separated extensions.
    #[arg(long = "include-ext", alias = "include_ext", value_name = "E1,E2")]
    include_ext: Option<String>,
    /// Count only files whose name matches this regex.
    #[arg(long = "match-f", alias = "match_f", value_name = "REGEX")]
    match_f: Option<String>,
    /// Skip files whose name matches this regex; repeatable.
    #[arg(long = "not-match-f", alias = "not_match_f", value_name = "REGEX")]
    not_match_f: Vec<String>,
    /// Count only directories matching this regex.
    #[arg(long = "match-d", alias = "match_d", value_name = "REGEX")]
    match_d: Option<String>,
    /// Skip directories matching this regex; repeatable.
    #[arg(long = "not-match-d", alias = "not_match_d", value_name = "REGEX")]
    not_match_d: Vec<String>,
    /// Match the file regexes against the whole path.
    #[arg(long)]
    fullpath: bool,
    /// Do not descend into subdirectories.
    #[arg(long = "no-recurse", alias = "no_recurse")]
    no_recurse: bool,
    /// Follow symbolic links.
    #[arg(long = "follow-links", alias = "follow_links")]
    follow_links: bool,
    /// Count files containing NUL bytes.
    #[arg(long = "read-binary-files", alias = "read_binary_files")]
    read_binary_files: bool,
    /// Skip files larger than this many megabytes.
    #[arg(long = "max-file-size", alias = "max_file_size", value_name = "MB")]
    max_file_size: Option<f64>,
    /// Skip files whose name begins with a dot.
    #[arg(long = "skip-win-hidden", alias = "skip_win_hidden")]
    skip_hidden: bool,

    // --- counting ---
    /// Strip comment markers inside string literals before counting.
    #[arg(long = "strip-str-comments", alias = "strip_str_comments")]
    strip_str_comments: bool,
    /// Count the code portion of a line that also has a comment.
    #[arg(long)]
    inline: bool,
    /// Treat Python docstrings as code.
    #[arg(long = "docstring-as-code", alias = "docstring_as_code")]
    docstring_as_code: bool,
    /// Skip machine-generated files.
    #[arg(long = "no-autogen", alias = "no_autogen")]
    no_autogen: bool,
    /// Drop N leading lines, optionally only for the listed extensions.
    #[arg(long = "skip-leading", alias = "skip_leading", value_name = "N[,ext]")]
    skip_leading: Option<String>,
    /// Remove lines matching this regex from the count; repeatable.
    #[arg(long = "ignore-regex", alias = "ignore_regex", value_name = "REGEX")]
    ignore_regex: Vec<String>,
    /// Treat `foo.c.in` as `foo.c`.
    #[arg(long)]
    autoconf: bool,
    /// Fold extensions to lower case when matching.
    #[arg(long = "ignore-case-ext", alias = "ignore_case_ext")]
    ignore_case_ext: bool,
    /// Language to assume for files with no extension.
    #[arg(long = "lang-no-ext", alias = "lang_no_ext", value_name = "LANG")]
    lang_no_ext: Option<String>,
    /// Compare two trees and report what changed between them.
    #[arg(long)]
    diff: bool,
    /// Count each input separately, then diff them.
    #[arg(long = "count-and-diff", alias = "count_and_diff")]
    count_and_diff: bool,
    /// Read inputs as git revisions rather than paths.
    #[arg(long)]
    git: bool,
    /// Diff two git revisions, comparing only the files that changed.
    #[arg(long = "git-diff-rel", alias = "git_diff_rel")]
    git_diff_rel: bool,
    /// Diff two git revisions, comparing every file.
    #[arg(long = "git-diff-all", alias = "git_diff_all")]
    git_diff_all: bool,
    /// Write the file pairing used by --diff to this file.
    #[arg(long = "diff-alignment", alias = "diff_alignment", value_name = "FILE")]
    diff_alignment: Option<PathBuf>,
    /// Compare with all whitespace removed.
    #[arg(long = "ignore-whitespace", alias = "ignore_whitespace")]
    ignore_whitespace: bool,
    /// Compare case-insensitively.
    #[arg(long = "ignore-case", alias = "ignore_case")]
    ignore_case: bool,
    /// Take the list of files and directories from FILE, one per line.
    #[arg(long = "list-file", alias = "list_file", value_name = "FILE")]
    list_file: Option<PathBuf>,
    /// Take the file list from a version control system: git, svn, auto, or
    /// any command that prints one path per line.
    #[arg(long, alias = "files-from", value_name = "VCS")]
    vcs: Option<String>,
    /// Include code in git submodules.
    #[arg(long = "include-submodules", alias = "include_submodules")]
    include_submodules: bool,
    /// Unpack archives with this command; `>FILE<` stands for the archive.
    #[arg(long = "extract-with", alias = "extract_with", value_name = "CMD")]
    extract_with: Option<String>,
    /// Skip input files whose name matches this regex before unpacking.
    #[arg(long = "skip-archive", alias = "skip_archive", value_name = "REGEX")]
    skip_archive: Option<String>,
    /// Unpack archives here instead of a temporary directory.
    #[arg(long, value_name = "DIR")]
    sdir: Option<PathBuf>,
    /// Count files whose content duplicates another file.
    #[arg(long = "skip-uniqueness", alias = "skip_uniqueness")]
    skip_uniqueness: bool,
    /// Number of worker threads; 0 uses one per core.
    #[arg(long, value_name = "N", default_value_t = 0)]
    processes: usize,

    // --- information ---
    /// Add language definitions from this file to the built-in ones.
    #[arg(long = "read-lang-def", alias = "read_lang_def", value_name = "FILE")]
    read_lang_def: Option<PathBuf>,
    /// Use only the language definitions in this file.
    #[arg(long = "force-lang-def", alias = "force_lang_def", value_name = "FILE")]
    force_lang_def: Option<PathBuf>,
    /// Write the built-in language definitions to this file.
    #[arg(long = "write-lang-def", alias = "write_lang_def", value_name = "FILE")]
    write_lang_def: Option<PathBuf>,
    /// As --write-lang-def, but also emit extensions shared between languages.
    #[arg(
        long = "write-lang-def-incl-dup",
        alias = "write_lang_def_incl_dup",
        value_name = "FILE"
    )]
    write_lang_def_incl_dup: Option<PathBuf>,
    /// Count LANG[,EXT]: with an extension, claim it; without, count
    /// everything as LANG. Repeatable.
    #[arg(long = "force-lang", alias = "force_lang", value_name = "LANG[,EXT]")]
    force_lang: Vec<String>,
    /// Treat files whose `#!` names INTERP as LANG, given as LANG,INTERP
    /// (e.g. Perl,perl5.8.8). Repeatable.
    #[arg(long = "script-lang", alias = "script_lang", value_name = "LANG,INTERP")]
    script_lang: Vec<String>,

    /// List the recognised languages and their extensions.
    #[arg(long = "show-lang", alias = "show_lang")]
    show_lang: bool,
    /// List the recognised extensions.
    #[arg(long = "show-ext", alias = "show_ext")]
    show_ext: bool,
    /// Explain how a language's comments are recognised.
    #[arg(long, value_name = "LANG")]
    explain: Option<String>,
    /// Write the list of skipped files, and why, to this file.
    #[arg(long, value_name = "FILE")]
    ignored: Option<PathBuf>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("cloc-rs: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let db = build_language_db(&cli)?;
    let db = &db;

    if let Some(path) = &cli.write_lang_def {
        return write_definitions(db, path, false);
    }
    if let Some(path) = &cli.write_lang_def_incl_dup {
        return write_definitions(db, path, true);
    }
    if cli.show_lang {
        print_languages(db);
        return Ok(());
    }
    if cli.show_ext {
        print_extensions(db);
        return Ok(());
    }
    if let Some(lang) = &cli.explain {
        return explain(db, lang);
    }
    if cli.inputs.is_empty() {
        bail!("no input files or directories given (try --help)");
    }

    if cli.processes > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(cli.processes)
            .build_global()
            .context("configuring the thread pool")?;
    }

    // Several options imply --diff, having nothing else to mean.
    if cli.diff || cli.diff_alignment.is_some() || cli.git_diff_rel || cli.git_diff_all {
        return run_diff(&cli, db);
    }
    if cli.count_and_diff {
        return run_count_and_diff(&cli, db);
    }

    let started = Instant::now();
    let mut report = count(&cli, db)?;
    report.elapsed_secs = started.elapsed().as_secs_f64();

    // --sql is a whole different shape of output: one row per file, with a
    // metadata row, rather than a table of languages.
    let rendered = match &cli.sql {
        Some(target) => {
            let style = cli.sql_style.as_deref().unwrap_or("");
            let style = sql::SqlStyle::parse(style)
                .with_context(|| format!("--sql-style expects default, named_columns or oracle, got {style:?}"))?;
            let text = sql::render(
                &report,
                db,
                &sql::SqlOptions {
                    style,
                    append: cli.sql_append,
                    project: cli
                        .sql_project
                        .clone()
                        .unwrap_or_else(sql::default_project),
                    elapsed_secs: report.elapsed_secs,
                    id: now_unix(),
                    timestamp: now_timestamp(),
                },
            );
            if target != "-" {
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(cli.sql_append)
                    .write(true)
                    .truncate(!cli.sql_append)
                    .open(target)
                    .with_context(|| format!("writing {target}"))?;
                use std::io::Write as _;
                file.write_all(text.as_bytes())?;
                return Ok(());
            }
            text
        }
        None => output::render(&report, format_of(&cli), &output_options(&cli)?),
    };

    match &cli.report_file {
        Some(path) => {
            std::fs::write(path, rendered).with_context(|| format!("writing {}", path.display()))?
        }
        None => print!("{rendered}"),
    }
    Ok(())
}

/// Build the language definitions this run will use, applying whichever of
/// the definition options were given.
fn build_language_db(cli: &Cli) -> Result<LangDb> {
    let mut db = LangDb::from_json(LangDb::embedded_json())?;

    // --force-lang-def replaces the built-ins outright; --read-lang-def adds
    // to them. Giving both is the user asking for two different things.
    if let Some(path) = &cli.force_lang_def {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        db.replace_definitions(&cloc_lang::langdef::parse(&text)?)?;
    }
    if let Some(path) = &cli.read_lang_def {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        db.merge_definitions(&cloc_lang::langdef::parse(&text)?)?;
    }

    for pair in &cli.force_lang {
        let (name, extension) = match pair.split_once(',') {
            Some((n, e)) => (n, Some(e)),
            None => (pair.as_str(), None),
        };
        let Some(language) = db.canonical_language(name).map(str::to_string) else {
            bail!("--force-lang: unknown language {name:?}; try --show-lang");
        };
        match extension {
            Some(ext) => db.force_extension(ext, &language),
            // Without an extension this claims every file in the run.
            None => db.force_all(&language),
        }
    }

    for pair in &cli.script_lang {
        // The language comes first, then the interpreter -- the opposite
        // order to --force-lang=LANG,EXT, but it is what cloc accepts.
        let Some((name, interpreter)) = pair.split_once(',') else {
            bail!("--script-lang expects LANG,INTERP, got {pair:?}");
        };
        let Some(language) = db.canonical_language(name).map(str::to_string) else {
            bail!("--script-lang: unknown language {name:?}; try --show-lang");
        };
        db.force_script(interpreter, &language);
    }

    Ok(db)
}

fn write_definitions(db: &LangDb, path: &std::path::Path, include_dup: bool) -> Result<()> {
    let text = cloc_lang::langdef::write(&db.to_definitions(include_dup));
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

/// Seconds since the epoch, used as the id tying a run's SQL rows together.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `YYYY-MM-DD HH:MM:SS` in UTC, computed without pulling in a date library.
fn now_timestamp() -> String {
    let secs = now_unix();
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // Civil-from-days, shifting the epoch to 1 March 0000 so leap days fall
    // at the end of the cycle and the month arithmetic stays branch-free.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

/// Resolve an input that may name a git revision instead of a path.
///
/// The export is returned alongside so the caller can keep it alive; a
/// dropped export takes its temporary directory with it.
fn resolve_input(input: &Path, force_git: bool) -> Result<(PathBuf, Option<git::Export>)> {
    let spec = input.to_string_lossy();
    if !force_git && !git::is_revision(&spec) {
        return Ok((input.to_path_buf(), None));
    }
    if !git::is_revision(&spec) {
        bail!("--git: {spec:?} is neither a path nor a git revision");
    }
    let export = git::export(&spec)?;
    Ok((export.path().to_path_buf(), Some(export)))
}

/// `--count-and-diff`: count each input, then compare them.
fn run_count_and_diff(cli: &Cli, db: &LangDb) -> Result<()> {
    if cli.inputs.len() != 2 {
        bail!(
            "--count-and-diff takes exactly two inputs, got {}",
            cli.inputs.len()
        );
    }
    for input in &cli.inputs {
        let single = Cli {
            inputs: vec![input.clone()],
            count_and_diff: false,
            ..cli.clone()
        };
        let started = Instant::now();
        let mut report = count(&single, db)?;
        report.elapsed_secs = started.elapsed().as_secs_f64();
        print!(
            "{}",
            output::render(&report, format_of(cli), &output_options(cli)?)
        );
        println!();
    }
    run_diff(cli, db)
}

/// `--diff`: compare exactly two inputs.
fn run_diff(cli: &Cli, db: &LangDb) -> Result<()> {
    if cli.inputs.len() != 2 {
        bail!(
            "--diff compares exactly two files or directories, got {}",
            cli.inputs.len()
        );
    }

    // Inputs may be git revisions; each is exported to a temporary
    // directory whose lifetime is tied to the guard held here.
    let force_git = cli.git || cli.git_diff_rel || cli.git_diff_all;
    let (left, _left_guard) = resolve_input(&cli.inputs[0], force_git)?;
    let (right, _right_guard) = resolve_input(&cli.inputs[1], force_git)?;
    let (left, right) = (&left, &right);

    let walk_opts = WalkOptions {
        exclude_dirs: split_list(&cli.exclude_dir),
        match_f: cli.match_f.clone(),
        not_match_f: cli.not_match_f.clone(),
        match_d: cli.match_d.clone(),
        not_match_d: cli.not_match_d.clone(),
        fullpath: cli.fullpath,
        exclude_ext: split_list(&cli.exclude_ext),
        no_recurse: cli.no_recurse,
        follow_links: cli.follow_links,
        read_binary_files: cli.read_binary_files,
        max_file_size_mb: cli.max_file_size,
        skip_hidden: cli.skip_hidden,
    };
    let classify_opts = classify_options(cli);
    // Each side is de-duplicated on its own, exactly as a plain count would
    // be; without it a tree holding two copies of a file reports both.
    let dedupe_side = |root: &PathBuf| -> Result<Vec<PathBuf>> {
        let found = walk::collect(std::slice::from_ref(root), &walk_opts)?.files;
        Ok(if cli.skip_uniqueness {
            found
        } else {
            dedupe::remove_duplicates(found, db, &classify_opts).unique
        })
    };
    let mut left_files = dedupe_side(left)?;
    let mut right_files = dedupe_side(right)?;

    // Diffing two revisions compares only the files that changed between
    // them, since the rest cannot contribute; --git-diff-all asks for the
    // whole tree instead. `--git --diff` already means the narrowed form,
    // which is what --git-diff-rel is a name for.
    if force_git && !cli.git_diff_all {
        let changed = git::changed_files(
            &cli.inputs[0].to_string_lossy(),
            &cli.inputs[1].to_string_lossy(),
        )?;
        left_files = git::restrict_to(left_files, left, &changed);
        right_files = git::restrict_to(right_files, right, &changed);
    }

    let opts = DiffOptions {
        count: count_options(cli)?,
        ignore_whitespace: cli.ignore_whitespace,
        ignore_case: cli.ignore_case,
        classify: classify_opts,
    };
    let report = diffmode::compare(left, &left_files, right, &right_files, db, &opts)?;

    if let Some(path) = &cli.diff_alignment {
        let mut text = report.alignment.join("\n");
        text.push('\n');
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    }

    let rendered = output::render_diff(&report, format_of(cli), &output_options(cli)?);
    match &cli.report_file {
        Some(path) => {
            std::fs::write(path, rendered).with_context(|| format!("writing {}", path.display()))?
        }
        None => print!("{rendered}"),
    }
    Ok(())
}

fn classify_options(cli: &Cli) -> ClassifyOptions {
    ClassifyOptions {
        autoconf: cli.autoconf,
        ignore_case_ext: cli.ignore_case_ext,
        lang_no_ext: cli.lang_no_ext.clone(),
        forced_extensions: vec![],
        no_autogen: cli.no_autogen,
    }
}

fn count_options(cli: &Cli) -> Result<CountOptions> {
    Ok(CountOptions {
        filters: FilterOptions {
            strip_str_comments: cli.strip_str_comments,
            inline: cli.inline,
            docstring_as_code: cli.docstring_as_code,
            no_autogen: cli.no_autogen,
        },
        skip_leading: parse_skip_leading(&cli.skip_leading)?,
        ignore_regex: cli.ignore_regex.clone(),
    })
}

fn output_options(cli: &Cli) -> Result<OutputOptions> {
    // --percent is spelled out as --by-percent t.
    let by_percent = match (&cli.by_percent, cli.percent) {
        (Some(spec), _) => Some(
            report::Denominator::parse(spec)
                .with_context(|| format!("--by-percent expects t, c, cm, cb or cmb, got {spec:?}"))?,
        ),
        (None, true) => Some(report::Denominator::ColumnTotal),
        (None, false) => None,
    };

    let cutoff = match &cli.summary_cutoff {
        Some(spec) => Some(
            report::Cutoff::parse(spec)
                .with_context(|| format!("--summary-cutoff expects X:N or X:N%, got {spec:?}"))?,
        ),
        None => None,
    };

    if let Some(n) = cli.fmt {
        if !(1..=5).contains(&n) {
            bail!("--fmt expects a number from 1 to 5, got {n}");
        }
    }

    Ok(OutputOptions {
        by_file: cli.by_file,
        hide_rate: cli.hide_rate,
        quiet: cli.quiet,
        csv_delimiter: cli.csv_delimiter.clone(),
        thousands_delimiter: cli.thousands_delimiter.clone(),
        by_percent,
        sum_one: cli.sum_one,
        cutoff,
        fmt: cli.fmt,
    })
}

fn format_of(cli: &Cli) -> Format {
    if cli.json {
        Format::Json
    } else if cli.yaml {
        Format::Yaml
    } else if cli.csv {
        Format::Csv
    } else if cli.md {
        Format::Markdown
    } else if cli.xml {
        Format::Xml
    } else {
        Format::Text
    }
}

fn count(cli: &Cli, db: &LangDb) -> Result<Report> {
    // --list-file supplies inputs from a file, one per line.
    let mut inputs = cli.inputs.clone();
    if let Some(path) = &cli.list_file {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        inputs.extend(
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(PathBuf::from),
        );
    }

    // --skip-archive drops matching inputs before anything is unpacked.
    if let Some(pattern) = &cli.skip_archive {
        let re = cloc_core::regex_cache::cached(&format!("(?:{pattern})$"))?;
        inputs.retain(|p| !re.is_match(&p.to_string_lossy()).unwrap_or(false));
    }

    // An archive is replaced by the directory it unpacks to. The guard owns
    // the temporary directories, so it has to outlive the counting below.
    let (inputs, _extraction) = archive::expand_inputs(
        &inputs,
        db,
        &ArchiveOptions {
            extract_with: cli.extract_with.clone(),
            sdir: cli.sdir.clone(),
        },
    )?;

    // --vcs asks the versioning system for the file list rather than
    // walking the disk, so build outputs and ignored files never appear.
    let generator = match &cli.vcs {
        Some(spec) => Some(vcs::resolve(spec, cli.include_submodules)?),
        None => None,
    };

    let mut exclude_dirs = split_list(&cli.exclude_dir);
    if let Some(g) = &generator {
        exclude_dirs.extend(g.exclude_dirs.iter().cloned());
    }

    let walk_opts = WalkOptions {
        exclude_dirs,
        match_f: cli.match_f.clone(),
        not_match_f: cli.not_match_f.clone(),
        match_d: cli.match_d.clone(),
        not_match_d: cli.not_match_d.clone(),
        fullpath: cli.fullpath,
        exclude_ext: split_list(&cli.exclude_ext),
        no_recurse: cli.no_recurse,
        follow_links: cli.follow_links,
        read_binary_files: cli.read_binary_files,
        max_file_size_mb: cli.max_file_size,
        skip_hidden: cli.skip_hidden,
    };
    let found = match &generator {
        Some(g) => walk::collect_listed(&vcs::list_files(g, &inputs)?, &walk_opts)?,
        None => walk::collect(&inputs, &walk_opts)?,
    };

    let classify_opts = classify_options(cli);
    let count_opts = count_options(cli)?;

    // cloc drops files whose content already appeared elsewhere; a tree of
    // Python packages is full of identical __init__.py files.
    let mut ignored = found.ignored;
    let files = if cli.skip_uniqueness {
        found.files
    } else {
        let deduped = dedupe::remove_duplicates(found.files, db, &classify_opts);
        ignored.extend(deduped.duplicates);
        deduped.unique
    };

    let include_lang = split_list(&cli.include_lang);
    let exclude_lang = split_list(&cli.exclude_lang);
    let include_ext = split_list(&cli.include_ext);

    // Files are independent, so classifying and counting fans out across
    // cores; only the merge at the end is serial.
    let outcomes: Vec<Outcome> = files
        .par_iter()
        .map(|path| {
            if !include_ext.is_empty() {
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !include_ext.contains(&ext) {
                    return Outcome::Skipped(path.clone(), "not in --include-ext".to_string());
                }
            }
            let language = match classify::classify(path, db, &classify_opts) {
                Ok(Classification::Language(l)) => l,
                Ok(Classification::Ignored { reason }) => {
                    return Outcome::Skipped(path.clone(), reason)
                }
                Err(e) => return Outcome::Skipped(path.clone(), e.to_string()),
            };
            if !include_lang.is_empty() && !include_lang.contains(&language) {
                return Outcome::Skipped(path.clone(), "not in --include-lang".to_string());
            }
            if exclude_lang.contains(&language) {
                return Outcome::Skipped(path.clone(), "in --exclude-lang".to_string());
            }
            match counter::count_file(path, &language, db, &count_opts) {
                Ok(counts) => Outcome::Counted(FileEntry {
                    path: path.clone(),
                    language,
                    counts,
                }),
                Err(e) => Outcome::Skipped(path.clone(), e.to_string()),
            }
        })
        .collect();

    let mut report = Report {
        ignored,
        ..Default::default()
    };
    for outcome in outcomes {
        match outcome {
            Outcome::Counted(entry) => report.files.push(entry),
            Outcome::Skipped(path, reason) => report.ignored.push((path, reason)),
        }
    }

    if let Some(path) = &cli.ignored {
        let mut text = String::new();
        for (p, why) in &report.ignored {
            text.push_str(&format!("{}: {}\n", p.display(), why));
        }
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    }

    Ok(report)
}

enum Outcome {
    Counted(FileEntry),
    Skipped(PathBuf, String),
}

fn split_list(value: &Option<String>) -> HashSet<String> {
    value
        .as_deref()
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `--skip-leading=N` or `--skip-leading=N,ext1,ext2`.
fn parse_skip_leading(value: &Option<String>) -> Result<Option<(usize, Vec<String>)>> {
    let Some(spec) = value else { return Ok(None) };
    let mut parts = spec.split(',');
    let n = parts
        .next()
        .unwrap_or("")
        .parse()
        .with_context(|| format!("--skip-leading expects a number, got {spec:?}"))?;
    Ok(Some((n, parts.map(str::to_string).collect())))
}

fn print_languages(db: &LangDb) {
    let mut by_lang: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    for ext in db.extensions() {
        if let Some(lang) = db.language_for_extension(ext) {
            by_lang.entry(lang).or_default().push(ext);
        }
    }
    for (lang, exts) in by_lang {
        println!("{lang} ({})", exts.join(", "));
    }
}

fn print_extensions(db: &LangDb) {
    for ext in db.extensions() {
        if let Some(lang) = db.language_for_extension(ext) {
            println!("{ext}  {lang}");
        }
    }
}

fn explain(db: &LangDb, language: &str) -> Result<()> {
    let Some(filters) = db.filters(language) else {
        bail!("unknown language {language:?}; try --show-lang");
    };
    println!("{language}");
    for f in filters {
        println!("    {f:?}");
    }
    Ok(())
}
