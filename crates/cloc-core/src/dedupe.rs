//! Dropping files whose content already appeared under another name.
//!
//! cloc does this by default — `--skip-uniqueness` turns it off — and it is
//! not a rounding error: a source tree of Python packages is full of
//! identical `__init__.py` files, and counting each one inflates the totals.
//!
//! The original compares file sizes first and only digests the files that
//! collide, which is worth keeping: most files are a unique length and never
//! get read at all.

use crate::classify::{self, Classification, ClassifyOptions};
use cloc_lang::LangDb;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Files kept, and those dropped with the file they duplicate.
#[derive(Debug, Default)]
pub struct DedupeResult {
    pub unique: Vec<PathBuf>,
    pub duplicates: Vec<(PathBuf, String)>,
}

/// Remove files whose content duplicates an earlier one.
pub fn remove_duplicates(
    files: Vec<PathBuf>,
    db: &LangDb,
    opts: &ClassifyOptions,
) -> DedupeResult {
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let mut result = DedupeResult::default();

    for path in files {
        match path.metadata() {
            Ok(m) => by_size.entry(m.len()).or_default().push(path),
            // Unreadable metadata is not this pass's problem; let the
            // counter report it.
            Err(_) => result.unique.push(path),
        }
    }

    let mut sizes: Vec<u64> = by_size.keys().copied().collect();
    sizes.sort_unstable();

    for size in sizes {
        let mut group = by_size.remove(&size).unwrap_or_default();
        if group.len() == 1 {
            result.unique.extend(group);
            continue;
        }
        // Sorting by full path rather than basename keeps the choice
        // reproducible when two directories hold the same file name.
        group.sort();
        resolve_group(group, db, opts, &mut result);
    }

    result.unique.sort();
    result
}

/// Partition one equally-sized group into distinct contents.
fn resolve_group(
    group: Vec<PathBuf>,
    db: &LangDb,
    opts: &ClassifyOptions,
    result: &mut DedupeResult,
) {
    // Bucket by content hash, then confirm by comparing bytes, so a hash
    // collision cannot silently discard a file.
    let mut buckets: Vec<(Vec<u8>, Vec<PathBuf>)> = Vec::new();

    for path in group {
        let Ok(content) = std::fs::read(&path) else {
            result.unique.push(path);
            continue;
        };
        match buckets.iter_mut().find(|(c, _)| *c == content) {
            Some((_, members)) => members.push(path),
            None => buckets.push((content, vec![path])),
        }
    }

    for (_, members) in buckets {
        if members.len() == 1 {
            result.unique.extend(members);
            continue;
        }
        let keep = best_of(&members, db, opts);
        for (i, path) in members.iter().enumerate() {
            if i == keep {
                result.unique.push(path.clone());
            } else {
                result.duplicates.push((
                    path.clone(),
                    format!("duplicate of {}", members[keep].display()),
                ));
            }
        }
    }
}

/// Which member of an identical set to keep.
///
/// The original scans from the second element onward and keeps the *last*
/// one that classifies to a known language, falling back to the first when
/// none of the others do. The point is to prefer a name cloc can recognise —
/// `main.py` over `main.bak` — but the "last" part is what the code does, so
/// it is what we do.
fn best_of(members: &[PathBuf], db: &LangDb, opts: &ClassifyOptions) -> usize {
    let mut best = 0;
    for (i, path) in members.iter().enumerate().skip(1) {
        if classifies(path, db, opts) {
            best = i;
        }
    }
    best
}

fn classifies(path: &Path, db: &LangDb, opts: &ClassifyOptions) -> bool {
    matches!(
        classify::classify(path, db, opts),
        Ok(Classification::Language(_))
    )
}

/// Not used for identity — kept for callers that want a cheap content key.
pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("cloc-dedupe-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (rel, content) in files {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        root
    }

    fn run(root: &Path, names: &[&str]) -> DedupeResult {
        let files: Vec<PathBuf> = names.iter().map(|n| root.join(n)).collect();
        remove_duplicates(files, LangDb::default_db(), &ClassifyOptions::default())
    }

    #[test]
    fn identical_files_collapse_to_one() {
        let root = tree(
            "same",
            &[("a/mod.py", "x = 1\n"), ("b/mod.py", "x = 1\n")],
        );
        let r = run(&root, &["a/mod.py", "b/mod.py"]);
        assert_eq!(r.unique.len(), 1);
        assert_eq!(r.duplicates.len(), 1);
        assert!(r.duplicates[0].1.starts_with("duplicate of "));
    }

    /// Same length, different content: both are kept. This is the case the
    /// size-first shortcut must not get wrong.
    #[test]
    fn equally_sized_but_different_files_both_survive() {
        let root = tree(
            "sized",
            &[("a.py", "x = 1\n"), ("b.py", "y = 2\n")],
        );
        let r = run(&root, &["a.py", "b.py"]);
        assert_eq!(r.unique.len(), 2);
        assert!(r.duplicates.is_empty());
    }

    #[test]
    fn distinct_sizes_are_never_read_twice() {
        let root = tree("diff", &[("a.py", "x = 1\n"), ("b.py", "y = 22222\n")]);
        let r = run(&root, &["a.py", "b.py"]);
        assert_eq!(r.unique.len(), 2);
    }

    /// Among identical files, one with a recognisable extension is preferred
    /// over one cloc cannot classify.
    #[test]
    fn a_classifiable_name_wins() {
        let root = tree(
            "best",
            &[("a.qqq", "x = 1\n"), ("b.py", "x = 1\n")],
        );
        let r = run(&root, &["a.qqq", "b.py"]);
        assert_eq!(r.unique.len(), 1);
        assert!(r.unique[0].to_string_lossy().ends_with("b.py"));
    }

    #[test]
    fn three_copies_leave_one() {
        let root = tree(
            "three",
            &[
                ("a/i.py", "pass\n"),
                ("b/i.py", "pass\n"),
                ("c/i.py", "pass\n"),
            ],
        );
        let r = run(&root, &["a/i.py", "b/i.py", "c/i.py"]);
        assert_eq!(r.unique.len(), 1);
        assert_eq!(r.duplicates.len(), 2);
    }
}
