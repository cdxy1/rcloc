//! Comment scanners standing in for `Regexp::Common`'s `$RE{comment}{...}`.
//!
//! The Perl original hands the whole file to a generated regex and deletes
//! every match. Only seven dialects are ever requested, so each gets a
//! hand-written scanner here: faster, and free of the catastrophic
//! backtracking that forces the original to wrap every filter in an alarm.
//!
//! These scanners deliberately do **not** understand string literals. Neither
//! does `Regexp::Common`, so `"/* not a comment */"` is stripped by both. The
//! optional `--strip-str-comments` pre-pass exists precisely because of this,
//! and "fixing" it here would make the counts disagree with cloc.

use cloc_lang::CommentDialect;

/// Remove comments of `dialect` from `text`, returning the remainder.
///
/// Comment bodies are replaced by nothing, exactly as `s/$1//g` does, so a
/// line that was entirely a comment becomes empty rather than disappearing;
/// the caller drops blank lines afterwards.
pub fn strip_comments(text: &str, dialect: CommentDialect) -> String {
    match dialect {
        CommentDialect::C => strip_block(text, "/*", "*/"),
        CommentDialect::Cpp => strip_c_style(text, true),
        CommentDialect::Html => strip_block(text, "<!--", "-->"),
        CommentDialect::Pascal => strip_pascal(text),
        CommentDialect::Smalltalk => strip_smalltalk(text),
        CommentDialect::PlSql => strip_pl_sql(text),
        CommentDialect::Brainfuck => strip_brainfuck(text),
    }
}

/// Remove `start ... end` spans.
///
/// An **unterminated** opener removes nothing: the generated regexes require
/// a closing delimiter, so a lone `/*` simply fails to match and the rest of
/// the file stays code. Truncating there instead would silently swallow
/// everything after, say, a `text/*` inside a Python docstring.
fn strip_block(text: &str, start: &str, end: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;

    while let Some(open) = find_from(text, start, pos) {
        let body = open + start.len();
        match find_from(text, end, body) {
            Some(close) => {
                out.push_str(&text[pos..open]);
                // Newlines inside the comment go with it; the Perl
                // substitution does the same, so line counts agree.
                pos = close + end.len();
            }
            None => {
                // Keep the opener as ordinary text and carry on past it, so a
                // later comment is still found.
                out.push_str(&text[pos..body]);
                pos = body;
            }
        }
    }
    out.push_str(&text[pos..]);
    out
}

/// C-style comments: `/* ... */` plus, when `line_comments`, `// ...`.
///
/// A `//` inside a `/* */` span is not a comment start, and vice versa; a
/// single left-to-right scan gets this right where two independent passes
/// would not.
fn strip_c_style(text: &str, line_comments: bool) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'*' {
                match find_from(text, "*/", i + 2) {
                    Some(close) => {
                        i = close + 2;
                        continue;
                    }
                    // Unterminated: not a comment at all. Emit the opener and
                    // keep scanning, so a later `//` is still recognised.
                    None => {
                        out.push_str("/*");
                        i += 2;
                        continue;
                    }
                }
            }
            if line_comments && bytes[i + 1] == b'/' {
                // A `//` comment takes its terminating newline with it. That
                // is invisible in ordinary C++ because the caller hands this
                // scanner two newlines per line, but it is what merges lines
                // when only one is present -- see `call_regexp_common`.
                //
                // A C compiler would also let a trailing backslash continue
                // the comment onto the next line; Regexp::Common does not,
                // and cloc counts the continued line as code.
                i = past_end_of_line(bytes, i + 2);
                continue;
            }
        }
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Pascal: `{ ... }`, `(* ... *)` and `// ...`.
fn strip_pascal(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(close) = find_from(text, "}", i + 1) {
                i = close + 1;
                continue;
            }
            out.push('{');
            i += 1;
            continue;
        }
        if bytes[i] == b'(' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            if let Some(close) = find_from(text, "*)", i + 2) {
                i = close + 2;
                continue;
            }
            out.push_str("(*");
            i += 2;
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i = end_of_line(bytes, i + 2);
            continue;
        }
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Smalltalk: comments are delimited by double quotes, with `""` escaping a
/// literal quote inside one.
fn strip_smalltalk(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'"' {
            let mut j = i + 1;
            let mut closed = false;
            while let Some(close) = find_from(text, "\"", j) {
                if close + 1 < bytes.len() && bytes[close + 1] == b'"' {
                    j = close + 2; // doubled quote: stay in the comment
                    continue;
                }
                i = close + 1;
                closed = true;
                break;
            }
            if !closed {
                out.push('"');
                i += 1;
            }
            continue;
        }
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// PL/SQL: `/* ... */` and `-- ...`.
fn strip_pl_sql(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            if let Some(close) = find_from(text, "*/", i + 2) {
                i = close + 2;
                continue;
            }
            out.push_str("/*");
            i += 2;
            continue;
        }
        if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            i = end_of_line(bytes, i + 2);
            continue;
        }
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Brainfuck: every character outside the eight commands is a comment.
/// Newlines are kept so the line structure survives for the counter.
fn strip_brainfuck(text: &str) -> String {
    text.chars()
        .filter(|c| matches!(c, '<' | '>' | '+' | '-' | '.' | ',' | '[' | ']' | '\n'))
        .collect()
}

/// Byte index of the next newline at or after `from`, or end of input. The
/// newline itself is left in place.
fn end_of_line(bytes: &[u8], from: usize) -> usize {
    match memchr::memchr(b'\n', &bytes[from..]) {
        Some(off) => from + off,
        None => bytes.len(),
    }
}

/// As [`end_of_line`], but consuming the newline as well.
fn past_end_of_line(bytes: &[u8], from: usize) -> usize {
    match memchr::memchr(b'\n', &bytes[from..]) {
        Some(off) => from + off + 1,
        None => bytes.len(),
    }
}

fn find_from(text: &str, needle: &str, from: usize) -> Option<usize> {
    if from > text.len() {
        return None;
    }
    text[from..].find(needle).map(|off| from + off)
}

/// Length in bytes of the UTF-8 sequence starting with `b`.
fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_block_comments() {
        assert_eq!(strip_comments("a /* x */ b", CommentDialect::C), "a  b");
        // `//` is not a comment in plain C.
        assert_eq!(strip_comments("a // b", CommentDialect::C), "a // b");
    }

    #[test]
    fn cpp_line_and_block_comments() {
        // The line comment takes its newline with it; the filter layer
        // compensates by supplying two.
        assert_eq!(strip_comments("a // b\nc", CommentDialect::Cpp), "a c");
        assert_eq!(strip_comments("a // b\n\nc", CommentDialect::Cpp), "a \nc");
        assert_eq!(strip_comments("a /* b */ c", CommentDialect::Cpp), "a  c");
        // `//` inside a block comment does not end it early.
        assert_eq!(strip_comments("a /* // */ b", CommentDialect::Cpp), "a  b");
        // `/*` inside a line comment does not open a block.
        assert_eq!(strip_comments("a // /* \n\nb", CommentDialect::Cpp), "a \nb");
    }

    /// A C compiler continues a `//` comment past a trailing backslash.
    /// Regexp::Common does not, so cloc counts the next line as code and so
    /// must we -- languages whose strings use backslash continuation (Rust,
    /// C itself) would otherwise lose lines wholesale.
    #[test]
    fn cpp_line_comment_stops_at_the_newline_despite_a_backslash() {
        assert_eq!(
            strip_comments("a // b\\\n\nstill code\n\nc", CommentDialect::Cpp),
            "a \nstill code\n\nc"
        );
    }

    /// An unterminated opener is not a comment: Regexp::Common needs a
    /// closing delimiter, so cloc leaves the rest of the file as code.
    #[test]
    fn unterminated_block_removes_nothing() {
        assert_eq!(strip_comments("a /* b\nc", CommentDialect::C), "a /* b\nc");
    }

    /// A complete comment is still removed even when an unterminated opener
    /// follows it, and a later line comment is still found.
    #[test]
    fn scanning_continues_past_an_unterminated_opener() {
        assert_eq!(
            strip_comments("a /* x */ b /* c\nd // e\n\n", CommentDialect::Cpp),
            "a  b /* c\nd \n"
        );
    }

    /// Matching cloc means matching its blind spot: comment markers inside
    /// string literals are stripped, because Regexp::Common does that too.
    #[test]
    fn strings_are_not_respected() {
        assert_eq!(
            strip_comments(r#"s = "/* not a comment */";"#, CommentDialect::Cpp),
            r#"s = "";"#
        );
    }

    #[test]
    fn html_comments() {
        assert_eq!(
            strip_comments("<p>a</p><!-- c -->b", CommentDialect::Html),
            "<p>a</p>b"
        );
    }

    #[test]
    fn pascal_comments() {
        assert_eq!(strip_comments("a { c } b", CommentDialect::Pascal), "a  b");
        assert_eq!(strip_comments("a (* c *) b", CommentDialect::Pascal), "a  b");
        assert_eq!(strip_comments("a // c\nb", CommentDialect::Pascal), "a \nb");
    }

    #[test]
    fn smalltalk_doubled_quote_stays_inside_comment() {
        assert_eq!(
            strip_comments(r#"a "c ""q"" c" b"#, CommentDialect::Smalltalk),
            "a  b"
        );
    }

    #[test]
    fn pl_sql_comments() {
        assert_eq!(strip_comments("a -- c\nb", CommentDialect::PlSql), "a \nb");
        assert_eq!(strip_comments("a /* c */ b", CommentDialect::PlSql), "a  b");
    }

    #[test]
    fn brainfuck_keeps_only_commands() {
        assert_eq!(
            strip_comments("add two: ++[->+<]\ndone", CommentDialect::Brainfuck),
            "++[->+<]\n"
        );
    }

    #[test]
    fn multibyte_text_survives() {
        assert_eq!(
            strip_comments("привет /* c */ мир", CommentDialect::Cpp),
            "привет  мир"
        );
    }
}
