//! Deciding what language a file is written in.
//!
//! The order of tests mirrors `classify_file()` in the original, because it is
//! load-bearing: a name in the file-type table wins over its extension, a
//! longer extension wins over a shorter one, and only a file whose extension
//! is unknown gets its first line inspected.
//!
//! Around twenty extensions are claimed by more than one language (`.m`,
//! `.pl`, `.ts`, ...). Those map to a pseudo-language such as `Perl/Prolog`,
//! which a resolver turns into a real one by scoring the file's contents.

use crate::regex_cache;
use anyhow::Result;
use cloc_lang::LangDb;
use std::collections::BTreeMap;
use std::path::Path;

/// The name cloc gives a file it cannot place.
pub const UNKNOWN: &str = "(unknown)";

/// Options that steer classification.
#[derive(Debug, Clone, Default)]
pub struct ClassifyOptions {
    /// `--autoconf`: treat `foo.c.in` as `foo.c`.
    pub autoconf: bool,
    /// `--ignore-case-ext`: fold extensions to lower case.
    pub ignore_case_ext: bool,
    /// `--lang-no-ext=LANG`: language for extensionless files.
    pub lang_no_ext: Option<String>,
    /// `--force-lang`: extensions the user insists on counting even when the
    /// not-code table would drop them.
    pub forced_extensions: Vec<String>,
    /// `--no-autogen`: skip files marked as machine-generated.
    pub no_autogen: bool,
}

/// The outcome of classifying one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classification {
    /// Count it as this language.
    Language(String),
    /// Skip it, with a reason for `--ignored`.
    Ignored { reason: String },
}

impl Classification {
    pub fn language(&self) -> Option<&str> {
        match self {
            Self::Language(l) => Some(l),
            Self::Ignored { .. } => None,
        }
    }
}

/// Decide the language of `path`, reading its contents only if needed.
pub fn classify(path: &Path, db: &LangDb, opts: &ClassifyOptions) -> Result<Classification> {
    let raw_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    // `--autoconf`: strip the `.in` that marks a template.
    let name = if opts.autoconf {
        raw_name.strip_suffix(".in").unwrap_or(&raw_name).to_string()
    } else {
        raw_name.clone()
    };

    if db.is_not_code_filename(&name) {
        return Ok(ignored(format!("listed in Not_Code_Filename: {name}")));
    }
    // Editor backups.
    if name.ends_with('~') {
        return Ok(ignored("temporary editor file".to_string()));
    }

    if let Some(lang) = db.language_for_file_name(&name) {
        return resolve(lang, path, db, opts);
    }

    if name.contains('.') {
        // Try the longest extension first: `foo.tar.gz` before `gz`.
        for ext in extension_candidates(&name, opts.ignore_case_ext) {
            if db.is_not_code_extension(&ext) && !opts.forced_extensions.contains(&ext) {
                return Ok(ignored(format!("listed in Not_Code_Extension: {ext}")));
            }
            if let Some(lang) = db.language_for_extension(&ext) {
                return resolve(lang, path, db, opts);
            }
        }
        // Fall back to the part before the extension, for `Dockerfile.dev`.
        if let Some(stem) = name.rsplit_once('.').map(|(s, _)| s) {
            if let Some(lang) = db.language_for_prefix(stem) {
                return Ok(Classification::Language(lang.to_string()));
            }
        }
    } else if let Some(lang) = db.language_for_file_name(&name.to_lowercase()) {
        return resolve(lang, path, db, opts);
    } else if let Some(forced) = &opts.lang_no_ext {
        if db.filters(forced).is_some() {
            return Ok(Classification::Language(forced.clone()));
        }
    }

    // Unrecognised or absent extension: look at the first line.
    peek_first_line(path, db)
}

fn ignored(reason: String) -> Classification {
    Classification::Ignored { reason }
}

/// Extensions to try, longest first: `a.b.c.d` yields `b.c.d`, `c.d`, `d`.
fn extension_candidates(name: &str, fold_case: bool) -> Vec<String> {
    let parts: Vec<&str> = name.split('.').collect();
    let mut out = Vec::new();
    // The name itself is not an extension, so start after the first segment.
    for take in (1..parts.len().min(4)).rev() {
        let ext = parts[parts.len() - take..].join(".");
        out.push(if fold_case { ext.to_lowercase() } else { ext });
    }
    out
}

/// A shebang, or an XML declaration, names the language when nothing else did.
fn peek_first_line(path: &Path, db: &LangDb) -> Result<Classification> {
    let Some(line) = crate::io::first_line(path)? else {
        return Ok(ignored("language unknown (empty file)".to_string()));
    };

    if let Some(interpreter) = shebang_interpreter(&line) {
        if let Some(lang) = db.language_for_script(&interpreter) {
            if db.filters(lang).is_some() {
                return Ok(Classification::Language(lang.to_string()));
            }
            return Ok(ignored(format!(
                "no filters defined for scripting language {interpreter}"
            )));
        }
    }

    // XML dialects use all sorts of extensions; the declaration is decisive.
    if line.contains("<?xml ") || line.contains("<?xml\t") {
        return Ok(Classification::Language("XML".to_string()));
    }

    Ok(ignored("language unknown".to_string()))
}

/// The interpreter named by a `#!` line, allowing for `/usr/bin/env`.
fn shebang_interpreter(line: &str) -> Option<String> {
    let rest = line.strip_prefix("#!")?;
    let mut words = rest.split_whitespace();
    let first = words.next()?;
    let basename = first.rsplit('/').next().unwrap_or(first);
    if basename == "env" {
        // `#!/usr/bin/env python3` — the interpreter is the next word, but
        // skip any VAR=value assignments env may carry.
        for word in words {
            if !word.contains('=') {
                return Some(word.rsplit('/').next().unwrap_or(word).to_string());
            }
        }
        return None;
    }
    Some(basename.to_string())
}

/// Turn a pseudo-language into a real one, or pass a real one through.
fn resolve(
    language: &str,
    path: &Path,
    db: &LangDb,
    opts: &ClassifyOptions,
) -> Result<Classification> {
    if !db.is_collision(language) && !NEEDS_CONTENT_CHECK.contains(&language) {
        return Ok(Classification::Language(language.to_string()));
    }

    let lines = match crate::io::read_lines(path) {
        Ok(l) => l,
        Err(_) => return Ok(ignored(format!("unable to read to resolve {language}"))),
    };

    Ok(match language {
        "MATLAB/Mathematica/Objective-C/MUMPS/Mercury" => lang(matlab_or_objective_c(&lines)?),
        "PHP/Pascal/Fortran/Pawn/BitBake/Assembly" => lang(php_pascal_fortran_pawn(&lines)?),
        "Pascal/Puppet" => lang(pascal_or_puppet(&lines)?),
        "Lisp/OpenCL" => lang(lisp_or_opencl(&lines)?),
        "Lisp/Julia" => lang(lisp_or_julia(&lines)?),
        "Perl/Prolog" => lang(perl_or_prolog(&lines)?),
        "Raku/Prolog" => lang(match perl_or_prolog(&lines)?.as_str() {
            "Perl" => "Raku".to_string(),
            other => other.to_string(),
        }),
        "IDL/Qt Project/Prolog/ProGuard" => lang(idl_or_qtproject(&lines)?),
        "Fortran 77/Forth" => lang(forth_or_fortran(&lines)?),
        "F#/Forth" => lang(forth_or_fsharp(&lines)?),
        "Verilog-SystemVerilog/Coq" => lang(verilog_or_coq(&lines)?),
        "TypeScript/Qt Linguist" => lang(typescript_or_qtlinguist(&lines)?),
        "XML-Qt-GTK/Glade" => lang(qt_or_glade(&lines)?),
        "C#/Smalltalk" => lang(csharp_or_smalltalk(&lines)?),
        "Visual Basic/TeX/Apex Class" => lang(visual_basic_or_tex_or_apex(&lines)?),
        "Scheme/SaltStack" => lang(scheme_or_saltstack(&lines)?),
        "Pascal/Pawn" => lang(pascal_or_pawn(&lines)?),
        "SKILL/.NET IL" => lang(skill_or_dotnet_il(&lines)?),
        "Clojure/Cangjie" => lang(clojure_or_cangjie(&lines)?),
        "Unknown/BitBake" => match really_is_bitbake(&lines)? {
            true => lang("BitBake".to_string()),
            false => ignored("not BitBake".to_string()),
        },
        "D/dtrace" => lang(match really_is_d(&lines)? {
            true => "D".to_string(),
            false => "dtrace".to_string(),
        }),
        "ADSO/IDSM" => lang("ADSO/IDSM".to_string()),
        "Ant/XML" => lang(ant_or_xml(&lines)?),
        "Maven/XML" => lang(maven_or_xml(&lines)?),
        // Brainfuck's extensions are common enough that cloc verifies the
        // content before claiming a file.
        "Brainfuck" => match really_is_bf(&lines)? {
            true => lang("Brainfuck".to_string()),
            false => ignored("does not look like Brainfuck".to_string()),
        },
        other => {
            let _ = opts;
            lang(other.to_string())
        }
    })
}

fn lang(name: String) -> Classification {
    Classification::Language(name)
}

/// Real languages whose extension is shared with something cloc checks for by
/// content even though the table does not call it a collision.
static NEEDS_CONTENT_CHECK: &[&str] = &["Ant/XML", "Maven/XML", "Brainfuck"];

// --- scoring -------------------------------------------------------------- {{{1

/// The `sort { $points{$b} <=> $points{$a} or $a cmp $b }` idiom: highest
/// score wins, ties broken by language name.
#[derive(Default)]
struct Scores(BTreeMap<&'static str, f64>);

impl Scores {
    fn add(&mut self, language: &'static str, points: f64) {
        *self.0.entry(language).or_insert(0.0) += points;
    }
    fn set(&mut self, language: &'static str, points: f64) {
        self.0.insert(language, points);
    }
    fn winner(&self) -> String {
        self.0
            .iter()
            // BTreeMap iterates in name order, so a strict `>` keeps the
            // alphabetically first of any tie.
            .fold(None::<(&str, f64)>, |best, (name, score)| match best {
                Some((_, b)) if *score <= b => best,
                _ => Some((name, *score)),
            })
            .map(|(name, _)| name.to_string())
            .unwrap_or_else(|| UNKNOWN.to_string())
    }
}

/// Compile once, use across every file of a run.
fn re(pattern: &str) -> Result<&'static fancy_regex::Regex> {
    regex_cache::cached(pattern)
}

// --- resolvers ------------------------------------------------------------ {{{1

fn matlab_or_objective_c(lines: &[String]) -> Result<String> {
    let mut s = Scores::default();
    let (c_comment, mercury, matrix) = (re(r"^\s*(/\*|//)")?, re(r"^:-\s+")?, re(r"\w+\s*=\s*\[")?);
    let (fn_call, assign, mumps_pair) =
        (re(r"\w+\[")?, re(r"^\s*\w+\s*=\s*")?, re(r"^\s*\.?(\w)\s+(\w)")?);
    let (include, keyword) = (
        re(r"^\s*#(include|import)")?,
        re(r"^\s*@(interface|implementation|protocol|public|protected|private|end)\s")?,
    );
    let kill = re(r"^\sK(ill)?\s+")?;

    for (i, line) in lines.iter().enumerate() {
        if i == 0 && re(r"^[A-Z]")?.is_match(line)? {
            s.add("MUMPS", 1.0);
        }
        if c_comment.is_match(line)? {
            s.add("Objective-C", 5.0);
            s.add("MATLAB", -5.0);
        } else if mercury.is_match(line)? {
            s.set("Mercury", 1000.0);
            break;
        } else if matrix.is_match(line)? {
            s.add("MATLAB", 5.0);
        }

        if fn_call.is_match(line)? {
            s.add("Mathematica", 2.0);
        } else if assign.is_match(line)? {
            s.add("MUMPS", -1.0);
        } else if let Some(caps) = mumps_pair.captures(line)? {
            let digitless = |n: usize| {
                caps.get(n)
                    .is_some_and(|m| !m.as_str().chars().any(|c| c.is_ascii_digit()))
            };
            if digitless(1) && digitless(2) {
                s.add("MUMPS", 1.0);
            }
        } else if re(r"^\s*;")?.is_match(line)? {
            s.add("MUMPS", 1.0);
        }

        if include.is_match(line)? {
            s.set("Objective-C", 1000.0);
            s.set("MATLAB", 0.0);
            break;
        } else if keyword.is_match(line)? {
            s.set("Objective-C", 1000.0);
            s.set("MATLAB", 0.0);
            break;
        } else if re(r"^\s*BeginPackage")?.is_match(line)? {
            s.add("Mathematica", 2.0);
        } else if re(r"^\s*\[")?.is_match(line)? {
            s.add("MATLAB", 5.0);
        } else if kill.is_match(line)? {
            s.add("MUMPS", 5.0);
        } else if re(r"^\s*function")?.is_match(line)? {
            s.add("Objective-C", -1.0);
            s.add("MATLAB", 1.0);
        } else if re(r"^\s*%")?.is_match(line)? {
            s.add("Objective-C", -1.0);
            s.add("MATLAB", 1.0);
        }
    }

    for l in ["MATLAB", "Mathematica", "MUMPS", "Objective-C", "Mercury"] {
        s.0.entry(l).or_insert(0.0);
    }
    Ok(s.winner())
}

fn php_pascal_fortran_pawn(lines: &[String]) -> Result<String> {
    let mut s = Scores::default();
    let mut fortran_90 = false;

    for line in lines {
        if re(r"(?i)^\s*<\?php")?.is_match(line)? {
            s.add("PHP", 100.0);
            break;
        } else if re(r"(?i)^\s*end\.")?.is_match(line)? {
            s.add("Pascal", 100.0);
            break;
        } else if re(r"(?i)^\s*implicit\s+(none|real|integer|complex|double)")?.is_match(line)? {
            s.add("Fortran", 100.0);
        } else if re(r"(?i)^\s*end;")?.is_match(line)? {
            s.add("Pascal", 1.0);
            break;
        } else if re(r"(?i)^\s*program\b")?.is_match(line)? {
            s.add("Fortran", 1.0);
            s.add("Pascal", 1.0);
        } else if re(r"(?i)^\s*(subroutine|module|stop)\b")?.is_match(line)? {
            s.add("Fortran", 1.0);
        } else if re(r"(?i)^\s*(writeln|begin|procedure|interface|implementation|const)\b")?
            .is_match(line)?
        {
            s.add("Pascal", 1.0);
        } else if re(r"^\s*(<|\?>)")?.is_match(line)? {
            s.add("PHP", 1.0);
        } else if re(r";\s*$")?.is_match(line)? {
            s.add("Pascal", 1.0);
            s.add("PHP", 1.0);
        } else if re(r"^@")?.is_match(line)? || re(r"^\s*(Float:|bool:)")?.is_match(line)? {
            s.add("Pawn", 1e20);
            break;
        } else if re(r"(?i)^\s*(%include|\.include|include)\b")?.is_match(line)? {
            s.add("Assembly", 100.0);
            break;
        }

        if re(r"(?i)^\s*(integer|real|complex|double|character|logical|public|private).*?::")?
            .is_match(line)?
        {
            fortran_90 = true;
        }
        if re(r"(?i)^\s*(SRC_URI|SRC_REV|SUMMARY|LICENSE|DEPENDS|RDEPENDS|LIC_FILES_CHKSUM)\s*=")?
            .is_match(line)?
        {
            s.add("BitBake", 1.0);
        }
        if re(r"(?i)(\.=|=\.|\?=|\?\?=)")?.is_match(line)? {
            s.add("BitBake", 1.0);
        }
        if re(r"^[A-Z0-9_:-]+:[A-Za-z0-9_-]+\s*=")?.is_match(line)? {
            s.add("BitBake", 1.0);
        }
        if re(r"(?i)^\s*(mov|push|pop|add|jmp|call|ret)\b")?.is_match(line)? {
            s.add("Assembly", 1.0);
        }
    }

    for l in ["Fortran", "PHP", "Pascal", "Pawn", "BitBake", "Assembly"] {
        s.0.entry(l).or_insert(0.0);
    }
    let winner = s.winner();
    Ok(if winner == "Fortran" {
        // The table has no bare "Fortran"; pick a dialect.
        if fortran_90 { "Fortran 90" } else { "Fortran 77" }.to_string()
    } else {
        winner
    })
}

fn pascal_or_puppet(lines: &[String]) -> Result<String> {
    let mut pascal = 0.0f64;
    let mut puppet = 0.0f64;

    for line in lines {
        if re(r"^\s*#\s+")?.is_match(line)? {
            puppet += 0.001;
            continue;
        }
        for p in [
            r"(?i)\bprogram\s+[A-Za-z]",
            r"(?i)\bunit\s+[A-Za-z]",
            r"(?i)\bmodule\s+[A-Za-z]",
            r"(?i)\bprocedure\b",
            r"(?i)\bfunction\b",
            r"(?i)^\s*interface\s+",
            r"(?i)^\s*implementation\s+",
            r"(?i)^\s*uses\s+",
            r"(?i)(?<!::)\bbegin\b(?!::)",
            r"(?i)(?<!::)\bend\b(?!::)",
            r":=",
            r"<>",
            r"(?i)^\s*\{\$(I|INCLUDE)\s+.*\}",
            r"writeln",
        ] {
            if re(p)?.is_match(line)? {
                pascal += 1.0;
            }
        }
        if re(r"^\s*class\s+")?.is_match(line)? && !re(r"class\s+operator\s+")?.is_match(line)? {
            puppet += 1.0;
        }
        for p in [
            r"^\s*function\s+[a-z][a-z0-9]+::[a-z][a-z0-9]+\s*",
            r"^\s*type\s+[A-Z]\w+::[A-Z]\w+\s+",
            r"^\s*case\s+",
            r"^\s*package\s+",
            r"^\s*file\s+",
            r"^\s*include\s\w+",
            r"^\s*service\s+",
            r"\s\$\w+\s*=\s*\S",
            r"\S\s*=>\s*\S",
        ] {
            if re(p)?.is_match(line)? {
                puppet += 1.0;
            }
        }
        // Stop early once the answer is not in doubt.
        if (pascal - puppet).abs() > 20.0 {
            break;
        }
    }
    Ok(if pascal > puppet { "Pascal" } else { "Puppet" }.to_string())
}

/// Lisp shares `.cl` and `.jl`; both resolvers score the same Lisp markers
/// against a different rival.
fn lisp_points(line: &str) -> Result<f64> {
    let mut p = 0.0;
    if re(r"^\s*;")?.is_match(line)? {
        p += 1.0;
    }
    if re(r"\((def|eval|require|export|let|loop|dec|format)")?.is_match(line)? {
        p += 1.0;
    }
    Ok(p)
}

fn lisp_or_opencl(lines: &[String]) -> Result<String> {
    let (mut lisp, mut opencl) = (0.0, 0.0);
    for line in lines {
        lisp += lisp_points(line)?;
        if re(r"^\s*(int|float|const|\{)")?.is_match(line)? {
            opencl += 1.0;
        }
    }
    Ok(if lisp > opencl { "Lisp" } else { "OpenCL" }.to_string())
}

fn lisp_or_julia(lines: &[String]) -> Result<String> {
    let (mut lisp, mut julia) = (0.0, 0.0);
    for line in lines {
        lisp += lisp_points(line)?;
        if re(r"^\s*(function|end|println|for|while)")?.is_match(line)? {
            julia += 1.0;
        }
    }
    Ok(if lisp > julia { "Lisp" } else { "Julia" }.to_string())
}

fn perl_or_prolog(lines: &[String]) -> Result<String> {
    let (mut perl, mut prolog) = (0.0, 0.0);
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        if i == 0 && re(r"^#!.*?\bperl")?.is_match(line)? {
            perl = 100.0;
            break;
        }
        for p in [
            r"^=(head|over|item|cut)",
            r";\s*$",
            r"(\{|\})",
            r"^\s*sub\s+",
            r"\s*<<'",
            r"\$(\w+->|[_!])",
        ] {
            if re(p)?.is_match(line)? {
                perl += 1.0;
            }
        }
        if !re(r"\s*#")?.is_match(line)? && re(r"\.\s*$")?.is_match(line)? {
            prolog += 1.0;
        }
        if re(r":-")?.is_match(line)? {
            prolog += 1.0;
        }
    }
    Ok(if perl > prolog { "Perl" } else { "Prolog" }.to_string())
}

fn idl_or_qtproject(lines: &[String]) -> Result<String> {
    let mut s = Scores::default();
    for line in lines {
        if re(r"^\s*;")?.is_match(line)? {
            s.add("IDL", 1.0);
        }
        if re(r"(?i)plot\(")?.is_match(line)? {
            s.add("IDL", 1.0);
        }
        if re(r"(?i)^\s*(qt|configs|sources|template|target|targetpath|subdirs)\b")?
            .is_match(line)?
        {
            s.add("Qt Project", 1.0);
        }
        if re(r"(?i)qthavemodule")?.is_match(line)? {
            s.add("Qt Project", 1.0);
        }
        if re(r"\.\s*$")?.is_match(line)? {
            s.add("Prolog", 1.0);
        }
        if re(r":-")?.is_match(line)? {
            s.add("Prolog", 1.0);
        }
        if re(r"^\s*#")?.is_match(line)? {
            s.add("ProGuard", 1.0);
        }
        if re(r"^-keep")?.is_match(line)? {
            s.add("ProGuard", 1.0);
        }
        if re(r"^-(dont)?obfuscate")?.is_match(line)? {
            s.add("ProGuard", 1.0);
        }
    }
    for l in ["IDL", "Qt Project", "Prolog", "ProGuard"] {
        s.0.entry(l).or_insert(0.0);
    }
    Ok(s.winner())
}

fn forth_or_fortran(lines: &[String]) -> Result<String> {
    let (mut forth, mut fortran) = (0.0, 0.0);
    for line in lines {
        if re(r"^:\s")?.is_match(line)? {
            forth += 1.0;
        }
        if re(r"(?i)^([c*][^a-z]|\s{6,}(subroutine|program|end|implicit)\s|\s*!)")?
            .is_match(line)?
        {
            fortran += 1.0;
        }
    }
    Ok(if forth > fortran { "Forth" } else { "Fortran 77" }.to_string())
}

fn forth_or_fsharp(lines: &[String]) -> Result<String> {
    let (mut forth, mut fsharp) = (0.0, 0.0);
    for line in lines {
        if re(r"^:\s")?.is_match(line)? {
            forth += 1.0;
        }
        if re(r"^\s*(#light|import|let|module|namespace|open|type)")?.is_match(line)? {
            fsharp += 1.0;
        }
    }
    Ok(if forth > fsharp { "Forth" } else { "F#" }.to_string())
}

fn verilog_or_coq(lines: &[String]) -> Result<String> {
    let (mut verilog, mut coq) = (0.0, 0.0);
    let coq_kw = re(concat!(
        r"\b(Inductive|Fixpoint|Definition|Theorem|Lemma|Proof|Qed|forall|",
        r"Section|Check|Notation|Variable|Goal|Fail|Require|Scheme|Module|Ltac|",
        r"Set|Unset|Parameter|Coercion|Axiom|Locate|Type|Record|Existing|Class)\b"
    ))?;
    for line in lines {
        if re(r"^\s*(module|begin|input|output|always)")?.is_match(line)? {
            verilog += 1.0;
        }
        if coq_kw.is_match(line)? {
            coq += 1.0;
        }
    }
    Ok(if coq > verilog {
        "Coq"
    } else {
        "Verilog-SystemVerilog"
    }
    .to_string())
}

fn typescript_or_qtlinguist(lines: &[String]) -> Result<String> {
    let (mut ts, mut linguist) = (0.0, 0.0);
    for line in lines {
        if re(r"\b</?(message|source|translation)>")?.is_match(line)? {
            linguist += 1.0;
        }
        if re(r"^\s*(var|const|let|class|document)\b")?.is_match(line)? {
            ts += 1.0;
        }
        if re(r"[;}]\s*$")?.is_match(line)? {
            ts += 1.0;
        }
        if re(r"^\s*//")?.is_match(line)? {
            ts += 1.0;
        }
    }
    // A tie goes to TypeScript here, unlike most of the other resolvers.
    Ok(if ts >= linguist {
        "TypeScript"
    } else {
        "Qt Linguist"
    }
    .to_string())
}

fn qt_or_glade(lines: &[String]) -> Result<String> {
    for line in lines {
        if re(r"(?i)generated\s+with\s+glade")?.is_match(line)? {
            return Ok("Glade".to_string());
        }
    }
    Ok("XML (Qt/GTK)".to_string())
}

fn csharp_or_smalltalk(lines: &[String]) -> Result<String> {
    let (mut cs, mut smalltalk) = (0.0, 0.0);
    for line in lines {
        if re(r"[;}{]\s*$")?.is_match(line)? {
            cs += 1.0;
        } else if re(r"^(using|namespace)\s")?.is_match(line)? {
            cs += 20.0;
        } else if re(r"^\s*(public|private|new)\s")?.is_match(line)? {
            cs += 20.0;
        } else if re(r"^\s*\[assembly:")?.is_match(line)? {
            cs += 1.0;
        }
        if re(r"(!|\]\.)\s*$")?.is_match(line)? {
            smalltalk += 1.0;
            cs -= 1.0;
        }
    }
    Ok(if smalltalk > cs { "Smalltalk" } else { "C#" }.to_string())
}

fn visual_basic_or_tex_or_apex(lines: &[String]) -> Result<String> {
    let mut s = Scores::default();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        if re(r"\s*%")?.is_match(line)? || re(r"\s*\\")?.is_match(line)? {
            s.add("TeX", 1.0);
        } else {
            if re(r"(?i)^\s*(public|private)\s")?.is_match(line)? {
                s.add("Visual Basic", 1.0);
                s.add("Apex Class", 1.0);
            } else if re(r"(?i)^\s*(end|attribute|version)\s")?.is_match(line)? {
                s.add("Visual Basic", 1.0);
            }
            if re(r"[{}]")?.is_match(line)? || re(r";\s*$")?.is_match(line)? {
                s.add("Apex Class", 1.0);
            }
        }
    }
    for l in ["Visual Basic", "TeX", "Apex Class"] {
        s.0.entry(l).or_insert(0.0);
    }
    Ok(s.winner())
}

fn scheme_or_saltstack(lines: &[String]) -> Result<String> {
    let (mut scheme, mut salt) = (0.0, 0.0);
    for line in lines {
        if re(r"\{%.*%\}")?.is_match(line)? {
            salt += 5.0;
        } else if re(r"map\.jinja\b")?.is_match(line)? {
            salt += 5.0;
        } else if re(r"\((define|lambda|let|cond|do)\s")?.is_match(line)? {
            scheme += 1.0;
        }
    }
    Ok(if scheme > salt { "Scheme" } else { "SaltStack" }.to_string())
}

fn pascal_or_pawn(lines: &[String]) -> Result<String> {
    let (mut pascal, mut pawn) = (0.0f64, 0.0f64);
    for line in lines {
        if re(r"^\s*(program|begin|end|[Ww]riteln|procedure)\s")?.is_match(line)? {
            pascal += 1e20;
            break;
        } else if re(r"^@")?.is_match(line)?
            || line.contains("/*")
            || line.contains("*/")
            || re(r"^\s*(Float:|bool:)")?.is_match(line)?
        {
            pawn += 1e20;
            break;
        } else if re(r";\s*$")?.is_match(line)? {
            pawn += 1.0;
            pascal += 1.0;
        }
    }
    Ok(if pascal > pawn { "Pascal" } else { "Pawn" }.to_string())
}

fn skill_or_dotnet_il(lines: &[String]) -> Result<String> {
    let (mut skill, mut il) = (0.0, 0.0);
    for line in lines {
        if re(r"^\s*;")?.is_match(line)? {
            skill += 50.0;
        } else if re(r"^\.(class|assembly|method|custom|entrypoint)")?.is_match(line)? {
            il += 50.0;
            break;
        } else if re(r"\{\s*$")?.is_match(line)? || re(r"^\s*\}")?.is_match(line)? {
            il += 5.0;
        } else if re(r"^\s*(procedure|let|foreach)\b")?.is_match(line)? {
            skill += 1.0;
        }
    }
    Ok(if skill > il { "SKILL" } else { ".NET IL" }.to_string())
}

fn clojure_or_cangjie(lines: &[String]) -> Result<String> {
    let (mut clojure, mut cangjie) = (0.0, 0.0);
    for line in lines {
        if re(r"^\s*(;|\(|\[)")?.is_match(line)? {
            clojure += 50.0;
        } else if re(r"^(import|func|main)")?.is_match(line)? {
            cangjie += 50.0;
        } else if re(r"\{\s*$")?.is_match(line)? || re(r"^\s*\}$")?.is_match(line)? {
            cangjie += 5.0;
        } else if re(r"^\s*(procedure|let|foreach)\b")?.is_match(line)? {
            clojure += 1.0;
        }
    }
    Ok(if clojure > cangjie {
        "Clojure"
    } else {
        "Cangjie"
    }
    .to_string())
}

fn really_is_bitbake(lines: &[String]) -> Result<bool> {
    let mut points = 0.0;
    for line in lines {
        if re(r"(?i)^\s*(SRC_URI|SRC_REV|SUMMARY|LICENSE|DEPENDS|RDEPENDS|LIC_FILES_CHKSUM)\s*=")?
            .is_match(line)?
        {
            points += 1.0;
        }
        if re(r"(?i)(\.=|=\.|\?=|\?\?=)")?.is_match(line)? {
            points += 1.0;
        }
        if re(r"^[A-Z0-9_:-]+:[A-Za-z0-9_-]+\s*=")?.is_match(line)? {
            points += 1.0;
        }
    }
    Ok(points > 0.0)
}

fn really_is_d(lines: &[String]) -> Result<bool> {
    // dtrace scripts open with a probe clause; D source does not.
    for line in lines {
        if re(r"^\s*(BEGIN|END|dtrace:|profile:|syscall:|proc:|fbt:)")?.is_match(line)? {
            return Ok(false);
        }
        if re(r"^\s*(module|import)\s+\w")?.is_match(line)? {
            return Ok(true);
        }
    }
    Ok(true)
}

/// Brainfuck is mostly punctuation, so cloc checks for dense runs of its
/// commands before claiming a file.
fn really_is_bf(lines: &[String]) -> Result<bool> {
    let indicator = re(r"([+-]{4,}|[\[\]]{4,}|[<>][+-]|<{3,}|^\s*[\[\]]\s*$)")?;
    let mut hits = 0usize;
    for line in lines {
        if indicator.is_match(line)? {
            hits += 1;
        }
    }
    let ratio = if lines.is_empty() {
        0.0
    } else {
        hits as f64 / lines.len() as f64
    };
    Ok(ratio > 0.5 || hits > 5)
}

fn ant_or_xml(lines: &[String]) -> Result<String> {
    let (mut ant, mut xml) = (0.0, 1.0);
    for line in lines {
        if re(r"^\s*<project\s+")?.is_match(line)? {
            ant += 1.0;
            xml -= 1.0;
        }
        if line.contains(r#"xmlns:artifact="antlib:org.apache.maven.artifact.ant""#) {
            ant += 1.0;
            xml -= 1.0;
        }
    }
    // A tie goes to XML.
    Ok(if xml >= ant { "XML" } else { "Ant" }.to_string())
}

fn maven_or_xml(lines: &[String]) -> Result<String> {
    let (mut mvn, mut xml) = (0.0, 1.0);
    for line in lines {
        if re(r"^\s*<project\s+")?.is_match(line)? {
            mvn += 1.0;
            xml -= 1.0;
        }
    }
    Ok(if xml >= mvn { "XML" } else { "Maven" }.to_string())
}

#[cfg(test)]
mod tests;
