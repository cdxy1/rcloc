//! `--html`: a marked-up copy of each counted file.
//!
//! One `<source>.html` per file, with every line shown as code or comment.
//! This changes nothing about the counts; it is a way of checking them, since
//! a line classified surprisingly is easier to see than to deduce.

use cloc_core::diff;
use std::path::Path;

/// Render one file, given its lines and the comment-stripped version.
pub fn render(name: &str, without_blanks: &[String], without_comments: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&header(name));

    // Reuse the classification the counter itself uses, so the page cannot
    // disagree with the numbers it is meant to explain. Comparing the
    // original text against the stripped text line by line would not do:
    // stripping a trailing comment leaves a line that resembles neither.
    let flags = diff::code_line_flags(without_blanks, without_comments);

    let (mut code_num, mut comment_num) = (0usize, 0usize);
    for (line, &is_code) in without_blanks.iter().zip(flags.iter()) {
        if is_code {
            code_num += 1;
        } else {
            comment_num += 1;
        }
        let (class, num_class, num) = if is_code {
            ("normal", "linenum", code_num)
        } else {
            ("comment", "clinenum", comment_num)
        };
        out.push_str(&format!(
            "&nbsp; <span class=\"{num_class}\"> {num} </span> &nbsp;\
             <span class=\"{class}\">{}</span> &nbsp;\n",
            escape(line)
        ));
    }

    out.push_str("</tt></pre>\n</body>\n</html>\n");
    out
}

/// Where the marked-up copy goes: alongside the source, named after it.
pub fn output_path(source: &Path, original_dir: bool) -> std::path::PathBuf {
    let name = format!(
        "{}.html",
        source.file_name().unwrap_or_default().to_string_lossy()
    );
    if original_dir {
        source.with_file_name(name)
    } else {
        std::path::PathBuf::from(name)
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn header(name: &str) -> String {
    format!(
        "<html>\n<head>\n\
         <meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\">\n\
         <meta name=\"GENERATOR\" content=\"cloc-rs\">\n\n\
         <title>{}</title>\n\n\
         <style TYPE=\"text/css\">\n<!--\n\
         \x20   body {{\n        color: black;\n        background-color: white;\n\
         \x20       font-family: monospace\n    }}\n\n\
         \x20   .comment {{\n        color: gray;\n        font-style: italic;\n    }}\n\n\
         \x20   .clinenum {{\n        color: red;\n    }}\n\n\
         \x20   .linenum {{\n        color: green;\n    }}\n -->\n</style>\n\
         </head>\n<body>\n<pre><tt>\n",
        escape(name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn code_and_comments_get_different_classes() {
        let all = lines("int x;\n/* note */\nint y;");
        let code = lines("int x;\nint y;");
        let html = render("a.c", &all, &code);
        assert!(html.contains("class=\"normal\">int x;"));
        assert!(html.contains("class=\"comment\">/* note */"));
        assert!(html.contains("class=\"normal\">int y;"));
    }

    /// Code and comment lines are numbered on separate sequences, which is
    /// how the original presents them.
    #[test]
    fn the_two_kinds_are_numbered_separately() {
        let all = lines("a;\n// one\n// two\nb;");
        let code = lines("a;\nb;");
        let html = render("a.c", &all, &code);
        assert!(html.contains("class=\"linenum\"> 1 </span>"));
        assert!(html.contains("class=\"linenum\"> 2 </span>"));
        assert!(html.contains("class=\"clinenum\"> 1 </span>"));
        assert!(html.contains("class=\"clinenum\"> 2 </span>"));
    }

    /// Source is escaped, or a file containing markup would break the page.
    #[test]
    fn markup_in_the_source_is_escaped() {
        let all = lines("if (a < b && c > d)");
        let html = render("a.c", &all, &all);
        assert!(html.contains("a &lt; b &amp;&amp; c &gt; d"));
        assert!(!html.contains("a < b &&"));
    }

    #[test]
    fn output_lands_beside_the_source_only_when_asked() {
        let src = Path::new("src/deep/main.rs");
        assert_eq!(output_path(src, false), Path::new("main.rs.html"));
        assert_eq!(output_path(src, true), Path::new("src/deep/main.rs.html"));
    }
}
