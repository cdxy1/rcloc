//! Deciding which file on the left corresponds to which on the right.
//!
//! Comparing two revisions of a tree is easy — the paths line up. Comparing
//! two *releases* is not: `foo-1.2/src/main.c` and `foo-1.3/src/main.c` are
//! the same file under different names, and matching on the full path would
//! call one added and the other removed.
//!
//! So the leading directories are stripped before matching. How many
//! components to strip is worked out from files whose basename occurs exactly
//! once on each side: those are certainly the same file, and the number of
//! trailing path components they share says how much of the front is
//! packaging rather than structure.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// The pairing of two file lists.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Alignment {
    /// Files present on both sides, left first.
    pub pairs: Vec<(PathBuf, PathBuf)>,
    /// Present only on the right.
    pub added: Vec<PathBuf>,
    /// Present only on the left.
    pub removed: Vec<PathBuf>,
}

/// Pair up two file lists.
pub fn align(left: &[PathBuf], right: &[PathBuf]) -> Alignment {
    let mut l: Vec<PathBuf> = left.to_vec();
    let mut r: Vec<PathBuf> = right.to_vec();
    l.sort();
    r.sort();

    if l.is_empty() && r.is_empty() {
        return Alignment::default();
    }
    if r.is_empty() {
        return Alignment { removed: l, ..Default::default() };
    }
    if l.is_empty() {
        return Alignment { added: r, ..Default::default() };
    }
    // One file against one file is always that pair, whatever they are
    // called; otherwise the naming logic would report an addition and a
    // removal for what the user plainly meant as a comparison.
    if l.len() == 1 && r.len() == 1 {
        return Alignment {
            pairs: vec![(l[0].clone(), r[0].clone())],
            ..Default::default()
        };
    }

    let (keys_l, keys_r) = match leading_dirs(&l, &r) {
        Some((drop_l, drop_r)) => (strip_prefix(&l, &drop_l), strip_prefix(&r, &drop_r)),
        // No uniquely named file in common to calibrate against; fall back
        // to removing whatever leading directory each side has in common.
        None => (remove_common_leading_dir(&l), remove_common_leading_dir(&r)),
    };

    let map_l: BTreeMap<String, PathBuf> = keys_l.into_iter().zip(l.iter().cloned()).collect();
    let map_r: BTreeMap<String, PathBuf> = keys_r.into_iter().zip(r.iter().cloned()).collect();

    let mut out = Alignment::default();
    for (key, path) in &map_l {
        match map_r.get(key) {
            Some(other) => out.pairs.push((path.clone(), other.clone())),
            None => out.removed.push(path.clone()),
        }
    }
    for (key, path) in &map_r {
        if !map_l.contains_key(key) {
            out.added.push(path.clone());
        }
    }
    out
}

/// How much of the front of each side's paths is packaging.
///
/// Returns `None` when no basename occurs exactly once on both sides, since
/// then there is nothing to calibrate against.
fn leading_dirs(left: &[PathBuf], right: &[PathBuf]) -> Option<(String, String)> {
    let unique_l = unique_basenames(left);
    let unique_r = unique_basenames(right);

    let mut drop_l: Option<String> = None;
    let mut drop_r: Option<String> = None;
    let mut found = false;

    for (name, path_l) in &unique_l {
        let Some(path_r) = unique_r.get(name) else {
            continue;
        };
        found = true;

        let dl: Vec<&str> = components(path_l);
        let dr: Vec<&str> = components(path_r);

        // Count the trailing components the two share; what precedes them is
        // the part to drop.
        let mut shared = 0;
        while shared < dl.len().min(dr.len())
            && dl[dl.len() - 1 - shared] == dr[dr.len() - 1 - shared]
        {
            shared += 1;
        }
        // Built by walking up rather than by joining components: for an
        // absolute path the first component is the root separator, and
        // joining it back with "/" yields a leading "//" that then fails to
        // match the path it came from.
        let common_l = drop_last(path_l, shared);
        let common_r = drop_last(path_r, shared);

        // The shortest prefix that works for any pair is the one to use, or
        // files under a shallower directory would lose part of their path.
        if drop_l.as_ref().is_none_or(|d| common_l.len() < d.len()) {
            drop_l = Some(common_l);
        }
        if drop_r.as_ref().is_none_or(|d| common_r.len() < d.len()) {
            drop_r = Some(common_r);
        }
    }

    if !found {
        return None;
    }
    let finish = |d: Option<String>| match d {
        Some(s) if s.is_empty() => String::new(),
        Some(s) => format!("{s}/"),
        None => String::new(),
    };
    Some((finish(drop_l), finish(drop_r)))
}

/// Basenames occurring exactly once, mapped to their path.
fn unique_basenames(files: &[PathBuf]) -> BTreeMap<String, PathBuf> {
    let mut counts: HashMap<String, (usize, PathBuf)> = HashMap::new();
    for path in files {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let entry = counts.entry(name).or_insert((0, path.clone()));
        entry.0 += 1;
        entry.1 = path.clone();
    }
    counts
        .into_iter()
        .filter(|(_, (n, _))| *n == 1)
        .map(|(name, (_, path))| (name, path))
        .collect()
}

fn components(path: &std::path::Path) -> Vec<&str> {
    path.iter().filter_map(|c| c.to_str()).collect()
}

/// The path with its last `n` components removed, as a string.
fn drop_last(path: &std::path::Path, n: usize) -> String {
    let mut current = path.to_path_buf();
    for _ in 0..n {
        current = current.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    }
    current.to_string_lossy().into_owned()
}

fn strip_prefix(files: &[PathBuf], prefix: &str) -> Vec<String> {
    files
        .iter()
        .map(|p| {
            let s = p.to_string_lossy().into_owned();
            s.strip_prefix(prefix).unwrap_or(&s).to_string()
        })
        .collect()
}

/// Drop the directory prefix every path shares.
///
/// With a single file there is no baseline to compare against, so just the
/// first component goes.
fn remove_common_leading_dir(files: &[PathBuf]) -> Vec<String> {
    if files.len() == 1 {
        let parts = components(&files[0]);
        return vec![if parts.len() > 1 {
            parts[1..].join("/")
        } else {
            parts.join("/")
        }];
    }

    let first = components(&files[0]);
    let mut shared = first.len().saturating_sub(1);
    for path in &files[1..] {
        let parts = components(path);
        let mut k = 0;
        while k < shared && k + 1 < parts.len() && parts[k] == first[k] {
            k += 1;
        }
        shared = k;
    }

    files
        .iter()
        .map(|p| components(p)[shared..].join("/"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    fn keys(pairs: &[(PathBuf, PathBuf)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| {
                (
                    a.to_string_lossy().into_owned(),
                    b.to_string_lossy().into_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn an_empty_side_makes_everything_added_or_removed() {
        let a = align(&paths(&["a.c", "b.c"]), &[]);
        assert_eq!(a.removed.len(), 2);
        assert!(a.pairs.is_empty());

        let a = align(&[], &paths(&["a.c"]));
        assert_eq!(a.added.len(), 1);
    }

    /// One file against one file is that pair, whatever the names.
    #[test]
    fn a_single_file_each_side_is_forced_into_a_pair() {
        let a = align(&paths(&["old/thing.c"]), &paths(&["new/other.c"]));
        assert_eq!(keys(&a.pairs), vec![("old/thing.c".into(), "new/other.c".into())]);
        assert!(a.added.is_empty() && a.removed.is_empty());
    }

    /// The point of the exercise: two release directories line up despite
    /// their names differing.
    #[test]
    fn release_directories_align_by_their_contents() {
        let a = align(
            &paths(&["foo-1.2/src/main.c", "foo-1.2/src/util.c", "foo-1.2/README"]),
            &paths(&["foo-1.3/src/main.c", "foo-1.3/src/util.c", "foo-1.3/README"]),
        );
        assert_eq!(a.pairs.len(), 3);
        assert!(a.added.is_empty());
        assert!(a.removed.is_empty());
        for (l, r) in &a.pairs {
            assert_eq!(l.file_name(), r.file_name());
        }
    }

    #[test]
    fn files_on_one_side_only_are_reported() {
        let a = align(
            &paths(&["v1/a.c", "v1/gone.c", "v1/keep.c"]),
            &paths(&["v2/a.c", "v2/keep.c", "v2/fresh.c"]),
        );
        assert_eq!(a.pairs.len(), 2);
        assert_eq!(a.added.len(), 1);
        assert!(a.added[0].ends_with("fresh.c"));
        assert_eq!(a.removed.len(), 1);
        assert!(a.removed[0].ends_with("gone.c"));
    }

    /// Identical layouts under the same root still pair up.
    #[test]
    fn identical_paths_pair_up() {
        let a = align(
            &paths(&["src/a.c", "src/b.c"]),
            &paths(&["src/a.c", "src/b.c"]),
        );
        assert_eq!(a.pairs.len(), 2);
    }

    /// Where no basename is unique on both sides there is nothing to
    /// calibrate against, and the common leading directory goes instead.
    #[test]
    fn falls_back_to_the_common_leading_directory() {
        let a = align(
            &paths(&["L/x/a.c", "L/y/a.c", "L/z/b.c"]),
            &paths(&["R/x/a.c", "R/y/a.c", "R/z/b.c"]),
        );
        assert_eq!(a.pairs.len(), 3);
        assert!(a.added.is_empty() && a.removed.is_empty());
    }

    /// Nested directories keep their structure below the stripped prefix, so
    /// two files of the same name in different subdirectories stay distinct.
    #[test]
    fn structure_below_the_prefix_is_preserved() {
        let a = align(
            &paths(&["v1/a/dup.c", "v1/b/dup.c", "v1/only.c"]),
            &paths(&["v2/a/dup.c", "v2/b/dup.c", "v2/only.c"]),
        );
        assert_eq!(a.pairs.len(), 3);
        for (l, r) in &a.pairs {
            let (ls, rs) = (l.to_string_lossy(), r.to_string_lossy());
            assert_eq!(&ls[2..], &rs[2..], "paired across subdirectories");
        }
    }
}
