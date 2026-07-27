//! Line diffing, for `--diff`.
//!
//! The original uses `Algorithm::Diff::sdiff`, which pairs the two sides up
//! line by line and labels each pair unchanged, changed, removed or added.
//! "Changed" is the interesting one: within a run where both sides have
//! lines, they are matched off positionally, and only the surplus on either
//! side counts as added or removed. A rewritten line is therefore one
//! modification rather than a removal plus an addition, which is what makes
//! cloc's diff totals read the way they do.

use std::collections::HashMap;

/// How a pair of lines relates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    /// Present and equal on both sides.
    Same,
    /// Present on both sides but different.
    Modified,
    /// Present only on the left.
    Removed,
    /// Present only on the right.
    Added,
}

impl Edit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Modified => "modified",
            Self::Removed => "removed",
            Self::Added => "added",
        }
    }
}

/// One row of a side-by-side diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub edit: Edit,
    pub left: Option<String>,
    pub right: Option<String>,
}

/// Side-by-side diff of two line sequences.
pub fn sdiff(left: &[String], right: &[String]) -> Vec<Row> {
    let matches = lcs(left, right);

    let mut rows = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);

    let flush = |rows: &mut Vec<Row>,
                     li: &mut usize,
                     lj: &mut usize,
                     i_end: usize,
                     j_end: usize| {
        // A run where both sides have lines is a block of modifications,
        // with whatever is left over on one side added or removed.
        let removed = i_end - *li;
        let added = j_end - *lj;
        let paired = removed.min(added);
        for k in 0..paired {
            rows.push(Row {
                edit: Edit::Modified,
                left: Some(left[*li + k].clone()),
                right: Some(right[*lj + k].clone()),
            });
        }
        for k in paired..removed {
            rows.push(Row {
                edit: Edit::Removed,
                left: Some(left[*li + k].clone()),
                right: None,
            });
        }
        for k in paired..added {
            rows.push(Row {
                edit: Edit::Added,
                left: None,
                right: Some(right[*lj + k].clone()),
            });
        }
        *li = i_end;
        *lj = j_end;
    };

    for (mi, mj) in matches {
        flush(&mut rows, &mut i, &mut j, mi, mj);
        rows.push(Row {
            edit: Edit::Same,
            left: Some(left[mi].clone()),
            right: Some(right[mj].clone()),
        });
        i = mi + 1;
        j = mj + 1;
    }
    flush(&mut rows, &mut i, &mut j, left.len(), right.len());

    rows
}

/// Index pairs of a longest common subsequence.
///
/// Equal prefixes and suffixes are peeled off first, which is what keeps this
/// fast on the usual case of a small edit to a large file; the quadratic core
/// then only sees the part that actually changed.
fn lcs(left: &[String], right: &[String]) -> Vec<(usize, usize)> {
    let mut prefix = 0;
    while prefix < left.len() && prefix < right.len() && left[prefix] == right[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < left.len() - prefix
        && suffix < right.len() - prefix
        && left[left.len() - 1 - suffix] == right[right.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let mut out: Vec<(usize, usize)> = (0..prefix).map(|k| (k, k)).collect();

    let a = &left[prefix..left.len() - suffix];
    let b = &right[prefix..right.len() - suffix];
    for (i, j) in lcs_core(a, b) {
        out.push((prefix + i, prefix + j));
    }

    for k in 0..suffix {
        out.push((left.len() - suffix + k, right.len() - suffix + k));
    }
    out
}

/// Hunt–Szymanski longest common subsequence.
///
/// Each left line is looked up in an index of the right side, so the work is
/// proportional to the number of matching pairs rather than to the product of
/// the lengths. Source files repeat few lines, so this stays close to linear
/// where a full dynamic-programming table would not fit in memory.
fn lcs_core(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }

    let mut positions: HashMap<&str, Vec<usize>> = HashMap::new();
    for (j, line) in b.iter().enumerate() {
        positions.entry(line.as_str()).or_default().push(j);
    }

    // thresholds[k] is the smallest right-hand index ending a common
    // subsequence of length k+1 found so far.
    let mut thresholds: Vec<usize> = Vec::new();
    // Back-links, so the chosen subsequence can be recovered at the end.
    let mut links: Vec<Option<usize>> = Vec::new();
    let mut nodes: Vec<(usize, usize, Option<usize>)> = Vec::new();

    for (i, line) in a.iter().enumerate() {
        let Some(js) = positions.get(line.as_str()) else {
            continue;
        };
        // Descending, so several matches for one left line cannot chain to
        // each other within a single step.
        for &j in js.iter().rev() {
            let k = thresholds.partition_point(|&t| t < j);
            if k < thresholds.len() && thresholds[k] == j {
                continue;
            }
            let previous = if k == 0 { None } else { Some(links[k - 1].unwrap()) };
            nodes.push((i, j, previous));
            let node = nodes.len() - 1;
            if k == thresholds.len() {
                thresholds.push(j);
                links.push(Some(node));
            } else {
                thresholds[k] = j;
                links[k] = Some(node);
            }
        }
    }

    let mut result = Vec::new();
    let mut cursor = links.last().copied().flatten();
    while let Some(node) = cursor {
        let (i, j, previous) = nodes[node];
        result.push((i, j));
        cursor = previous;
    }
    result.reverse();
    result
}

/// Which of a file's lines are code and which are comments.
///
/// Determined by diffing the file against itself with comments stripped: a
/// line surviving both is code, a line present only in the original is a
/// comment. This is how the original classifies lines before comparing two
/// revisions, and it means comment counting and diffing agree by
/// construction.
pub fn split_code_and_comments(
    without_blanks: &[String],
    without_comments: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut code = Vec::new();
    let mut comments = Vec::new();
    for row in sdiff(without_blanks, without_comments) {
        match row.edit {
            // Survived comment stripping, so it is code.
            Edit::Same | Edit::Modified => {
                if let Some(line) = row.left {
                    code.push(line);
                }
            }
            // Present before stripping and gone after: a comment.
            Edit::Removed => {
                if let Some(line) = row.left {
                    comments.push(line);
                }
            }
            // Stripping cannot invent lines.
            Edit::Added => {}
        }
    }
    (code, comments)
}

/// Per-line classification, in the order the lines appear.
///
/// `true` means the line survived comment stripping and is therefore code.
/// Same mechanism as [`split_code_and_comments`], but keeping the positions
/// so a caller can annotate the original file.
pub fn code_line_flags(without_blanks: &[String], without_comments: &[String]) -> Vec<bool> {
    let mut flags = Vec::with_capacity(without_blanks.len());
    for row in sdiff(without_blanks, without_comments) {
        match row.edit {
            Edit::Same | Edit::Modified => flags.push(true),
            Edit::Removed => flags.push(false),
            Edit::Added => {}
        }
    }
    flags
}

/// Tally of how one category of lines changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Delta {
    pub same: usize,
    pub modified: usize,
    pub added: usize,
    pub removed: usize,
}

impl Delta {
    /// Count the edits between two line sequences.
    pub fn between(left: &[String], right: &[String]) -> Self {
        let mut d = Self::default();
        for row in sdiff(left, right) {
            match row.edit {
                Edit::Same => d.same += 1,
                Edit::Modified => d.modified += 1,
                Edit::Added => d.added += 1,
                Edit::Removed => d.removed += 1,
            }
        }
        d
    }
}

impl std::ops::AddAssign for Delta {
    fn add_assign(&mut self, rhs: Self) {
        self.same += rhs.same;
        self.modified += rhs.modified;
        self.added += rhs.added;
        self.removed += rhs.removed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    fn edits(a: &str, b: &str) -> Vec<Edit> {
        sdiff(&lines(a), &lines(b)).into_iter().map(|r| r.edit).collect()
    }

    #[test]
    fn identical_input_is_all_same() {
        assert_eq!(edits("a\nb\nc", "a\nb\nc"), vec![Edit::Same; 3]);
    }

    #[test]
    fn an_inserted_line_is_added() {
        assert_eq!(
            edits("a\nc", "a\nb\nc"),
            vec![Edit::Same, Edit::Added, Edit::Same]
        );
    }

    #[test]
    fn a_deleted_line_is_removed() {
        assert_eq!(
            edits("a\nb\nc", "a\nc"),
            vec![Edit::Same, Edit::Removed, Edit::Same]
        );
    }

    /// A rewritten line is one modification, not a removal plus an addition.
    /// The whole shape of cloc's diff totals depends on this.
    #[test]
    fn a_rewritten_line_is_one_modification() {
        assert_eq!(
            edits("a\nb\nc", "a\nB\nc"),
            vec![Edit::Same, Edit::Modified, Edit::Same]
        );
    }

    /// Where the sides are uneven, the overlap is modified and the surplus
    /// is added or removed.
    #[test]
    fn uneven_runs_pair_off_then_spill() {
        assert_eq!(
            edits("a\nx\ny\nb", "a\nX\nb"),
            vec![Edit::Same, Edit::Modified, Edit::Removed, Edit::Same]
        );
        assert_eq!(
            edits("a\nx\nb", "a\nX\nY\nb"),
            vec![Edit::Same, Edit::Modified, Edit::Added, Edit::Same]
        );
    }

    #[test]
    fn empty_sides() {
        assert_eq!(edits("", "a\nb"), vec![Edit::Added, Edit::Added]);
        assert_eq!(edits("a\nb", ""), vec![Edit::Removed, Edit::Removed]);
        assert!(edits("", "").is_empty());
    }

    /// Every left line appears once in the rows, and so does every right
    /// line: a diff must not lose or duplicate input.
    #[test]
    fn every_line_is_accounted_for() {
        let a = lines("one\ntwo\nthree\nfour\nfive");
        let b = lines("one\nTWO\nthree\nfive\nsix");
        let rows = sdiff(&a, &b);
        let left: Vec<&String> = rows.iter().filter_map(|r| r.left.as_ref()).collect();
        let right: Vec<&String> = rows.iter().filter_map(|r| r.right.as_ref()).collect();
        assert_eq!(left, a.iter().collect::<Vec<_>>());
        assert_eq!(right, b.iter().collect::<Vec<_>>());
    }

    /// Repeated lines must not confuse the index-based subsequence search.
    #[test]
    fn repeated_lines_are_handled() {
        let rows = sdiff(&lines("x\nx\nx"), &lines("x\nx"));
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().filter(|r| r.edit == Edit::Same).count(), 2);
        assert_eq!(rows.iter().filter(|r| r.edit == Edit::Removed).count(), 1);
    }

    #[test]
    fn delta_counts_each_category() {
        let d = Delta::between(&lines("a\nb\nc\nd"), &lines("a\nB\nd\ne"));
        assert_eq!(d.same, 2); // a, d
        assert_eq!(d.modified, 1); // b -> B
        assert_eq!(d.removed, 1); // c
        assert_eq!(d.added, 1); // e
    }

    /// The flags line up one-to-one with the input lines.
    #[test]
    fn code_line_flags_align_with_the_input() {
        let all = lines("a;\n// note\nb;");
        let code = lines("a;\nb;");
        assert_eq!(code_line_flags(&all, &code), vec![true, false, true]);
    }

    /// Comment classification falls out of diffing a file against its
    /// comment-stripped self.
    #[test]
    fn code_and_comments_are_split_by_diffing() {
        let all = lines("int x;\n/* note */\nint y;");
        let code_only = lines("int x;\nint y;");
        let (code, comments) = split_code_and_comments(&all, &code_only);
        assert_eq!(code, vec!["int x;", "int y;"]);
        assert_eq!(comments, vec!["/* note */"]);
    }

    /// A long file with one edit must not take quadratic time; this would
    /// hang rather than fail if the prefix and suffix trimming were absent.
    #[test]
    fn a_small_edit_to_a_large_file_is_fast() {
        let mut a: Vec<String> = (0..20_000).map(|i| format!("line {i}")).collect();
        let b = a.clone();
        a[10_000] = "changed".to_string();
        let d = Delta::between(&a, &b);
        assert_eq!(d.modified, 1);
        assert_eq!(d.same, 19_999);
    }
}
