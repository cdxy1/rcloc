//! Building the list of files to count.

use crate::io;
use crate::regex_cache;
use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Filters applied while walking.
#[derive(Debug, Clone, Default)]
pub struct WalkOptions {
    /// `--exclude-dir`: directory names, matched against any path component.
    pub exclude_dirs: HashSet<String>,
    /// `--match-f` / `--not-match-f`: regexes on the file name.
    pub match_f: Option<String>,
    pub not_match_f: Vec<String>,
    /// `--match-d` / `--not-match-d`: regexes on the directory path.
    pub match_d: Option<String>,
    pub not_match_d: Vec<String>,
    /// `--fullpath`: match the file regexes against the whole path.
    pub fullpath: bool,
    /// `--exclude-ext`: extensions to skip.
    pub exclude_ext: HashSet<String>,
    /// `--no-recurse`: do not descend into subdirectories.
    pub no_recurse: bool,
    /// `--follow-links`: follow symbolic links.
    pub follow_links: bool,
    /// `--read-binary-files`: count files containing NUL bytes.
    pub read_binary_files: bool,
    /// `--max-file-size`: skip files larger than this many megabytes.
    pub max_file_size_mb: Option<f64>,
    /// `--skip-win-hidden`: skip files whose name starts with a dot.
    pub skip_hidden: bool,
}

/// A file that survived the walk, plus anything skipped and why.
#[derive(Debug, Default)]
pub struct WalkResult {
    pub files: Vec<PathBuf>,
    pub ignored: Vec<(PathBuf, String)>,
}

/// Collect the files under `inputs`.
///
/// A path given explicitly on the command line is taken at face value: cloc
/// counts a named file even if a `--not-match-f` would have excluded it
/// during recursion.
pub fn collect(inputs: &[PathBuf], opts: &WalkOptions) -> Result<WalkResult> {
    let mut result = WalkResult::default();
    let mut seen = HashSet::new();

    for input in inputs {
        if input.is_file() {
            consider(input, opts, &mut result, &mut seen, true)?;
            continue;
        }
        if !input.is_dir() {
            result
                .ignored
                .push((input.clone(), "neither file nor directory".to_string()));
            continue;
        }

        let mut walker = WalkDir::new(input).follow_links(opts.follow_links);
        if opts.no_recurse {
            walker = walker.max_depth(1);
        }

        for entry in walker.into_iter().filter_entry(|e| {
            // Pruning at the directory level saves descending into a tree
            // that is excluded wholesale.
            !e.file_type().is_dir() || dir_allowed(e.path(), opts).unwrap_or(true)
        }) {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    if let Some(p) = e.path() {
                        result.ignored.push((p.to_path_buf(), e.to_string()));
                    }
                    continue;
                }
            };
            if entry.file_type().is_file() {
                consider(entry.path(), opts, &mut result, &mut seen, false)?;
            }
        }
    }

    result.files.sort();
    Ok(result)
}

/// Whether to descend into a directory.
fn dir_allowed(path: &Path, opts: &WalkOptions) -> Result<bool> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if opts.exclude_dirs.contains(&name) {
        return Ok(false);
    }
    let as_str = path.to_string_lossy();
    if let Some(p) = &opts.match_d {
        if !regex_cache::cached(p)?.is_match(&as_str)? {
            // Not a match yet, but a subdirectory still might be, so keep
            // descending; the file-level check is what excludes.
            return Ok(true);
        }
    }
    for p in &opts.not_match_d {
        if regex_cache::cached(p)?.is_match(&as_str)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Decide a single candidate file.
fn consider(
    path: &Path,
    opts: &WalkOptions,
    result: &mut WalkResult,
    seen: &mut HashSet<PathBuf>,
    explicit: bool,
) -> Result<()> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canonical) {
        return Ok(());
    }

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let subject = if opts.fullpath {
        path.to_string_lossy().into_owned()
    } else {
        name.clone()
    };

    let skip = |reason: &str, result: &mut WalkResult| {
        result.ignored.push((path.to_path_buf(), reason.to_string()));
    };

    if !explicit {
        if opts.skip_hidden && name.starts_with('.') {
            skip("hidden file", result);
            return Ok(());
        }
        if let Some(p) = &opts.match_f {
            if !regex_cache::cached(p)?.is_match(&subject)? {
                skip("did not match --match-f", result);
                return Ok(());
            }
        }
        for p in &opts.not_match_f {
            if regex_cache::cached(p)?.is_match(&subject)? {
                skip("matched --not-match-f", result);
                return Ok(());
            }
        }
        if let Some(p) = &opts.match_d {
            let dir = path.parent().unwrap_or(Path::new(".")).to_string_lossy();
            if !regex_cache::cached(p)?.is_match(&dir)? {
                skip("directory did not match --match-d", result);
                return Ok(());
            }
        }
    }

    if let Some(ext) = path.extension().map(|e| e.to_string_lossy().into_owned()) {
        if opts.exclude_ext.contains(&ext) {
            skip("excluded extension", result);
            return Ok(());
        }
    }

    let metadata = match path.metadata() {
        Ok(m) => m,
        Err(e) => {
            skip(&format!("unable to stat: {e}"), result);
            return Ok(());
        }
    };
    if metadata.len() == 0 {
        // Empty files are counted, but with nothing in them.
        result.files.push(path.to_path_buf());
        return Ok(());
    }
    if let Some(limit) = opts.max_file_size_mb {
        if metadata.len() as f64 / (1024.0 * 1024.0) > limit {
            skip("exceeds --max-file-size", result);
            return Ok(());
        }
    }
    if !opts.read_binary_files && io::is_binary(path).unwrap_or(false) {
        skip("binary file", result);
        return Ok(());
    }

    result.files.push(path.to_path_buf());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Build a small tree in a uniquely named temp directory.
    fn tree(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("cloc-walk-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (rel, content) in files {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        root
    }

    fn names(result: &WalkResult, root: &Path) -> Vec<String> {
        let mut v: Vec<String> = result
            .files
            .iter()
            .map(|p| {
                p.strip_prefix(root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn walks_recursively() {
        let root = tree("rec", &[("a.rs", "fn a(){}"), ("sub/b.rs", "fn b(){}")]);
        let r = collect(&[root.clone()], &WalkOptions::default()).unwrap();
        assert_eq!(names(&r, &root), vec!["a.rs", "sub/b.rs"]);
    }

    #[test]
    fn no_recurse_stays_shallow() {
        let root = tree("norec", &[("a.rs", "fn a(){}"), ("sub/b.rs", "fn b(){}")]);
        let opts = WalkOptions {
            no_recurse: true,
            ..Default::default()
        };
        let r = collect(&[root.clone()], &opts).unwrap();
        assert_eq!(names(&r, &root), vec!["a.rs"]);
    }

    #[test]
    fn exclude_dir_prunes_the_tree() {
        let root = tree(
            "excl",
            &[("a.rs", "fn a(){}"), ("vendor/b.rs", "fn b(){}")],
        );
        let opts = WalkOptions {
            exclude_dirs: ["vendor".to_string()].into_iter().collect(),
            ..Default::default()
        };
        let r = collect(&[root.clone()], &opts).unwrap();
        assert_eq!(names(&r, &root), vec!["a.rs"]);
    }

    #[test]
    fn not_match_f_skips_files() {
        let root = tree("nmf", &[("a.rs", "fn a(){}"), ("b_test.rs", "fn b(){}")]);
        let opts = WalkOptions {
            not_match_f: vec!["_test\\.rs$".to_string()],
            ..Default::default()
        };
        let r = collect(&[root.clone()], &opts).unwrap();
        assert_eq!(names(&r, &root), vec!["a.rs"]);
    }

    #[test]
    fn binary_files_are_skipped_by_default() {
        let root = tree("bin", &[("a.rs", "fn a(){}")]);
        fs::write(root.join("blob.dat"), b"\x00\x01\x02binary").unwrap();
        let r = collect(&[root.clone()], &WalkOptions::default()).unwrap();
        assert_eq!(names(&r, &root), vec!["a.rs"]);
        assert!(r.ignored.iter().any(|(_, why)| why == "binary file"));
    }

    /// The same file reached twice must be counted once.
    #[test]
    fn duplicate_inputs_are_collapsed() {
        let root = tree("dup", &[("a.rs", "fn a(){}")]);
        let file = root.join("a.rs");
        let r = collect(&[file.clone(), file], &WalkOptions::default()).unwrap();
        assert_eq!(r.files.len(), 1);
    }

    /// An explicitly named file is counted even when a filter would exclude
    /// it during recursion.
    #[test]
    fn explicit_files_bypass_name_filters() {
        let root = tree("expl", &[("b_test.rs", "fn b(){}")]);
        let opts = WalkOptions {
            not_match_f: vec!["_test\\.rs$".to_string()],
            ..Default::default()
        };
        let r = collect(&[root.join("b_test.rs")], &opts).unwrap();
        assert_eq!(r.files.len(), 1);
    }
}
