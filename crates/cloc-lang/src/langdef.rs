//! The plain-text language definition format.
//!
//! This is what `--write-lang-def` produces and what `--read-lang-def` and
//! `--force-lang-def` consume. A language is an unindented name followed by
//! four-space-indented properties:
//!
//! ```text
//! Rust
//!     filter rm_comments_in_strings " /* */
//!     filter call_regexp_common C++
//!     extension rs
//!     3rd_gen_scale 1.00
//! ```
//!
//! Arguments are whitespace-separated and never quoted, which is a real
//! limit of the format rather than of this parser: a filter argument
//! containing a space cannot be expressed, and the original has the same
//! hole.

use crate::filter::RawFilter;
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;

/// One language's definition, as read from or written to the text format.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LangDef {
    pub filters: Vec<RawFilter>,
    pub extensions: Vec<String>,
    pub filenames: Vec<String>,
    pub script_exes: Vec<String>,
    pub scale_factor: Option<f64>,
    pub eol_continuation: Option<String>,
}

/// A whole definition file, keyed by language name.
pub type LangDefs = BTreeMap<String, LangDef>;

/// Parse a definition file.
pub fn parse(text: &str) -> Result<LangDefs> {
    let mut defs = LangDefs::new();
    let mut current: Option<String> = None;

    for (index, raw) in text.lines().enumerate() {
        let line_num = index + 1;
        if raw.trim().is_empty() || raw.trim_start().starts_with('#') {
            continue;
        }

        // An unindented line names a language; anything else is a property
        // of the language named most recently.
        if !raw.starts_with(char::is_whitespace) {
            current = Some(raw.trim().to_string());
            defs.entry(raw.trim().to_string()).or_default();
            continue;
        }

        let Some(language) = current.clone() else {
            bail!("missing computer language name, line {line_num}");
        };
        let entry = defs.entry(language).or_default();

        let mut words = raw.trim().split_whitespace();
        let Some(keyword) = words.next() else { continue };
        let rest: Vec<String> = words.map(str::to_string).collect();

        match keyword {
            "filter" => {
                let mut args = rest.into_iter();
                let Some(name) = args.next() else {
                    bail!("filter with no name, line {line_num}");
                };
                entry.filters.push(RawFilter {
                    filter: name,
                    args: args.collect(),
                });
            }
            "extension" => entry.extensions.extend(rest),
            "filename" => entry.filenames.extend(rest),
            "script_exe" => entry.script_exes.extend(rest),
            "3rd_gen_scale" => {
                let value = rest.first().cloned().unwrap_or_default();
                entry.scale_factor = Some(
                    value
                        .parse()
                        .with_context(|| format!("bad 3rd_gen_scale, line {line_num}"))?,
                );
            }
            "end_of_line_continuation" => {
                entry.eol_continuation = rest.first().cloned();
            }
            other => bail!("unexpected keyword {other:?}, line {line_num}"),
        }
    }

    Ok(defs)
}

/// Render definitions back to the text format.
pub fn write(defs: &LangDefs) -> String {
    let mut out = String::new();
    for (language, def) in defs {
        out.push_str(language);
        out.push('\n');
        for filter in &def.filters {
            out.push_str("    filter ");
            out.push_str(&filter.filter);
            for arg in &filter.args {
                out.push(' ');
                out.push_str(arg);
            }
            out.push('\n');
        }
        for ext in &def.extensions {
            out.push_str(&format!("    extension {ext}\n"));
        }
        for name in &def.filenames {
            out.push_str(&format!("    filename {name}\n"));
        }
        for exe in &def.script_exes {
            out.push_str(&format!("    script_exe {exe}\n"));
        }
        if let Some(scale) = def.scale_factor {
            out.push_str(&format!("    3rd_gen_scale {scale:.2}\n"));
        }
        if let Some(eol) = &def.eol_continuation {
            out.push_str(&format!("    end_of_line_continuation {eol}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_language() {
        let defs = parse(
            "Widget\n    filter remove_matches ^\\s*//\n    extension wdg\n    3rd_gen_scale 1.50\n",
        )
        .unwrap();
        let w = &defs["Widget"];
        assert_eq!(w.filters.len(), 1);
        assert_eq!(w.filters[0].filter, "remove_matches");
        assert_eq!(w.filters[0].args, vec![r"^\s*//"]);
        assert_eq!(w.extensions, vec!["wdg"]);
        assert_eq!(w.scale_factor, Some(1.5));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let defs = parse("# a note\n\nWidget\n    extension wdg\n").unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs["Widget"].extensions, vec!["wdg"]);
    }

    #[test]
    fn a_property_before_any_language_is_an_error() {
        assert!(parse("    extension wdg\n").is_err());
    }

    #[test]
    fn an_unknown_keyword_is_an_error() {
        let err = parse("Widget\n    frobnicate yes\n").unwrap_err();
        assert!(err.to_string().contains("frobnicate"));
    }

    /// Multi-argument filters keep their arguments in order.
    #[test]
    fn multi_argument_filters_round_trip() {
        let text = "Widget\n    filter remove_between_general <!-- -->\n    3rd_gen_scale 1.00\n";
        let defs = parse(text).unwrap();
        assert_eq!(defs["Widget"].filters[0].args, vec!["<!--", "-->"]);
        assert_eq!(write(&defs), text);
    }

    #[test]
    fn writing_then_parsing_gives_the_same_definitions() {
        let text = "Alpha\n    filter remove_matches ^;\n    extension a\n    \
                    filename Alphafile\n    script_exe alpha\n    3rd_gen_scale 2.00\n    \
                    end_of_line_continuation \\\\$\nBeta\n    extension b\n    \
                    3rd_gen_scale 1.00\n";
        let defs = parse(text).unwrap();
        assert_eq!(parse(&write(&defs)).unwrap(), defs);
    }
}
