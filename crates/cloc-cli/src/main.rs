//! `cloc-rs` — count lines of code.

mod output;
mod report;

use anyhow::{bail, Context, Result};
use clap::Parser;
use cloc_core::archive::{self, ArchiveOptions};
use cloc_core::classify::{self, Classification, ClassifyOptions};
use cloc_core::counter::{self, CountOptions};
use cloc_core::dedupe;
use cloc_core::filters::FilterOptions;
use cloc_core::walk::{self, WalkOptions};
use cloc_lang::LangDb;
use output::{Format, OutputOptions};
use rayon::prelude::*;
use report::{FileEntry, Report};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser, Debug)]
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
    let db = LangDb::default_db();

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

    let started = Instant::now();
    let mut report = count(&cli, db)?;
    report.elapsed_secs = started.elapsed().as_secs_f64();

    let rendered = output::render(
        &report,
        format_of(&cli),
        &OutputOptions {
            by_file: cli.by_file,
            hide_rate: cli.hide_rate,
            quiet: cli.quiet,
            csv_delimiter: cli.csv_delimiter.clone(),
            thousands_delimiter: cli.thousands_delimiter.clone(),
        },
    );

    match &cli.report_file {
        Some(path) => {
            std::fs::write(path, rendered).with_context(|| format!("writing {}", path.display()))?
        }
        None => print!("{rendered}"),
    }
    Ok(())
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

fn count(cli: &Cli, db: &'static LangDb) -> Result<Report> {
    // --skip-archive drops matching inputs before anything is unpacked.
    let mut inputs = cli.inputs.clone();
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
    let found = walk::collect(&inputs, &walk_opts)?;

    let classify_opts = ClassifyOptions {
        autoconf: cli.autoconf,
        ignore_case_ext: cli.ignore_case_ext,
        lang_no_ext: cli.lang_no_ext.clone(),
        forced_extensions: vec![],
        no_autogen: cli.no_autogen,
    };
    let count_opts = CountOptions {
        filters: FilterOptions {
            strip_str_comments: cli.strip_str_comments,
            inline: cli.inline,
            docstring_as_code: cli.docstring_as_code,
            no_autogen: cli.no_autogen,
        },
        skip_leading: parse_skip_leading(&cli.skip_leading)?,
        ignore_regex: cli.ignore_regex.clone(),
    };

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
