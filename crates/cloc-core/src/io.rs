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
        // A file holding nothing but a byte-order mark still has a line in
        // it. The original forces a trailing newline onto the raw content
        // and only then strips the mark, which leaves one blank line behind;
        // stripping first and finding nothing left would lose it.
        return if bytes.is_empty() {
            Vec::new()
        } else {
            vec![String::new()]
        };
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

/// Whether a file looks binary. Binary files are skipped unless
/// `--read-binary-files`.
///
/// cloc delegates this to Perl's `-B`, so we reimplement that rather than
/// invent a rule: it decides from the first 512 bytes, and its verdict is
/// what keeps, say, a Latin-1 encoded source file out of the counts.
pub fn is_binary(path: &Path) -> Result<bool> {
    let mut buf = [0u8; 512];
    let n = File::open(path)?.read(&mut buf)?;
    Ok(looks_binary(&buf[..n]))
}

/// Perl's `-B` heuristic: a NUL anywhere in the block means binary, and
/// otherwise a block is binary when more than a third of it is "odd".
///
/// Odd means a control character other than the usual whitespace and escape,
/// or a high-bit byte that is not part of a valid UTF-8 sequence. That last
/// clause is why a UTF-8 file full of Cyrillic counts as text while the same
/// text in Latin-1 does not.
pub fn looks_binary(block: &[u8]) -> bool {
    if block.is_empty() {
        return false;
    }
    if block.contains(&0) {
        return true;
    }

    let mut odd = 0usize;
    let mut i = 0usize;
    while i < block.len() {
        let b = block[i];
        if b & 0x80 != 0 {
            match utf8_sequence_len(&block[i..]) {
                Some(len) => {
                    i += len;
                    continue;
                }
                None => odd += 1,
            }
        } else if b < 32 && !matches!(b, b'\n' | b'\r' | 8 | b'\t' | 12 | 27) {
            odd += 1;
        }
        i += 1;
    }

    odd * 3 > block.len()
}

/// Length of the valid UTF-8 sequence starting at `bytes[0]`, if there is one.
///
/// A sequence cut off by the end of the block is accepted at its available
/// length: the block boundary is arbitrary and should not make a text file
/// look binary.
fn utf8_sequence_len(bytes: &[u8]) -> Option<usize> {
    let first = bytes[0];
    let len = match first {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None,
    };
    let available = len.min(bytes.len());
    for &b in &bytes[1..available] {
        if b & 0xC0 != 0x80 {
            return None;
        }
    }
    Some(available)
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

    /// A file consisting only of a byte-order mark is one blank line, not
    /// an empty file.
    #[test]
    fn a_lone_bom_is_one_blank_line() {
        assert_eq!(lines_from_bytes(b"\xef\xbb\xbf"), vec![""]);
    }

    #[test]
    fn plain_ascii_is_text() {
        assert!(!looks_binary(b"int main() { return 0; }\n"));
    }

    #[test]
    fn a_nul_byte_means_binary() {
        assert!(looks_binary(b"ELF\x00\x01\x02 and then some text"));
    }

    /// The same words in UTF-8 and in Latin-1 must land on opposite sides:
    /// valid UTF-8 is text, lone high bytes are not.
    #[test]
    fn utf8_is_text_but_latin1_is_binary() {
        assert!(!looks_binary("Русский текст в файле".as_bytes()));
        let latin1: Vec<u8> = (0..40).map(|i| 0xC0 + (i % 30) as u8).collect();
        assert!(looks_binary(&latin1));
    }

    /// A little high-bit content in mostly-ASCII text is still text.
    #[test]
    fn occasional_odd_bytes_stay_text() {
        let mut block = b"plain ascii source code line after line, ".to_vec();
        block.push(0xFF);
        assert!(!looks_binary(&block));
    }

    #[test]
    fn empty_block_is_not_binary() {
        assert!(!looks_binary(b""));
    }
}
