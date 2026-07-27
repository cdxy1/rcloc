//! The comment-stripping filter chain.
//!
//! Lines are held **without** their trailing newline. The Perl original keeps
//! newlines attached, but its filters constantly round-trip through
//! `split("\n", ...)` and `join("", ...)`, so the two representations agree as
//! long as the multi-line filters re-insert separators when they join — which
//! they do here. Chomping also sidesteps a dialect difference: Perl's `$`
//! matches before a trailing newline, Rust's does not.
//!
//! Fidelity notes that look like bugs but are load-bearing:
//!
//! * `remove_inline` does nothing without `--inline`, and
//!   `rm_comments_in_strings` does nothing without `--strip-str-comments`.
//!   Together that is over a third of all filter steps in the table, skipped
//!   by default.
//! * `remove_matches` and `remove_inline` match case-insensitively;
//!   `remove_above`, `remove_below` and `remove_below_above` do not.
//! * In the `remove_between_*` family, Perl's `$1` survives a *failed* match,
//!   so a line with no start marker is judged against the capture from some
//!   earlier line. [`LastGroup1`] reproduces that.

use crate::dialects;
use crate::regex_cache;
use anyhow::Result;
use cloc_lang::{CommentDialect, Filter};
use fancy_regex::Regex;
use std::path::Path;

/// Run-time switches that change what the filters do.
#[derive(Debug, Clone, Default)]
pub struct FilterOptions {
    /// `--strip-str-comments`: enable `rm_comments_in_strings`.
    pub strip_str_comments: bool,
    /// `--inline`: enable `remove_inline`.
    pub inline: bool,
    /// `--docstring-as-code`: leave Python docstrings alone.
    pub docstring_as_code: bool,
    /// `--no-autogen`: drop PlusCal-generated TLA+ code.
    pub no_autogen: bool,
}

/// Everything a filter may need beyond the lines themselves.
pub struct FilterContext<'a> {
    pub options: &'a FilterOptions,
    pub file: &'a Path,
    pub language: &'a str,
    /// The language's end-of-line continuation regex, if it has one.
    pub eol_continuation: Option<&'a str>,
}

/// Apply a language's whole filter chain.
///
/// After each step the original removes blank lines again, because stripping
/// a comment usually leaves one behind; that repeat is what makes the final
/// `total - blank - remaining` arithmetic come out right.
pub fn apply_chain(
    mut lines: Vec<String>,
    filters: &[Filter],
    ctx: &FilterContext<'_>,
) -> Result<Vec<String>> {
    if lines.is_empty() {
        return Ok(lines);
    }
    let original_first = lines[0].clone();

    for filter in filters {
        if matches!(filter, Filter::RmCommentsInStrings { .. }) && !ctx.options.strip_str_comments {
            continue;
        }
        lines = apply_one(lines, filter, ctx)?;
        lines = remove_blank_lines(lines, ctx.eol_continuation)?;
    }

    // Scripting languages count their `#!` line as code. If a filter ate it,
    // put it back.
    if original_first.starts_with("#!")
        && lines.first().map_or(true, |l| *l != original_first)
        && cloc_lang::LangDb::default_db().is_script_language(ctx.language)
    {
        lines.insert(0, original_first);
    }

    Ok(lines)
}

fn apply_one(lines: Vec<String>, filter: &Filter, ctx: &FilterContext<'_>) -> Result<Vec<String>> {
    Ok(match filter {
        Filter::RemoveMatches { re } => {
            let re = regex_cache::cached(&caseless(re))?;
            retain_unmatched(lines, re)?
        }
        Filter::RemoveInline { re } => {
            if !ctx.options.inline {
                lines
            } else {
                let re = regex_cache::cached(&caseless(re))?;
                lines
                    .into_iter()
                    .map(|l| re.replace(&l, "").into_owned())
                    .collect()
            }
        }
        Filter::RemoveAbove { re } => {
            let re = regex_cache::cached(re)?;
            match first_match(&lines, re)? {
                // The marker is kept; only what precedes it goes.
                Some(idx) => lines.into_iter().skip(idx).collect(),
                None => lines,
            }
        }
        Filter::RemoveBelow { re } => {
            let re = regex_cache::cached(re)?;
            match first_match(&lines, re)? {
                Some(idx) => lines.into_iter().take(idx).collect(),
                None => lines,
            }
        }
        Filter::RemoveBelowAbove { below, above } => {
            let (start, end) = (regex_cache::cached(below)?, regex_cache::cached(above)?);
            let mut out = Vec::with_capacity(lines.len());
            let mut between = false;
            for line in lines {
                if !between && start.is_match(&line)? {
                    between = true;
                    continue;
                }
                if between && end.is_match(&line)? {
                    between = false;
                    continue;
                }
                if !between {
                    out.push(line);
                }
            }
            out
        }
        Filter::RmCommentsInStrings {
            string_marker,
            start_comment,
            end_comment,
            multiline,
        } => rm_comments_in_strings(
            lines,
            string_marker,
            start_comment,
            opt_str(end_comment),
            *multiline,
        )?,
        Filter::RemoveBetweenGeneral { start, end } => {
            remove_between(lines, &Marker::literal(start), &Marker::literal(end))?
        }
        Filter::RemoveBetweenRegex { start, end } => remove_between(
            lines,
            &Marker::regex(regex_cache::cached(start)?),
            &Marker::regex(regex_cache::cached(end)?),
        )?,
        Filter::ReplaceBetweenRegex {
            start,
            end,
            replacement,
            multiline,
        } => replace_between_regex(
            lines,
            regex_cache::cached(start)?,
            regex_cache::cached(end)?,
            replacement,
            *multiline,
        )?,
        Filter::ReplaceRegex { re, replacement } => {
            let re = regex_cache::cached(re)?;
            lines
                .into_iter()
                .filter(|l| !is_blank(l))
                .map(|l| re.replace_all(&l, replacement.as_str()).into_owned())
                .filter(|l| !is_blank(l))
                .collect()
        }
        Filter::RemoveHtmlComments => remove_html_comments(lines)?,
        Filter::CallRegexpCommon { dialect } => call_regexp_common(lines, *dialect),
        Filter::RemoveF77Comments => lines
            .into_iter()
            .filter(|l| !(starts_with_any(l, &['*', 'c', 'C']) || trimmed(l).starts_with('!')))
            .collect(),
        Filter::RemoveF90Comments => remove_f90_comments(lines)?,
        Filter::RemoveCobolComments => remove_cobol_comments(lines),
        Filter::RemoveJclComments => remove_jcl_comments(lines),
        Filter::RemoveJspComments => remove_jsp_comments(lines)?,
        Filter::RemoveOCamlComments => remove_ocaml_comments(lines)?,
        Filter::RemoveHaskellComments { .. } => remove_haskell_comments(lines, ctx.file)?,
        Filter::RemoveTlaPlusComments => remove_tlaplus_comments(lines)?,
        Filter::RemoveTlaPlusGeneratedCode => {
            if !ctx.options.no_autogen {
                lines
            } else {
                remove_between(
                    lines,
                    &Marker::regex(regex_cache::cached(r"^\\\* BEGIN TRANSLATION\b")?),
                    &Marker::regex(regex_cache::cached(r"^\\\* END TRANSLATION\b")?),
                )?
            }
        }
        Filter::RemoveBfComments => lines
            .into_iter()
            .map(|l| {
                l.chars()
                    .filter(|c| matches!(c, '<' | '>' | '+' | '-' | '.' | ',' | '[' | ']'))
                    .collect::<String>()
            })
            .filter(|l| !is_blank(l))
            .collect(),
        Filter::RemoveHamlBlock => remove_indented_block(lines, r"^(\s*)(/|-#)\s*$")?,
        Filter::RemovePugBlock => remove_indented_block(lines, r"^(\s*)(//)\s*$")?,
        Filter::RemoveSlimBlock => remove_indented_block(lines, r"^(\s*)(/[^!])")?,
        Filter::ReduceToRmdCodeBlocks => reduce_to_rmd_code_blocks(lines)?,
        Filter::DocstringToC => {
            if ctx.options.docstring_as_code {
                lines
            } else {
                docstring_to_c(lines)
            }
        }
        Filter::DocstringRmComments => docstring_rm_comments(lines)?,
        Filter::ElixirDocToC => elixir_doc_to_c(lines)?,
        Filter::ForthParenToC => forth_paren_to_c(lines)?,
        Filter::PowershellToC => lines
            .into_iter()
            .map(|l| l.replace("<#", "/*").replace("#>", "*/"))
            .collect(),
        Filter::SmartyToC => lines
            .into_iter()
            .map(|l| l.replace("{*", "/*").replace("*}", "*/"))
            .collect(),
        Filter::JupyterNb => jupyter_nb(lines),
        Filter::CallParseCivet => call_parse_civet(lines)?,
        // With newline-free lines, re-attaching newlines is a no-op: the
        // multi-line filters insert their own separators when joining.
        Filter::AddNewlines => lines,
        Filter::PrePostFix { prefix, postfix } => {
            let mut out = Vec::with_capacity(lines.len() + 2);
            out.push(prefix.clone());
            out.extend(lines);
            out.push(postfix.clone());
            out
        }
        Filter::RmLastLine => {
            let mut lines = lines;
            lines.pop();
            lines
        }
        Filter::Die { message } => {
            anyhow::bail!(
                "language {:?} is not directly countable: {}",
                ctx.language,
                if message.is_empty() {
                    "ambiguous extension should have been resolved first"
                } else {
                    message
                }
            )
        }
    })
}

// --- shared helpers ------------------------------------------------------ {{{1

fn opt_str(s: &str) -> Option<&str> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Wrap a pattern so it matches case-insensitively, as Perl's `/i` does.
fn caseless(re: &str) -> String {
    format!("(?i){re}")
}

fn is_blank(line: &str) -> bool {
    line.chars().all(char::is_whitespace)
}

fn trimmed(line: &str) -> &str {
    line.trim_start()
}

fn starts_with_any(line: &str, chars: &[char]) -> bool {
    line.chars().next().is_some_and(|c| chars.contains(&c))
}

fn retain_unmatched(lines: Vec<String>, re: &Regex) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        if !re.is_match(&line)? {
            out.push(line);
        }
    }
    Ok(out)
}

fn first_match(lines: &[String], re: &Regex) -> Result<Option<usize>> {
    for (i, line) in lines.iter().enumerate() {
        if re.is_match(line)? {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

/// Drop blank lines, honouring a language's line-continuation marker: a blank
/// line whose predecessor ends in a continuation is part of that statement.
pub fn remove_blank_lines(lines: Vec<String>, continuation: Option<&str>) -> Result<Vec<String>> {
    let Some(cont) = continuation else {
        return Ok(lines.into_iter().filter(|l| !is_blank(l)).collect());
    };
    let cont_re = regex_cache::cached(&caseless(cont))?;
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let drop = if i == 0 {
            is_blank(line)
        } else {
            is_blank(line) && !cont_re.is_match(&lines[i - 1])?
        };
        if !drop {
            out.push(line.clone());
        }
    }
    Ok(out)
}

/// Tab expansion to 8-column stops, matching Perl's `Text::Tabs::expand`.
fn expand_tabs(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut col = 0;
    for ch in line.chars() {
        if ch == '\t' {
            let width = 8 - (col % 8);
            out.extend(std::iter::repeat(' ').take(width));
            col += width;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

/// What a filter loop does with a line, and — just as importantly — how it
/// got there.
///
/// Perl restores the capture variables when `next` unwinds the loop body, but
/// not when the body simply runs to its end. So the way an iteration finishes
/// decides whether `$1` is still visible on the following line, and the three
/// endings have to be distinguished.
enum Step {
    /// `next` — drop the line.
    Skip,
    /// `push ...; next;` — keep the line, but `$1` does not survive.
    KeepAndNext(String),
    /// The push at the bottom of the body: `$1` carries to the next line.
    Keep(String),
}

/// Perl's `$1`, with the update rules several filters depend on without
/// saying so.
///
/// * A **successful** match resets every capture variable, so a pattern with
///   no groups leaves `$1` undefined.
/// * A **failed** match leaves it untouched, so `$1` can outlive the match
///   that set it and be read on a later line.
/// * `next` restores it to what it was before the loop body was entered,
///   which is to say undefined.
///
/// None of this is academic. Get the second rule wrong and the line after
/// every Lua block comment vanishes from the code count; get the third wrong
/// and a comment opened mid-line makes every following line of the comment
/// report the code that preceded it.
#[derive(Default)]
struct LastGroup1(Option<String>);

impl LastGroup1 {
    /// A successful match whose pattern captured group 1.
    fn set(&mut self, value: &str) {
        self.0 = Some(value.to_string());
    }
    /// A successful match whose pattern had no capture groups.
    fn clear(&mut self) {
        self.0 = None;
    }
    /// Apply the rule for a group-less pattern: clear iff it matched.
    fn after_groupless(&mut self, matched: bool) {
        if matched {
            self.clear();
        }
    }
    /// `defined $1 and $1 =~ /^\s*$/`
    fn is_defined_and_blank(&self) -> bool {
        self.0.as_deref().is_some_and(is_blank)
    }
    fn get(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

// --- the between family -------------------------------------------------- {{{1

/// A comment delimiter, either a literal string or a compiled regex.
enum Marker<'a> {
    Literal(&'a str),
    Regex(&'a Regex),
}

impl<'a> Marker<'a> {
    fn literal(s: &'a str) -> Self {
        Marker::Literal(s)
    }
    fn regex(r: &'a Regex) -> Self {
        Marker::Regex(r)
    }

    /// Byte range of the first occurrence at or after `from`.
    fn find_at(&self, hay: &str, from: usize) -> Result<Option<(usize, usize)>> {
        Ok(match self {
            Marker::Literal(lit) => hay[from..]
                .find(*lit)
                .map(|off| (from + off, from + off + lit.len())),
            Marker::Regex(re) => re.find_from_pos(hay, from)?.map(|m| (m.start(), m.end())),
        })
    }

    fn find(&self, hay: &str) -> Result<Option<(usize, usize)>> {
        self.find_at(hay, 0)
    }
}

/// `remove_between_general` and `remove_between_regex` — identical logic,
/// differing only in how the markers are matched.
fn remove_between(lines: Vec<String>, start: &Marker<'_>, end: &Marker<'_>) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(lines.len());
    let mut in_comment = false;
    let mut g1 = LastGroup1::default();

    for line in lines {
        let step = remove_between_line(line, start, end, &mut in_comment, &mut g1)?;
        finish(step, &mut out, &mut g1);
    }
    Ok(out)
}

/// One iteration of the loop above, so that how it ends is explicit.
fn remove_between_line(
    line: String,
    start: &Marker<'_>,
    end: &Marker<'_>,
    in_comment: &mut bool,
    g1: &mut LastGroup1,
) -> Result<Step> {
    if is_blank(&line) {
        return Ok(Step::Skip);
    }
    // `s/start.*?end//g` — group-less, so it clears $1 if it fired.
    let (mut line, stripped) = strip_inline_pairs(&line, start, end)?;
    g1.after_groupless(stripped);
    if is_blank(&line) {
        return Ok(Step::Skip);
    }

    if *in_comment {
        // `if (/end/)` then `s/^.*?end//` — both group-less.
        if let Some((_, e)) = end.find(&line)? {
            g1.clear();
            line = line[e..].to_string();
            *in_comment = false;
        }
        if *in_comment {
            return Ok(Step::Skip);
        }
    }
    if is_blank(&line) {
        return Ok(Step::Skip);
    }

    // `$in_comment = 1 if /^(.*?)start/` — the capture is the text before the
    // marker, and it survives if this fails to match.
    if let Some((s, _)) = start.find(&line)? {
        *in_comment = true;
        g1.set(&line[..s]);
    }
    if g1.is_defined_and_blank() {
        return Ok(Step::Skip);
    }
    if *in_comment {
        if let Some((s, _)) = start.find(&line)? {
            let kept = line[..s].to_string();
            g1.set(&kept);
            line = kept;
        }
    }
    Ok(Step::Keep(line))
}

/// Apply a [`Step`], honouring Perl's rule that `next` discards `$1`.
fn finish(step: Step, out: &mut Vec<String>, g1: &mut LastGroup1) {
    match step {
        Step::Skip => g1.clear(),
        Step::KeepAndNext(line) => {
            out.push(line);
            g1.clear();
        }
        Step::Keep(line) => out.push(line),
    }
}

/// `s/start.*?end//g`: remove every start..end pair that both opens and
/// closes within this line. Reports whether anything was removed, since the
/// caller must mirror Perl's capture-variable reset.
fn strip_inline_pairs(
    line: &str,
    start: &Marker<'_>,
    end: &Marker<'_>,
) -> Result<(String, bool)> {
    let mut out = String::with_capacity(line.len());
    let mut pos = 0;
    let mut removed = false;
    loop {
        let Some((s, s_end)) = start.find_at(line, pos)? else {
            break;
        };
        let Some((_, e_end)) = end.find_at(line, s_end)? else {
            break;
        };
        out.push_str(&line[pos..s]);
        pos = e_end;
        removed = true;
    }
    out.push_str(&line[pos..]);
    Ok((out, removed))
}

/// `replace_between_regex`: like [`remove_between`], but the removed span is
/// replaced rather than deleted, and `multiline` gates the carry-over state.
fn replace_between_regex(
    lines: Vec<String>,
    start: &Regex,
    end: &Regex,
    replacement: &str,
    multiline: bool,
) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(lines.len());
    let mut in_comment = false;
    let mut g1 = LastGroup1::default();

    for line in lines {
        let step = replace_between_line(
            line,
            start,
            end,
            replacement,
            multiline,
            &mut in_comment,
            &mut g1,
        )?;
        finish(step, &mut out, &mut g1);
    }
    Ok(out)
}

fn replace_between_line(
    line: String,
    start: &Regex,
    end: &Regex,
    replacement: &str,
    multiline: bool,
    in_comment: &mut bool,
    g1: &mut LastGroup1,
) -> Result<Step> {
    if is_blank(&line) {
        return Ok(Step::Skip);
    }
    let (mut line, replaced) = replace_inline_pairs(&line, start, end, replacement)?;
    g1.after_groupless(replaced);
    if is_blank(&line) {
        return Ok(Step::Skip);
    }

    if *in_comment {
        if let Some(m) = end.find(&line)? {
            let caps = end.captures(&line)?;
            let repl = caps
                .as_ref()
                .map(|c| eval_replacement(replacement, c))
                .unwrap_or_else(|| unquote(replacement).to_string());
            // The end pattern may itself capture; either way this match
            // succeeded, so $1 takes its value from this pattern.
            match caps.as_ref().and_then(|c| c.get(1)) {
                Some(g) => g1.set(g.as_str()),
                None => g1.clear(),
            }
            line = format!("{repl}{}", &line[m.end()..]);
            *in_comment = false;
        }
        if *in_comment {
            return Ok(Step::Skip);
        }
    }
    if is_blank(&line) {
        return Ok(Step::Skip);
    }

    if multiline {
        if let Some(m) = start.find(&line)? {
            *in_comment = true;
            g1.set(&line[..m.start()]);
        }
    }
    if g1.is_defined_and_blank() {
        return Ok(Step::Skip);
    }
    if *in_comment {
        if let Some(m) = start.find(&line)? {
            let kept = line[..m.start()].to_string();
            g1.set(&kept);
            line = kept;
        }
    }
    Ok(Step::Keep(line))
}

fn replace_inline_pairs(
    line: &str,
    start: &Regex,
    end: &Regex,
    replacement: &str,
) -> Result<(String, bool)> {
    let mut out = String::with_capacity(line.len());
    let mut pos = 0;
    let mut replaced = false;
    loop {
        let Some(s) = start.find_from_pos(line, pos)? else {
            break;
        };
        let Some(e) = end.find_from_pos(line, s.end())? else {
            break;
        };
        out.push_str(&line[pos..s.start()]);
        let caps = start.captures(&line[s.start()..])?;
        out.push_str(&match caps {
            Some(c) => eval_replacement(replacement, &c),
            None => unquote(replacement).to_string(),
        });
        pos = e.end();
        replaced = true;
    }
    out.push_str(&line[pos..]);
    Ok((out, replaced))
}

/// Evaluate a `s///ee` replacement.
///
/// The table stores these as quoted Perl expressions: `"xx"` is the literal
/// `xx`, while `"$1$2$1 + $1$3$1$4"` interpolates capture groups.
fn eval_replacement(template: &str, caps: &fancy_regex::Captures<'_>) -> String {
    let body = unquote(template);
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        let mut num = String::new();
        while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
            num.push(*d);
            chars.next();
        }
        match num.parse::<usize>() {
            Ok(n) => out.push_str(caps.get(n).map_or("", |m| m.as_str())),
            Err(_) => out.push('$'),
        }
    }
    out
}

/// Strip the surrounding double quotes of a Perl expression literal.
fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

// --- HTML and JSP -------------------------------------------------------- {{{1

/// `<!-- ... -->`, including the original's habit of emitting the code part
/// of a mixed line as its own entry.
fn remove_html_comments(lines: Vec<String>) -> Result<Vec<String>> {
    let (open, close) = (Marker::literal("<!--"), Marker::literal("-->"));
    let mut out = Vec::with_capacity(lines.len());
    let mut in_comment = false;
    let mut g1 = LastGroup1::default();

    for line in lines {
        let step = html_comment_line(line, &open, &close, &mut in_comment, &mut g1)?;
        finish(step, &mut out, &mut g1);
    }
    Ok(out)
}

fn html_comment_line(
    line: String,
    open: &Marker<'_>,
    close: &Marker<'_>,
    in_comment: &mut bool,
    g1: &mut LastGroup1,
) -> Result<Step> {
    if is_blank(&line) {
        return Ok(Step::Skip);
    }
    let (mut line, stripped) = strip_inline_pairs(&line, open, close)?;
    g1.after_groupless(stripped);
    if is_blank(&line) {
        return Ok(Step::Skip);
    }
    if *in_comment {
        if let Some((_, e)) = close.find(&line)? {
            g1.clear();
            line = line[e..].to_string();
            *in_comment = false;
        }
        // Unlike the other members of this family there is no early `next`
        // here; the flag is re-tested further down.
    }
    if is_blank(&line) {
        return Ok(Step::Skip);
    }
    if let Some((s, _)) = open.find(&line)? {
        *in_comment = true;
        g1.set(&line[..s]);
    }
    // A line that is part code, part comment yields its code half as a line
    // of its own -- and, ending in `next`, does not pass $1 along.
    if let Some(kept) = g1.get() {
        if !is_blank(kept) {
            return Ok(Step::KeepAndNext(kept.to_string()));
        }
    }
    if *in_comment {
        return Ok(Step::Skip);
    }
    Ok(Step::Keep(line))
}

/// `<%-- ... --%>`
fn remove_jsp_comments(lines: Vec<String>) -> Result<Vec<String>> {
    let (open, close) = (Marker::literal("<%--"), Marker::literal("--%>"));
    let mut out = Vec::with_capacity(lines.len());
    let mut in_comment = false;
    let mut g1 = LastGroup1::default();

    for line in lines {
        let step = jsp_comment_line(line, &open, &close, &mut in_comment, &mut g1)?;
        finish(step, &mut out, &mut g1);
    }
    Ok(out)
}

fn jsp_comment_line(
    line: String,
    open: &Marker<'_>,
    close: &Marker<'_>,
    in_comment: &mut bool,
    g1: &mut LastGroup1,
) -> Result<Step> {
    if is_blank(&line) {
        return Ok(Step::Skip);
    }
    let (mut line, stripped) = strip_inline_pairs(&line, open, close)?;
    g1.after_groupless(stripped);
    if is_blank(&line) {
        return Ok(Step::Skip);
    }
    if *in_comment {
        if let Some((_, e)) = close.find(&line)? {
            g1.clear();
            line = line[e..].to_string();
            *in_comment = false;
        }
    }
    if is_blank(&line) {
        return Ok(Step::Skip);
    }
    if let Some((s, _)) = open.find(&line)? {
        *in_comment = true;
        g1.set(&line[..s]);
    }
    if g1.is_defined_and_blank() {
        return Ok(Step::Skip);
    }
    if *in_comment {
        return Ok(Step::Skip);
    }
    Ok(Step::Keep(line))
}

// --- Regexp::Common bridge ----------------------------------------------- {{{1

/// Join the lines, strip comments of `dialect`, and split again.
///
/// Every line keeps its own separator. The original appears to special-case
/// C++ here, but at that point its lines still carry their newlines: a
/// continued line is appended as-is (one newline) while an ordinary line gets
/// a second one, and the resulting blank lines are swept up by the blank pass
/// that follows every filter. So no lines are ever merged, and joining
/// uniformly is equivalent. Backslash continuation of a `//` comment is the
/// scanner's business, not the joiner's.
fn call_regexp_common(lines: Vec<String>, dialect: CommentDialect) -> Vec<String> {
    let mut text = lines.join("\n");
    text.push('\n');

    dialects::strip_comments(&text, dialect)
        .split('\n')
        .map(str::to_string)
        .collect()
}

// --- language-specific filters ------------------------------------------- {{{1

fn remove_f90_comments(lines: Vec<String>) -> Result<Vec<String>> {
    // HPF and OpenMP directives look like comments but are code.
    let directive = regex_cache::cached(r"(?i)^\s*!(hpf|omp)\$")?;
    let comment = regex_cache::cached(r"^(\s*!|\s*$)")?;
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        if !comment.is_match(&line)? || directive.is_match(&line)? {
            out.push(line);
        }
    }
    Ok(out)
}

fn remove_cobol_comments(lines: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut free_format = false;
    for line in lines {
        if is_source_format_free(&line) {
            free_format = true;
        }
        let is_comment = if free_format {
            trimmed(&line).starts_with('*')
        } else {
            // Fixed format: an asterisk in the indicator area (column 7).
            line.starts_with('*') || line.chars().nth(6) == Some('*')
        };
        if !is_comment {
            out.push(line);
        }
    }
    out
}

/// `$ SET SOURCEFORMAT FREE` in the indicator area switches COBOL to free
/// format for the rest of the file.
fn is_source_format_free(line: &str) -> bool {
    let upper = line.to_uppercase();
    upper.len() > 6
        && upper.as_bytes().get(6) == Some(&b'$')
        && upper.contains("SET")
        && upper.contains("SOURCEFORMAT")
        && upper.contains("FREE")
}

/// COBOL blank-line rules: a line with only a sequence number, or blank
/// through column 71, or a page-eject directive, counts as blank.
pub fn remove_cobol_blanks(lines: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut free_format = false;
    for line in lines {
        if is_blank(&line) {
            continue;
        }
        let expanded = expand_tabs(&line);
        if is_source_format_free(&expanded) {
            free_format = true;
        }
        if free_format {
            out.push(line);
            continue;
        }
        let only_sequence_number = line.len() == 6 && line.chars().all(|c| c.is_ascii_digit())
            || (line.len() > 6
                && line[..6].chars().all(|c| c.is_ascii_digit())
                && line[6..].chars().all(char::is_whitespace));
        let blank_through_71 = expanded.len() >= 72
            && expanded[6..72].chars().all(|c| c == ' ');
        let page_eject = expanded.chars().nth(6) == Some('/');
        if !(only_sequence_number || blank_through_71 || page_eject) {
            out.push(line);
        }
    }
    out
}

fn remove_jcl_comments(lines: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        if is_blank(&line) {
            continue;
        }
        if line.starts_with("//*") {
            continue;
        }
        // A bare `//` terminates the job stream; everything after is data.
        if trimmed(&line).trim_end() == "//" {
            break;
        }
        out.push(line);
    }
    out
}

/// OCaml comments nest, and string literals shield markers inside them.
fn remove_ocaml_comments(lines: Vec<String>) -> Result<Vec<String>> {
    let tokenizer = regex_cache::cached(r#"(\(\*|\*\)|".*?")"#)?;
    let mut out = Vec::with_capacity(lines.len());
    let mut depth: i32 = 0;

    for line in lines {
        if is_blank(&line) {
            continue;
        }
        let mut clean = String::new();
        for token in split_keep_delimiters(&line, tokenizer)? {
            match token.as_str() {
                "(*" => depth += 1,
                "*)" => depth -= 1,
                _ if depth <= 0 => clean.push_str(&token),
                _ => {}
            }
        }
        if !clean.is_empty() {
            out.push(clean);
        }
    }
    Ok(out)
}

/// TLA+ is OCaml-like, except that PlusCal algorithms live *inside* comments
/// and must still be counted as code.
fn remove_tlaplus_comments(lines: Vec<String>) -> Result<Vec<String>> {
    let tokenizer =
        regex_cache::cached(r#"(\(\*|\*\)|".*?"|[{}]|--fair\b|--algorithm|PlusCal options)"#)?;
    let mut out = Vec::with_capacity(lines.len());
    let mut depth: i32 = 0;
    let mut in_pluscal_head = false;
    let mut pluscal_depth: i32 = 0;
    let mut saved_depth: i32 = 0;

    for line in lines {
        if is_blank(&line) {
            continue;
        }
        let mut clean = String::new();
        for token in split_keep_delimiters(&line, tokenizer)? {
            match token.as_str() {
                "(*" => depth += 1,
                "*)" => depth -= 1,
                "{" if in_pluscal_head => {
                    in_pluscal_head = false;
                    pluscal_depth = 1;
                    saved_depth = depth;
                    depth = 0;
                    clean.push_str(&token);
                }
                "{" if pluscal_depth > 0 && depth == 0 => {
                    pluscal_depth += 1;
                    clean.push_str(&token);
                }
                "}" if pluscal_depth > 0 && depth == 0 => {
                    pluscal_depth -= 1;
                    if pluscal_depth == 0 {
                        depth = saved_depth;
                    }
                    clean.push_str(&token);
                }
                "--fair" | "--algorithm" => {
                    in_pluscal_head = true;
                    clean.push_str(&token);
                }
                "PlusCal options" => clean.push_str(&token),
                _ if depth == 0 || in_pluscal_head || pluscal_depth > 0 => clean.push_str(&token),
                _ => {}
            }
        }
        if !clean.is_empty() {
            out.push(clean);
        }
    }
    Ok(out)
}

/// Perl's `split(/(...)/, $s)`: the delimiters are kept as their own pieces.
fn split_keep_delimiters(line: &str, re: &Regex) -> Result<Vec<String>> {
    let mut pieces = Vec::new();
    let mut pos = 0;
    while let Some(m) = re.find_from_pos(line, pos)? {
        if m.start() > pos {
            pieces.push(line[pos..m.start()].to_string());
        }
        pieces.push(m.as_str().to_string());
        // A zero-width match would spin forever.
        pos = if m.end() > m.start() {
            m.end()
        } else {
            m.end() + 1
        };
        if pos > line.len() {
            break;
        }
    }
    if pos < line.len() {
        pieces.push(line[pos..].to_string());
    }
    Ok(pieces)
}

/// Haskell/Elm/Idris: `--` line comments and `{- -}` blocks, the latter
/// nesting in Elm. Literate Haskell keeps only the code lines.
fn remove_haskell_comments(lines: Vec<String>, file: &Path) -> Result<Vec<String>> {
    let name = file.to_string_lossy();
    let is_elm = name.ends_with(".elm");
    let literate = if name.ends_with(".lhs") {
        determine_lit_type(&lines)
    } else {
        0
    };

    let mut out = Vec::with_capacity(lines.len());
    let mut in_comment: i32 = 0;
    let mut in_lit_block = false;

    for line in lines {
        let mut line = match literate {
            // Bird style: code lines begin with `>`.
            1 => match line.strip_prefix('>') {
                Some(rest) => rest.to_string(),
                None => String::new(),
            },
            // LaTeX style: code lives between \begin{code} and \end{code}.
            2 => {
                if in_lit_block {
                    if line.starts_with("\\end{code}") {
                        in_lit_block = false;
                        String::new()
                    } else {
                        line
                    }
                } else {
                    if line.starts_with("\\begin{code}") {
                        in_lit_block = true;
                    }
                    String::new()
                }
            }
            _ => line,
        };

        // Pragmas `{-# ... #-}` are code, not comments.
        if trimmed(&line).starts_with("{-#") {
            out.push(line);
            continue;
        }

        let n_open = line.matches("{-").count() as i32;
        let n_close = line.matches("-}").count() as i32;
        line = remove_all_nonoverlapping(&line, "{-", "-}");

        if in_comment > 0 {
            if let Some(idx) = line.find("-}") {
                line = line[idx + 2..].to_string();
                in_comment = if is_elm {
                    in_comment + n_open - n_close
                } else {
                    0
                };
            } else {
                line.clear();
            }
        } else {
            if let Some(idx) = line.find("--") {
                line.truncate(idx);
            }
            if line.contains("{-") && !line.contains("{-#") {
                if let Some(idx) = line.find("{-") {
                    line.truncate(idx);
                }
                in_comment = if is_elm { in_comment + n_open - n_close } else { 1 };
            }
        }

        if line.chars().any(|c| !c.is_whitespace()) {
            out.push(line);
        }
    }
    Ok(out)
}

/// Which flavour of literate Haskell a file uses: 2 for LaTeX-style,
/// 1 for Bird-style, 0 for neither.
fn determine_lit_type(lines: &[String]) -> u8 {
    for line in lines {
        if line.starts_with("\\begin{code}") {
            return 2;
        }
        if line.starts_with('>') && line[1..].starts_with(char::is_whitespace) {
            return 1;
        }
    }
    0
}

/// `s/start.*?end//g` for literal markers.
fn remove_all_nonoverlapping(line: &str, start: &str, end: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut pos = 0;
    while let Some(s) = line[pos..].find(start).map(|o| pos + o) {
        let Some(e) = line[s + start.len()..].find(end).map(|o| s + start.len() + o) else {
            break;
        };
        out.push_str(&line[pos..s]);
        pos = e + end.len();
    }
    out.push_str(&line[pos..]);
    out
}

/// Haml, Pug and Slim mark a comment with a lone sigil; everything indented
/// further than it belongs to the comment.
fn remove_indented_block(lines: Vec<String>, pattern: &str) -> Result<Vec<String>> {
    let re = regex_cache::cached(pattern)?;
    let mut out = Vec::with_capacity(lines.len());
    // 0 means "not in a comment"; otherwise the indent that ends it.
    let mut in_comment: usize = 0;

    for line in lines {
        if is_blank(&line) {
            continue;
        }
        let expanded = expand_tabs(&line);
        if in_comment > 0 {
            let indent = expanded.len() - expanded.trim_start().len();
            if indent < in_comment {
                in_comment = 0;
            } else {
                continue;
            }
        } else if let Some(caps) = re.captures(&expanded)? {
            in_comment = caps.get(1).map_or(0, |m| m.as_str().len()) + 1;
            continue;
        }
        out.push(expanded);
    }
    Ok(out)
}

/// R Markdown: only fenced chunks like ```` ```{r} ```` hold code.
fn reduce_to_rmd_code_blocks(lines: Vec<String>) -> Result<Vec<String>> {
    let open = regex_cache::cached(r"^```\{\s*[[:alpha:]]")?;
    let close = regex_cache::cached(r"^```\s*$")?;
    let mut out = Vec::with_capacity(lines.len());
    let mut in_block = false;
    for line in lines {
        if open.is_match(&line)? {
            in_block = true;
            continue;
        }
        if close.is_match(&line)? {
            in_block = false;
        }
        if in_block {
            out.push(line);
        }
    }
    Ok(out)
}

/// Rewrite Python docstring delimiters as C comment markers so the C scanner
/// removes the body. Alternating delimiters open and close in turn.
fn docstring_to_c(lines: Vec<String>) -> Vec<String> {
    let mut in_docstring = false;
    lines
        .into_iter()
        .map(|line| {
            let mut result = String::with_capacity(line.len());
            let mut rest = line.as_str();
            while let Some(idx) = find_docstring_delim(rest) {
                let (before, at) = rest.split_at(idx);
                if in_docstring {
                    result.push_str(before);
                    result.push_str("*/");
                } else {
                    // A `u`/`U` string prefix is part of the delimiter.
                    let trimmed_len = before
                        .strip_suffix(['u', 'U'])
                        .map_or(before.len(), str::len);
                    result.push_str(&before[..trimmed_len]);
                    result.push_str("/*");
                }
                in_docstring = !in_docstring;
                rest = &at[3..];
            }
            result.push_str(rest);
            result
        })
        .collect()
}

fn find_docstring_delim(s: &str) -> Option<usize> {
    let triple_double = s.find("\"\"\"");
    let triple_single = s.find("'''");
    match (triple_double, triple_single) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Neutralise C comment markers that appear *inside* docstrings, so the C
/// scanner does not treat them as real comment boundaries.
fn docstring_rm_comments(lines: Vec<String>) -> Result<Vec<String>> {
    let mut in_docstring = false;
    let mut out = Vec::with_capacity(lines.len());

    for line in lines {
        let delim = find_docstring_delim(&line);
        let new_line = match delim {
            Some(start) if closes_on_same_line(&line, start) => {
                // Whole docstring on one line: scrub only its body.
                let body_start = start + 3;
                let body_end = line[body_start..]
                    .find(&line[start..start + 3])
                    .map(|o| body_start + o)
                    .unwrap_or(line.len());
                format!(
                    "{}{}{}",
                    &line[..body_start],
                    scrub_c_markers(&line[body_start..body_end]),
                    &line[body_end..]
                )
            }
            Some(start) if !in_docstring => {
                in_docstring = true;
                format!(
                    "{}{}",
                    &line[..start + 3],
                    scrub_c_markers(&line[start + 3..])
                )
            }
            Some(start) => {
                in_docstring = false;
                format!("{}{}", scrub_c_markers(&line[..start]), &line[start..])
            }
            None if in_docstring => scrub_c_markers(&line),
            None => line,
        };
        out.push(new_line);
    }
    Ok(out)
}

fn closes_on_same_line(line: &str, start: usize) -> bool {
    let delim = &line[start..start + 3];
    line[start + 3..].contains(delim)
}

fn scrub_c_markers(s: &str) -> String {
    s.replace("/*", "xx").replace("*/", "xx").replace("//", "xx")
}

/// Elixir `@doc """ ... """` heredocs become C comments.
fn elixir_doc_to_c(lines: Vec<String>) -> Result<Vec<String>> {
    let open = regex_cache::cached(r#"(@(module)?doc\s+(~[sScC])?['"]{3})"#)?;
    let close = regex_cache::cached(r#"(['"]{3})"#)?;
    let mut in_docstring = false;
    let mut out = Vec::with_capacity(lines.len());

    for line in lines {
        let new_line = if !in_docstring {
            match open.find(&line)? {
                Some(m) => {
                    in_docstring = true;
                    format!("{}/*{}", &line[..m.start()], &line[m.end()..])
                }
                None => line,
            }
        } else {
            match close.find(&line)? {
                Some(m) => {
                    in_docstring = false;
                    format!("{}*/{}", &line[..m.start()], &line[m.end()..])
                }
                None => line,
            }
        };
        out.push(new_line);
    }
    Ok(out)
}

/// Forth `( ... )` comments become C comments. The delimiters must be
/// surrounded by whitespace, which is what keeps `(` in code from matching.
fn forth_paren_to_c(lines: Vec<String>) -> Result<Vec<String>> {
    let inline = regex_cache::cached(r"\s+\(\s+.*?\)")?;
    let open = regex_cache::cached(r"\s+\(\s+")?;
    let mut in_comment = false;
    let mut out = Vec::with_capacity(lines.len());

    for line in lines {
        let mut line = line;
        // Bound the rewriting: a line with an unmatched `(` would otherwise
        // loop forever, which is why the original caps the iterations too.
        for _ in 0..256 {
            if inline.is_match(&line)? {
                line = inline.replace_all(&line, "").into_owned();
                continue;
            }
            if !in_comment {
                match open.find(&line)? {
                    Some(m) => {
                        line = format!("{}/*{}", &line[..m.start()], &line[m.end()..]);
                        in_comment = true;
                    }
                    None => break,
                }
            } else {
                match line.find(')') {
                    Some(idx) => {
                        line = format!("{}*/{}", &line[..idx], &line[idx + 1..]);
                        in_comment = false;
                    }
                    None => break,
                }
            }
        }
        out.push(line);
    }
    Ok(out)
}

/// Pull the source lines out of a Jupyter notebook's JSON, skipping the
/// markdown cells, blank source entries and `#` comment lines.
fn jupyter_nb(lines: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_code = false;
    let mut in_source = false;

    for line in lines {
        let t = line.trim();
        if !in_code && !in_source && t == "\"cell_type\": \"code\"," {
            in_code = true;
        } else if in_code && !in_source && t == "\"source\": [" {
            in_source = true;
        } else if in_code && in_source {
            if t == "\"\\n\"," || t.starts_with("\"\\n\"") && t.ends_with(',') {
                continue;
            }
            if t.starts_with("\"#") || t.starts_with("\" #") {
                continue;
            }
            if t == "]" {
                in_code = false;
                in_source = false;
            } else {
                out.push(line);
            }
        }
    }
    out
}

/// Civet has two comment syntaxes; a `civet coffeeCompat` pragma anywhere in
/// the file selects the CoffeeScript one.
fn call_parse_civet(lines: Vec<String>) -> Result<Vec<String>> {
    let pragma = regex_cache::cached(r"^\s*civet\s+coffee(Comment|Compat)")?;
    let mut coffee = false;
    for line in &lines {
        if pragma.is_match(line)? {
            coffee = true;
            break;
        }
    }

    let hash = Marker::literal("###");
    if coffee {
        let step = remove_between(lines, &hash, &hash)?;
        retain_unmatched(step, regex_cache::cached(&caseless(r"^\s*#"))?)
    } else {
        let step = call_regexp_common(lines, CommentDialect::C);
        let step = retain_unmatched(step, regex_cache::cached(&caseless(r"^///"))?)?;
        let step = retain_unmatched(step, regex_cache::cached(&caseless(r"^\s*//[^/]"))?)?;
        remove_between(step, &hash, &hash)
    }
}

/// Replace comment markers inside string literals with `xx`, so the comment
/// scanners that follow do not mistake them for comment boundaries. Off
/// unless `--strip-str-comments`.
fn rm_comments_in_strings(
    lines: Vec<String>,
    string_marker: &str,
    start_comment: &str,
    end_comment: Option<&str>,
    multiline: bool,
) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(lines.len());
    let mut in_ml_string = false;

    let scrub = |s: &str| -> String {
        let mut r = s.replace(start_comment, "xx");
        if let Some(end) = end_comment {
            r = r.replace(end, "xx");
        }
        r
    };

    for line in lines {
        if !line.contains(string_marker) {
            out.push(if in_ml_string { scrub(&line) } else { line });
            continue;
        }

        // An escaped marker is not a delimiter.
        let escaped = format!("\\{string_marker}");
        let mut line = line.replace(&escaped, "Q");
        let mut new_line = String::new();

        if in_ml_string {
            if let Some(idx) = line.find(string_marker) {
                new_line.push_str(&scrub(&line[..idx]));
                new_line.push_str(string_marker);
                line = line[idx + string_marker.len()..].to_string();
                in_ml_string = false;
            }
        }

        let pieces = split_string_literals(&line, string_marker);
        for piece in pieces {
            if piece.starts_with(string_marker) && piece.ends_with(string_marker) && piece.len() > 1
            {
                new_line.push_str(&scrub(&piece));
            } else if multiline && piece.contains(string_marker) {
                // An unclosed quote opens a multi-line string.
                let idx = piece.find(string_marker).unwrap();
                new_line.push_str(&piece[..idx + string_marker.len()]);
                new_line.push_str(&scrub(&piece[idx + string_marker.len()..]));
                in_ml_string = true;
            } else {
                new_line.push_str(&piece);
            }
        }
        out.push(new_line);
    }
    Ok(out)
}

/// Split a line into alternating outside-string and inside-string pieces,
/// keeping the quotes on the latter.
fn split_string_literals(line: &str, marker: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut pos = 0;
    while let Some(open) = line[pos..].find(marker).map(|o| pos + o) {
        let body = open + marker.len();
        match line[body..].find(marker).map(|o| body + o) {
            Some(close) => {
                if open > pos {
                    pieces.push(line[pos..open].to_string());
                }
                pieces.push(line[open..close + marker.len()].to_string());
                pos = close + marker.len();
            }
            None => break,
        }
    }
    if pos < line.len() {
        pieces.push(line[pos..].to_string());
    }
    pieces
}

#[cfg(test)]
mod tests;
