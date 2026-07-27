//! Comment-stripping filter specifications.
//!
//! In the Perl original a filter is a positional array whose first element
//! names a subroutine and whose remaining elements are its arguments:
//!
//! ```text
//! 'C++' => [ [ 'rm_comments_in_strings', '"', '/*', '*/' ],
//!            [ 'call_regexp_common'    , 'C++'           ], ]
//! ```
//!
//! Those arrays arrive here as [`RawFilter`] and are resolved into [`Filter`],
//! a tagged enum, so that a missing or misspelled argument is a load-time
//! error rather than an `undef` surfacing halfway through a count.

use serde::Deserialize;
use std::fmt;

/// A filter exactly as it appears in `languages.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct RawFilter {
    pub filter: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// The comment-removal language of `Regexp::Common`, as used by
/// `call_regexp_common`. Only these seven appear across all 422 languages, so
/// each gets a hand-written scanner rather than a generated regex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommentDialect {
    /// `/* ... */` only.
    C,
    /// `/* ... */` and `// ...`, with backslash line continuation.
    Cpp,
    /// `<!-- ... -->`.
    Html,
    /// `{ ... }`, `(* ... *)` and `// ...`.
    Pascal,
    /// `" ... "` double-quote comments.
    Smalltalk,
    /// `/* ... */` and `-- ...`.
    PlSql,
    /// Everything that is not one of the eight Brainfuck commands.
    Brainfuck,
}

impl CommentDialect {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "C" => Self::C,
            "C++" => Self::Cpp,
            "HTML" => Self::Html,
            "Pascal" => Self::Pascal,
            "Smalltalk" => Self::Smalltalk,
            "PL/SQL" => Self::PlSql,
            "Brainfuck" => Self::Brainfuck,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Html => "HTML",
            Self::Pascal => "Pascal",
            Self::Smalltalk => "Smalltalk",
            Self::PlSql => "PL/SQL",
            Self::Brainfuck => "Brainfuck",
        }
    }
}

/// A resolved filter step.
///
/// Regex arguments are kept as source strings; compilation is deferred to the
/// counting engine so that only the languages actually encountered in a run
/// pay for it.
#[derive(Debug, Clone)]
pub enum Filter {
    /// Drop lines matching the regex.
    RemoveMatches { re: String },
    /// Blank out the matched part of a line, keeping whatever precedes it.
    RemoveInline { re: String },
    /// Replace comment markers appearing inside string literals with `xx`, so
    /// a later comment filter does not mistake them for real comments.
    RmCommentsInStrings {
        string_marker: String,
        start_comment: String,
        end_comment: String,
        multiline: bool,
    },
    /// Remove everything between two literal markers, spanning lines.
    RemoveBetweenGeneral { start: String, end: String },
    /// Remove everything between two regex markers, spanning lines.
    RemoveBetweenRegex { start: String, end: String },
    /// Like [`Filter::RemoveBetweenRegex`] but substitutes a replacement in
    /// place of the removed span.
    ///
    /// `replacement` is a Perl expression, not a plain string: the original
    /// applies it with `s///ee`, so `"xx"` means the literal `xx` while
    /// `"$1$2$1 + $1$3$1$4"` interpolates capture groups. `multiline`
    /// defaults to true, unlike [`Filter::RmCommentsInStrings`].
    ReplaceBetweenRegex {
        start: String,
        end: String,
        replacement: String,
        multiline: bool,
    },
    /// Replace regex matches with a replacement string.
    ReplaceRegex { re: String, replacement: String },
    /// Remove all lines above the first line matching the regex.
    RemoveAbove { re: String },
    /// Remove all lines below the first line matching the regex.
    RemoveBelow { re: String },
    /// Remove lines above `above` and below `below`.
    RemoveBelowAbove { below: String, above: String },
    /// `<!-- ... -->`, with the quirks of the Perl implementation.
    RemoveHtmlComments,
    /// Run a `Regexp::Common`-equivalent comment scanner.
    CallRegexpCommon { dialect: CommentDialect },
    /// Fortran fixed-form comments (`c`, `C`, `*`, `!` in column 1).
    RemoveF77Comments,
    /// Fortran free-form comments (`!`).
    RemoveF90Comments,
    /// COBOL comments (indicator area, column 7).
    RemoveCobolComments,
    /// JCL comments (`//*`).
    RemoveJclComments,
    /// JSP comments (`<%-- --%>`).
    RemoveJspComments,
    /// OCaml comments (`(* ... *)`, nesting).
    RemoveOCamlComments,
    /// Haskell comments (`--`, `{- -}`), with literate-Haskell handling driven
    /// by the file name.
    RemoveHaskellComments { arg: String },
    /// TLA+ comments (`\* ...`, `(* ... *)`).
    RemoveTlaPlusComments,
    /// Drop the code TLA+ tooling appends after the module terminator.
    RemoveTlaPlusGeneratedCode,
    /// Brainfuck: keep only the eight command characters.
    RemoveBfComments,
    /// Haml block comments (`-#`).
    RemoveHamlBlock,
    /// Pug block comments (`//`, `//-`).
    RemovePugBlock,
    /// Slim block comments (`/`).
    RemoveSlimBlock,
    /// Keep only fenced code blocks (R Markdown).
    ReduceToRmdCodeBlocks,
    /// Rewrite Python docstrings as C comments so the C scanner removes them.
    DocstringToC,
    /// Remove docstrings directly, respecting `--docstring-as-code`.
    DocstringRmComments,
    /// Rewrite Elixir `@doc` heredocs as C comments.
    ElixirDocToC,
    /// Rewrite Forth `( ... )` comments as C comments.
    ForthParenToC,
    /// Rewrite PowerShell `<# ... #>` comments as C comments.
    PowershellToC,
    /// Rewrite Smarty `{* ... *}` comments as C comments.
    SmartyToC,
    /// Extract source cells from a Jupyter notebook.
    JupyterNb,
    /// Parse a Civet file via the external `civet` tool.
    CallParseCivet,
    /// Re-attach a trailing newline to every line, so a following multi-line
    /// scanner sees line terminators rather than one run-together string.
    AddNewlines,
    /// Wrap the input in a prefix and postfix line.
    PrePostFix { prefix: String, postfix: String },
    /// Drop the final line.
    RmLastLine,
    /// A language that is never counted directly — the Perl source has `die`
    /// here. Reaching it is a bug in language resolution.
    Die { message: String },
}

/// Failure to resolve a [`RawFilter`] into a [`Filter`].
#[derive(Debug)]
pub struct FilterParseError {
    pub language: String,
    pub filter: String,
    pub detail: String,
}

impl fmt::Display for FilterParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "language {:?}: filter {:?}: {}",
            self.language, self.filter, self.detail
        )
    }
}

impl std::error::Error for FilterParseError {}

impl Filter {
    /// Resolve a raw filter spec for `language`.
    pub fn from_raw(language: &str, raw: &RawFilter) -> Result<Self, FilterParseError> {
        let err = |detail: String| FilterParseError {
            language: language.to_string(),
            filter: raw.filter.clone(),
            detail,
        };
        // Positional argument accessor: the Perl arrays are dense, so a
        // missing index means the table and this enum disagree.
        let arg = |i: usize| -> Result<String, FilterParseError> {
            raw.args
                .get(i)
                .cloned()
                .ok_or_else(|| err(format!("expected at least {} argument(s)", i + 1)))
        };
        let opt = |i: usize| raw.args.get(i).cloned().unwrap_or_default();

        Ok(match raw.filter.as_str() {
            "remove_matches" => Self::RemoveMatches { re: arg(0)? },
            "remove_inline" => Self::RemoveInline { re: arg(0)? },
            "rm_comments_in_strings" => Self::RmCommentsInStrings {
                string_marker: arg(0)?,
                start_comment: arg(1)?,
                end_comment: opt(2),
                // The Perl signature defaults $multiline_mode to 0.
                multiline: matches!(opt(3).as_str(), "1"),
            },
            "remove_between_general" => Self::RemoveBetweenGeneral {
                start: arg(0)?,
                end: arg(1)?,
            },
            "remove_between_regex" => Self::RemoveBetweenRegex {
                start: arg(0)?,
                end: arg(1)?,
            },
            "replace_between_regex" => Self::ReplaceBetweenRegex {
                start: arg(0)?,
                end: arg(1)?,
                replacement: arg(2)?,
                // The Perl signature defaults $multiline_mode to 1.
                multiline: !matches!(opt(3).as_str(), "0"),
            },
            "replace_regex" => Self::ReplaceRegex {
                re: arg(0)?,
                replacement: opt(1),
            },
            "remove_above" => Self::RemoveAbove { re: arg(0)? },
            "remove_below" => Self::RemoveBelow { re: arg(0)? },
            "remove_below_above" => Self::RemoveBelowAbove {
                below: arg(0)?,
                above: arg(1)?,
            },
            "remove_html_comments" => Self::RemoveHtmlComments,
            "call_regexp_common" => {
                let name = arg(0)?;
                let dialect = CommentDialect::parse(&name)
                    .ok_or_else(|| err(format!("unknown comment dialect {name:?}")))?;
                Self::CallRegexpCommon { dialect }
            }
            "remove_f77_comments" => Self::RemoveF77Comments,
            "remove_f90_comments" => Self::RemoveF90Comments,
            "remove_cobol_comments" => Self::RemoveCobolComments,
            "remove_jcl_comments" => Self::RemoveJclComments,
            "remove_jsp_comments" => Self::RemoveJspComments,
            "remove_OCaml_comments" => Self::RemoveOCamlComments,
            "remove_haskell_comments" => Self::RemoveHaskellComments { arg: opt(0) },
            "remove_TLAPlus_comments" => Self::RemoveTlaPlusComments,
            "remove_TLAPlus_generated_code" => Self::RemoveTlaPlusGeneratedCode,
            "remove_bf_comments" => Self::RemoveBfComments,
            "remove_haml_block" => Self::RemoveHamlBlock,
            "remove_pug_block" => Self::RemovePugBlock,
            "remove_slim_block" => Self::RemoveSlimBlock,
            "reduce_to_rmd_code_blocks" => Self::ReduceToRmdCodeBlocks,
            "docstring_to_C" => Self::DocstringToC,
            "docstring_rm_comments" => Self::DocstringRmComments,
            "elixir_doc_to_C" => Self::ElixirDocToC,
            "Forth_paren_to_C" => Self::ForthParenToC,
            "powershell_to_C" => Self::PowershellToC,
            "smarty_to_C" => Self::SmartyToC,
            "jupyter_nb" => Self::JupyterNb,
            "call_parse_civet" => Self::CallParseCivet,
            "add_newlines" => Self::AddNewlines,
            "pre_post_fix" => Self::PrePostFix {
                prefix: arg(0)?,
                postfix: arg(1)?,
            },
            "rm_last_line" => Self::RmLastLine,
            "die" => Self::Die { message: opt(0) },
            other => return Err(err(format!("unknown filter {other:?}"))),
        })
    }
}
