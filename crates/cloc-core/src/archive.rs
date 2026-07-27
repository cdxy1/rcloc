//! Unpacking archives so their contents can be counted.
//!
//! Whether a file is an archive is decided by whether an extraction command
//! can be built for it, exactly as in the original — there is a table of
//! archive extensions too, but it governs only the search for *nested*
//! archives, so the two lists differ and `.whl` is in one and not the other.
//!
//! Extraction shells out to `tar`, `unzip` and friends rather than linking
//! decompressors, which is what cloc does and keeps the formats supported in
//! step with whatever is installed.

use anyhow::{bail, Context, Result};
use cloc_lang::LangDb;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Options governing extraction.
#[derive(Debug, Clone, Default)]
pub struct ArchiveOptions {
    /// `--extract-with=CMD`: a command with `>FILE<` standing for the
    /// archive. Given, it is used for *every* input.
    pub extract_with: Option<String>,
    /// `--sdir=DIR`: extract here instead of a temporary directory, leaving
    /// the results behind for inspection.
    pub sdir: Option<PathBuf>,
}

/// Where inputs were unpacked. Dropping this removes the temporary
/// directories, so it must outlive the counting pass.
#[derive(Debug, Default)]
pub struct Extraction {
    /// Directories to count in place of the original archive arguments.
    pub dirs: Vec<PathBuf>,
    /// Temporary directories, kept alive for as long as this value lives.
    temp_dirs: Vec<tempfile::TempDir>,
    /// Counter for naming subdirectories under `--sdir`.
    next_sdir: usize,
}

/// The shell command that unpacks `path` into the current directory, or
/// `None` if this is not something cloc knows how to unpack.
///
/// The single-quoting of the path matters: archive names routinely contain
/// spaces, and this text is handed to a shell.
pub fn extraction_command(path: &Path, opts: &ArchiveOptions) -> Result<Option<String>> {
    let display = path.to_string_lossy();

    if let Some(template) = &opts.extract_with {
        return Ok(Some(template.replace(">FILE<", &display)));
    }

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name == "-" {
        return Ok(Some("cat > -".to_string()));
    }

    let lower = display.to_lowercase();
    let quoted = format!("'{display}'");

    let tarball = lower.ends_with(".tar")
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".tar.z")
        || lower.ends_with(".tar.xz")
        || lower.ends_with(".tar.bz2")
        || lower.ends_with(".tgz")
        || lower.ends_with(".gem");

    if tarball {
        return Ok(Some(format!("tar xf {quoted}")));
    }
    if lower.ends_with(".src.rpm") {
        require("cpio")?;
        require("rpm2cpio")?;
        return Ok(Some(format!("rpm2cpio {quoted} | cpio -i")));
    }
    if lower.ends_with(".whl") || lower.ends_with(".zip") {
        require("unzip")?;
        return Ok(Some(format!("unzip -qq -d . {quoted}")));
    }
    if lower.ends_with(".deb") {
        // Only useful when the package carries source; most hold binaries.
        require("dpkg-deb")?;
        return Ok(Some(format!("dpkg-deb -x {quoted} .")));
    }
    Ok(None)
}

/// Fail with cloc's advice when a needed helper is missing.
fn require(utility: &str) -> Result<()> {
    let found = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {utility}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !found {
        bail!(
            "unable to expand the archive because the external utility \
             '{utility}' is not available; another possibility is to use \
             --extract-with"
        );
    }
    Ok(())
}

impl Extraction {
    /// Make somewhere to unpack into.
    fn new_dir(&mut self, opts: &ArchiveOptions) -> Result<PathBuf> {
        match &opts.sdir {
            Some(base) => {
                self.next_sdir += 1;
                let dir = base.join(self.next_sdir.to_string());
                if dir.is_dir() {
                    std::fs::remove_dir_all(&dir)
                        .with_context(|| format!("clearing {}", dir.display()))?;
                }
                std::fs::create_dir_all(&dir)
                    .with_context(|| format!("creating {}", dir.display()))?;
                Ok(dir)
            }
            None => {
                let temp = tempfile::tempdir().context("creating a temporary directory")?;
                let path = temp.path().to_path_buf();
                self.temp_dirs.push(temp);
                Ok(path)
            }
        }
    }

    fn unpack(&mut self, archive: &Path, command: &str, opts: &ArchiveOptions) -> Result<PathBuf> {
        let dir = self.new_dir(opts)?;
        let status = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&dir)
            .status()
            .with_context(|| format!("running: {command}"))?;
        if !status.success() {
            bail!("extracting {} failed: {command}", archive.display());
        }
        self.dirs.push(dir.clone());
        Ok(dir)
    }
}

/// Replace any archive among `inputs` with the directory it unpacks to.
///
/// Archives can nest — a Java `.ear` holds `.war` files — so the extracted
/// trees are rescanned until no new archives turn up. Each nested archive is
/// deleted once unpacked, or the scan would find it again forever.
pub fn expand_inputs(
    inputs: &[PathBuf],
    db: &LangDb,
    opts: &ArchiveOptions,
) -> Result<(Vec<PathBuf>, Extraction)> {
    let mut extraction = Extraction::default();
    let mut remaining = Vec::new();

    for input in inputs {
        if input.is_dir() {
            remaining.push(input.clone());
            continue;
        }
        let absolute = std::fs::canonicalize(input).unwrap_or_else(|_| input.clone());
        match extraction_command(&absolute, opts)? {
            Some(cmd) => {
                extraction.unpack(&absolute, &cmd, opts)?;
            }
            None => remaining.push(input.clone()),
        }
    }

    // Nested archives, found by extension this time.
    let mut scanned_from = 0;
    while scanned_from < extraction.dirs.len() {
        let frontier: Vec<PathBuf> = extraction.dirs[scanned_from..].to_vec();
        scanned_from = extraction.dirs.len();

        for dir in frontier {
            for nested in find_archives(&dir, db) {
                let Some(cmd) = extraction_command(&nested, opts)? else {
                    continue;
                };
                extraction.unpack(&nested, &cmd, opts)?;
                // Otherwise the next sweep finds it again.
                let _ = std::fs::remove_file(&nested);
            }
        }
    }

    remaining.extend(extraction.dirs.iter().cloned());
    Ok((remaining, extraction))
}

/// Files under `dir` whose name ends with a known archive extension.
fn find_archives(dir: &Path, db: &LangDb) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let extensions = db.binary_archive_extensions();
    for entry in walkdir::WalkDir::new(dir).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if extensions.iter().any(|ext| name.ends_with(ext.as_str())) {
            found.push(entry.path().to_path_buf());
        }
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(name: &str) -> Option<String> {
        extraction_command(Path::new(name), &ArchiveOptions::default()).unwrap()
    }

    #[test]
    fn tarballs_use_tar() {
        assert_eq!(cmd("x.tar").as_deref(), Some("tar xf 'x.tar'"));
        assert_eq!(cmd("x.tar.gz").as_deref(), Some("tar xf 'x.tar.gz'"));
        assert_eq!(cmd("x.tgz").as_deref(), Some("tar xf 'x.tgz'"));
        // A Ruby gem is a tarball under another name.
        assert_eq!(cmd("x.gem").as_deref(), Some("tar xf 'x.gem'"));
    }

    #[test]
    fn zips_and_wheels_use_unzip() {
        assert_eq!(cmd("x.zip").as_deref(), Some("unzip -qq -d . 'x.zip'"));
        assert_eq!(cmd("x.whl").as_deref(), Some("unzip -qq -d . 'x.whl'"));
    }

    /// Extensions are matched case-insensitively, so `.ZIP` works too.
    #[test]
    fn extension_matching_ignores_case() {
        assert!(cmd("X.ZIP").is_some());
        assert!(cmd("X.Tar.GZ").is_some());
    }

    #[test]
    fn ordinary_files_are_not_archives() {
        assert!(cmd("main.rs").is_none());
        assert!(cmd("notes.txt").is_none());
        assert!(cmd("archive").is_none());
    }

    /// A name with a space must survive the trip through the shell.
    #[test]
    fn paths_are_quoted() {
        assert_eq!(
            cmd("my archive.zip").as_deref(),
            Some("unzip -qq -d . 'my archive.zip'")
        );
    }

    /// `--extract-with` overrides the built-in table for every input.
    #[test]
    fn extract_with_substitutes_the_placeholder() {
        let opts = ArchiveOptions {
            extract_with: Some("7z x >FILE<".to_string()),
            ..Default::default()
        };
        let c = extraction_command(Path::new("x.rar"), &opts).unwrap();
        assert_eq!(c.as_deref(), Some("7z x x.rar"));
    }

    /// An unpacked archive's contents replace it in the input list.
    #[test]
    fn expanding_a_zip_yields_its_contents() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("a.py"), "x = 1\n").unwrap();

        let zip = dir.path().join("bundle.zip");
        let ok = Command::new("sh")
            .arg("-c")
            .arg(format!("cd {} && zip -qr {} src", dir.path().display(), zip.display()))
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("skipping: zip is not installed");
            return;
        }

        let (inputs, _guard) = expand_inputs(
            &[zip],
            LangDb::default_db(),
            &ArchiveOptions::default(),
        )
        .unwrap();
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].join("src/a.py").exists());
    }
}
