//! Reading source files the way cloc does.
//!
//! Three normalisations happen on the way in, all of them observable in the
//! counts: a byte-order mark is dropped, CR is stripped from CRLF endings so
//! Windows and Unix copies of a file count the same, and a file not ending in
//! a newline is treated as though it did.

use anyhow::{Context, Result};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

/// Byte-order marks cloc recognises, longest first so a prefix does not mask
/// a longer mark.
const BOMS: &[&[u8]] = &[
    &[0x2b, 0x2f, 0x76, 0x38, 0x2d],
    &[0x00, 0x00, 0xfe, 0xff],
    &[0xff, 0xfe, 0x00, 0x00],
    &[0x2b, 0x2f, 0x76, 0x38],
    &[0x2b, 0x2f, 0x76, 0x39],
    &[0x2b, 0x2f, 0x76, 0x2b],
    &[0x2b, 0x2f, 0x76, 0x2f],
    &[0xdd, 0x73, 0x66, 0x73],
    &[0x84, 0x31, 0x95, 0x33],
    &[0xef, 0xbb, 0xbf],
    &[0xf7, 0x64, 0x4c],
    &[0x0e, 0xfe, 0xff],
    &[0xfb, 0xee, 0x28],
    &[0xfe, 0xff],
    &[0xff, 0xfe],
];

/// Read a file into lines, without their terminators.
pub fn read_lines(path: &Path) -> Result<Vec<String>> {
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(lines_from_bytes(&bytes))
}

/// Split raw file bytes into lines, applying cloc's normalisations.
///
/// Invalid UTF-8 is replaced rather than rejected: cloc counts such files, and
/// refusing them would lose lines it counts.
pub fn lines_from_bytes(bytes: &[u8]) -> Vec<String> {
    let body = BOMS
        .iter()
        .find(|bom| bytes.starts_with(bom))
        .map_or(bytes, |bom| &bytes[bom.len()..]);

    let text = String::from_utf8_lossy(body);
    if text.is_empty() {
        return Vec::new();
    }
    let text = text.strip_suffix('\n').unwrap_or(&text);
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
        .collect()
}

/// The first line of a file, for shebang and XML-declaration checks.
pub fn first_line(path: &Path) -> Result<Option<String>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut line = Vec::new();
    let mut reader = BufReader::new(file);
    if reader.read_until(b'\n', &mut line)? == 0 {
        return Ok(None);
    }
    Ok(lines_from_bytes(&line).into_iter().next())
}

/// Whether a file looks binary, using cloc's rule: a NUL byte in the first
/// chunk. Binary files are skipped unless `--read-binary-files`.
pub fn is_binary(path: &Path) -> Result<bool> {
    let mut buf = [0u8; 8192];
    let n = File::open(path)?.read(&mut buf)?;
    Ok(buf[..n].contains(&0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_bom_is_stripped() {
        let bytes = b"\xef\xbb\xbfint x;\n";
        assert_eq!(lines_from_bytes(bytes), vec!["int x;"]);
    }

    #[test]
    fn crlf_endings_match_lf() {
        assert_eq!(
            lines_from_bytes(b"a\r\nb\r\n"),
            lines_from_bytes(b"a\nb\n")
        );
    }

    /// A file with no final newline still has its last line counted.
    #[test]
    fn missing_final_newline_still_counts() {
        assert_eq!(lines_from_bytes(b"a\nb"), vec!["a", "b"]);
    }

    #[test]
    fn empty_file_has_no_lines() {
        assert!(lines_from_bytes(b"").is_empty());
    }

    /// A single newline is one blank line, not zero.
    #[test]
    fn lone_newline_is_one_blank_line() {
        assert_eq!(lines_from_bytes(b"\n"), vec![""]);
    }

    #[test]
    fn invalid_utf8_does_not_lose_lines() {
        assert_eq!(lines_from_bytes(b"a\n\xff\xfe_bad\nc").len(), 3);
    }
}
