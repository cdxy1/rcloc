//! Counting git revisions rather than working directories.
//!
//! `cloc --git HEAD~10 HEAD` compares two points in history without touching
//! the working tree. Each revision is exported with `git archive` into a
//! temporary directory and counted from there, which is both simpler and
//! safer than checking anything out.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A revision exported to disk. Dropping this removes the export.
#[derive(Debug)]
pub struct Export {
    pub dir: PathBuf,
    _temp: tempfile::TempDir,
}

impl Export {
    pub fn path(&self) -> &Path {
        &self.dir
    }
}

/// Whether `spec` names something git can resolve — a hash, tag or branch.
///
/// Checked only for inputs that are not existing paths, since a file called
/// `main` should be read as a file.
pub fn is_revision(spec: &str) -> bool {
    if Path::new(spec).exists() {
        return false;
    }
    Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", &format!("{spec}^{{commit}}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Export a revision's tree into a temporary directory.
pub fn export(spec: &str) -> Result<Export> {
    let temp = tempfile::tempdir().context("creating a temporary directory")?;
    let dir = temp.path().to_path_buf();

    // `git archive` writes a tar to stdout; unpacking it in the target
    // directory avoids disturbing the working tree or the index.
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "git -c \"safe.directory=*\" archive '{spec}' | tar xf - -C '{}'",
            dir.display()
        ))
        .status()
        .with_context(|| format!("exporting git revision {spec}"))?;
    if !status.success() {
        bail!("could not export git revision {spec}");
    }

    Ok(Export { dir, _temp: temp })
}

/// Paths that differ between two revisions.
///
/// This is what `--git-diff-rel` narrows the comparison to: files untouched
/// between the two commits cannot contribute to the diff, so counting them is
/// wasted work.
pub fn changed_files(from: &str, to: &str) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(["-c", "safe.directory=*", "diff", "--name-only", from, to])
        .output()
        .with_context(|| format!("listing changes between {from} and {to}"))?;
    if !output.status.success() {
        bail!(
            "git diff --name-only {from} {to} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// Keep only the files under `root` whose relative path is in `wanted`.
pub fn restrict_to(files: Vec<PathBuf>, root: &Path, wanted: &[String]) -> Vec<PathBuf> {
    files
        .into_iter()
        .filter(|p| {
            let relative = p.strip_prefix(root).unwrap_or(p).to_string_lossy().into_owned();
            wanted.iter().any(|w| *w == relative)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An existing path is a path, even if it would also parse as a
    /// revision; otherwise a branch named after a directory would shadow it.
    #[test]
    fn existing_paths_are_not_revisions() {
        assert!(!is_revision("."));
        assert!(!is_revision("Cargo.toml"));
    }

    #[test]
    fn nonsense_is_not_a_revision() {
        assert!(!is_revision("definitely-not-a-ref-9f8e7d6c"));
    }

    /// The rest of the module needs a repository, so these run only when
    /// there is one to hand.
    fn in_repo() -> bool {
        Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn head_resolves_in_a_repository() {
        if !in_repo() {
            return;
        }
        assert!(is_revision("HEAD"));
    }

    /// `git archive` limits itself to the working directory's subtree, so
    /// running from a crate directory exports that crate. That is git's
    /// behaviour and worth keeping: `cloc --git HEAD` in a subdirectory
    /// counts the subdirectory.
    #[test]
    fn exporting_head_yields_the_tracked_files() {
        if !in_repo() {
            return;
        }
        let export = export("HEAD").expect("export HEAD");
        assert!(export.path().join("Cargo.toml").exists());
        assert!(export.path().join("src").is_dir());
    }

    #[test]
    fn restrict_keeps_only_the_named_paths() {
        let root = Path::new("/tmp/x");
        let files = vec![
            PathBuf::from("/tmp/x/a.rs"),
            PathBuf::from("/tmp/x/sub/b.rs"),
            PathBuf::from("/tmp/x/c.rs"),
        ];
        let kept = restrict_to(files, root, &["a.rs".into(), "sub/b.rs".into()]);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().any(|p| p.ends_with("a.rs")));
        assert!(kept.iter().any(|p| p.ends_with("sub/b.rs")));
    }
}
