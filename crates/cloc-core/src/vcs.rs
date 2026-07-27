//! Taking the file list from a version control system instead of the disk.
//!
//! `--vcs=git` counts what git tracks, which is usually what you want: no
//! build output, no editor droppings, no vendored dependencies. Any other
//! value is treated as a command to run, so `--vcs="find . -name '*.c'"`
//! works too.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What `--vcs` resolved to.
#[derive(Debug, Clone)]
pub struct Generator {
    /// The shell command that lists files, one per line.
    pub command: String,
    /// Submodule directories to exclude, discovered alongside a git listing.
    pub exclude_dirs: HashSet<String>,
}

/// Turn a `--vcs` value into a command to run.
///
/// `auto` looks for a working copy in the current directory; the named
/// systems expand to their listing command; anything else is passed through
/// as a command in its own right.
pub fn resolve(vcs: &str, include_submodules: bool) -> Result<Generator> {
    let mut resolved = vcs.to_string();

    if resolved == "auto" {
        resolved = if Path::new(".git").is_dir() {
            "git".to_string()
        } else if Path::new(".svn").is_dir() {
            "svn".to_string()
        } else {
            anyhow::bail!("--vcs auto: unable to determine the versioning system");
        };
    }

    let mut exclude_dirs = HashSet::new();

    let command = match resolved.as_str() {
        "git" => {
            // safe.directory=* keeps git from refusing to read a tree owned
            // by another user, which is routine in containers and CI.
            let mut cmd = r#"git -c "safe.directory=*" ls-files"#.to_string();
            if include_submodules {
                cmd.push_str(" --recurse-submodules");
            } else {
                // Submodules are separate repositories; their contents are
                // not this project's code unless asked for.
                exclude_dirs = submodule_dirs()?;
            }
            cmd
        }
        "svn" => "svn list -R".to_string(),
        other => other.to_string(),
    };

    Ok(Generator {
        command,
        exclude_dirs,
    })
}

/// Directory names reported by `git submodule status`.
fn submodule_dirs() -> Result<HashSet<String>> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(r#"git -c "safe.directory=*" submodule status"#)
        .output();
    let Ok(output) = output else {
        return Ok(HashSet::new());
    };
    if !output.status.success() {
        return Ok(HashSet::new());
    }

    let mut dirs = HashSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        // ` <sha> path/to/sub (heads/master)` — the leading character marks
        // the submodule's state and the trailing parenthesis its revision.
        let line = line.trim_start();
        let without_ref = match line.rfind(" (") {
            Some(idx) => &line[..idx],
            None => line,
        };
        if let Some((_sha, dir)) = without_ref.split_once(' ') {
            let dir = dir.trim();
            if !dir.is_empty() {
                dirs.insert(dir.to_string());
            }
        }
    }
    Ok(dirs)
}

/// Run the generator and collect the files it names.
///
/// The command runs once per input directory, with its output taken as
/// relative to that directory. Files named on the command line narrow the
/// result to those paths rather than adding to it.
pub fn list_files(generator: &Generator, inputs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut wanted: Vec<PathBuf> = Vec::new();

    for input in inputs {
        if input.is_dir() {
            dirs.push(input.clone());
        } else if input.is_file() {
            wanted.push(input.clone());
        }
    }
    if dirs.is_empty() && wanted.is_empty() {
        dirs.push(PathBuf::from("."));
    }
    if dirs.is_empty() {
        // Only files were named; run the generator where we stand so their
        // prefixes can still be matched against its output.
        dirs.push(PathBuf::from("."));
    }

    let mut files = Vec::new();
    for dir in &dirs {
        let output = Command::new("sh")
            .arg("-c")
            .arg(&generator.command)
            .current_dir(dir)
            .output()
            .with_context(|| format!("running: {}", generator.command))?;
        if !output.status.success() {
            anyhow::bail!(
                "{} failed: {}",
                generator.command,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if line.is_empty() {
                continue;
            }
            // Always prefixed with the directory the generator ran in,
            // including a bare ".", so reported paths match the original's.
            let path = dir.join(line);
            if wanted.is_empty() || is_wanted(&path, &wanted) {
                files.push(path);
            }
        }
    }

    files.sort();
    files.dedup();
    Ok(files)
}

/// Whether a generated path is one of, or below, the files asked for.
fn is_wanted(path: &Path, wanted: &[PathBuf]) -> bool {
    let as_str = path.to_string_lossy();
    wanted
        .iter()
        .any(|w| as_str.starts_with(&*w.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_systems_expand_to_listing_commands() {
        assert!(resolve("git", true).unwrap().command.contains("ls-files"));
        assert!(resolve("git", true)
            .unwrap()
            .command
            .contains("--recurse-submodules"));
        assert_eq!(resolve("svn", false).unwrap().command, "svn list -R");
    }

    /// Anything unrecognised is a command in its own right, which is how
    /// `--vcs="find . -name '*.c'"` works.
    #[test]
    fn an_unknown_value_is_used_as_a_command() {
        let g = resolve("find . -type f", false).unwrap();
        assert_eq!(g.command, "find . -type f");
        assert!(g.exclude_dirs.is_empty());
    }

    #[test]
    fn a_generator_lists_what_it_prints() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a(){}").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn b(){}").unwrap();

        let generator = Generator {
            command: "ls".to_string(),
            exclude_dirs: HashSet::new(),
        };
        let files = list_files(&generator, &[dir.path().to_path_buf()]).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.rs", "b.rs"]);
    }

    /// Naming a file narrows the generator's output rather than adding to it.
    #[test]
    fn named_files_narrow_the_listing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a(){}").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn b(){}").unwrap();

        let generator = Generator {
            command: "ls".to_string(),
            exclude_dirs: HashSet::new(),
        };
        let wanted = dir.path().join("a.rs");
        let files = list_files(&generator, &[dir.path().to_path_buf(), wanted]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].to_string_lossy().ends_with("a.rs"));
    }
}
