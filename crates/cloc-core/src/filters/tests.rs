use super::*;
use cloc_lang::LangDb;

/// Run a language's real filter chain over `source`, returning the lines that
/// survive as code.
fn run(language: &str, source: &str, options: FilterOptions) -> Vec<String> {
    run_as(language, source, options, "test.txt")
}

fn run_as(language: &str, source: &str, options: FilterOptions, name: &str) -> Vec<String> {
    let db = LangDb::default_db();
    let filters = db.filters(language).expect("language has filters");
    let path = std::path::PathBuf::from(name);
    let ctx = FilterContext {
        options: &options,
        file: &path,
        language,
        eol_continuation: db.eol_continuation(language),
    };
    let lines: Vec<String> = source.lines().map(str::to_string).collect();
    let lines = remove_blank_lines(lines, ctx.eol_continuation).unwrap();
    apply_chain(lines, filters, &ctx).unwrap()
}

#[test]
fn c_source_keeps_only_code() {
    let out = run(
        "C",
        "/* header */\n#include <stdio.h>\nint main() { /* x */ return 0; }\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["#include <stdio.h>", "int main() {  return 0; }"]);
}

#[test]
fn cpp_line_comments_go() {
    let out = run(
        "C++",
        "int a; // trailing\n// whole line\nint b;\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["int a; ", "int b;"]);
}

#[test]
fn python_docstrings_count_as_comments() {
    let out = run(
        "Python",
        "def f():\n    \"\"\"Doc.\n    More.\n    \"\"\"\n    return 1\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["def f():", "    return 1"]);
}

/// `--docstring-as-code` flips docstrings from comment to code.
#[test]
fn docstring_as_code_keeps_them() {
    let opts = FilterOptions {
        docstring_as_code: true,
        ..Default::default()
    };
    let out = run("Python", "def f():\n    \"\"\"Doc.\"\"\"\n    return 1\n", opts);
    assert_eq!(out.len(), 3);
}

#[test]
fn shell_keeps_its_shebang_as_code() {
    let out = run(
        "Bourne Again Shell",
        "#!/bin/bash\n# a comment\necho hi\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["#!/bin/bash", "echo hi"]);
}

/// `remove_inline` is a no-op unless `--inline` is given. Without it, the
/// code part of a mixed line survives via other filters, not this one.
#[test]
fn inline_filter_is_off_by_default() {
    let source = "x = 1 # note\n";
    let default = run("Python", source, FilterOptions::default());
    let inlined = run(
        "Python",
        source,
        FilterOptions {
            inline: true,
            ..Default::default()
        },
    );
    assert_eq!(default, vec!["x = 1 # note"]);
    assert_eq!(inlined, vec!["x = 1 "]);
}

/// Comment markers inside strings are stripped by default, matching cloc's
/// own behaviour; `--strip-str-comments` is what protects them.
#[test]
fn string_comment_protection_is_opt_in() {
    let source = "s = \"/* not a comment */\";\nint x;\n";
    let default = run("C", source, FilterOptions::default());
    let protected = run(
        "C",
        source,
        FilterOptions {
            strip_str_comments: true,
            ..Default::default()
        },
    );
    assert_eq!(default, vec!["s = \"\";", "int x;"]);
    assert_eq!(protected, vec!["s = \"xx not a comment xx\";", "int x;"]);
}

#[test]
fn html_comments_removed() {
    let out = run(
        "HTML",
        "<html>\n<!-- a\n     multi-line comment -->\n<body>\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["<html>", "<body>"]);
}

#[test]
fn fortran_77_fixed_form_comments() {
    let out = run(
        "Fortran 77",
        "C this is a comment\n      PROGRAM MAIN\n* another comment\n      END\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["      PROGRAM MAIN", "      END"]);
}

/// `remove_f90_comments` spares OpenMP and HPF directives — but Fortran 90's
/// chain runs `remove_f77_comments` first, which drops every `^\s*!` line and
/// takes the directives with it. cloc counts them as comments, so we do too.
#[test]
fn fortran_90_directives_are_eaten_by_the_f77_pass() {
    let out = run(
        "Fortran 90",
        "! a comment\n!$omp parallel do\nx = 1\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["x = 1"]);
}

#[test]
fn haskell_block_and_line_comments() {
    let out = run(
        "Haskell",
        "-- a comment\nmain = do\n{- block\n   comment -}\n  return ()\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["main = do", "  return ()"]);
}

/// Haskell pragmas are code even though they look like block comments.
#[test]
fn haskell_pragmas_are_code() {
    let out = run(
        "Haskell",
        "{-# LANGUAGE CPP #-}\nmain = return ()\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["{-# LANGUAGE CPP #-}", "main = return ()"]);
}

#[test]
fn ocaml_comments_nest() {
    let out = run(
        "OCaml",
        "let x = 1\n(* outer (* inner *) still comment *)\nlet y = 2\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["let x = 1", "let y = 2"]);
}

#[test]
fn lua_comments() {
    let out = run(
        "Lua",
        "-- a comment\nlocal x = 1\n--[[ block\ncomment ]]\nlocal y = 2\n",
        FilterOptions::default(),
    );
    assert!(out.contains(&"local x = 1".to_string()));
    assert!(out.contains(&"local y = 2".to_string()));
    assert!(!out.iter().any(|l| l.contains("block")));
}

#[test]
fn ruby_comments() {
    let out = run(
        "Ruby",
        "# a comment\nputs 1\n=begin\ndoc\n=end\nputs 2\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["puts 1", "puts 2"]);
}

#[test]
fn sql_comments() {
    let out = run(
        "SQL",
        "-- a comment\nSELECT 1;\n/* block */\nSELECT 2;\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["SELECT 1;", "SELECT 2;"]);
}

#[test]
fn asp_uses_an_octal_escaped_quote_marker() {
    let out = run(
        "ASP",
        "' a comment\nResponse.Write 1\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["Response.Write 1"]);
}

#[test]
fn apl_unicode_comment_marker() {
    let out = run("APL", "⍝ a comment\nx←1\n", FilterOptions::default());
    assert_eq!(out, vec!["x←1"]);
}

#[test]
fn brainfuck_keeps_only_commands() {
    let out = run(
        "Brainfuck",
        "this text is a comment\n++[->+<]\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["++[->+<]"]);
}

/// Perl's POD blocks and `__END__` section are documentation, not code.
#[test]
fn perl_pod_and_end_marker() {
    let out = run(
        "Perl",
        "my $x = 1;\n=pod\ndocs\n=cut\nmy $y = 2;\n__END__\ntrailing junk\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["my $x = 1;", "my $y = 2;"]);
}

/// Literate Haskell in Bird style: only lines starting with `>` are code.
#[test]
fn literate_haskell_bird_style() {
    let out = run_as(
        "Haskell",
        "This is prose.\n\n> main = return ()\n\nMore prose.\n",
        FilterOptions::default(),
        "lit.lhs",
    );
    assert_eq!(out, vec![" main = return ()"]);
}

#[test]
fn jcl_stops_at_the_job_terminator() {
    let out = run(
        "JCL",
        "//STEP1 EXEC PGM=X\n//* a comment\n//\nnot counted\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["//STEP1 EXEC PGM=X"]);
}

#[test]
fn powershell_block_comments() {
    let out = run(
        "PowerShell",
        "<#\n block comment\n#>\nWrite-Host 1\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["Write-Host 1"]);
}

#[test]
fn rmd_keeps_only_fenced_code() {
    let out = run(
        "Rmd",
        "Some prose.\n\n```{r}\nx <- 1\n```\n\nMore prose.\n",
        FilterOptions::default(),
    );
    assert_eq!(out, vec!["x <- 1"]);
}

/// A language whose filter chain is `die` must be resolved to a real language
/// before counting; reaching it is an error rather than a silent zero.
#[test]
fn collision_pseudo_language_refuses_to_count() {
    let db = LangDb::default_db();
    let filters = db.filters("Perl/Prolog").expect("pseudo-language present");
    let options = FilterOptions::default();
    let path = std::path::PathBuf::from("x.pl");
    let ctx = FilterContext {
        options: &options,
        file: &path,
        language: "Perl/Prolog",
        eol_continuation: None,
    };
    let err = apply_chain(vec!["x".to_string()], filters, &ctx).unwrap_err();
    assert!(err.to_string().contains("not directly countable"));
}

/// Every language in the table must survive its own filter chain without
/// panicking or erroring on ordinary input. This is the broad guard against
/// an unimplemented or mis-wired filter.
#[test]
fn every_language_chain_runs() {
    let db = LangDb::default_db();
    let options = FilterOptions::default();
    let sample = "alpha beta\n\
                  /* c comment */\n\
                  // line comment\n\
                  # hash comment\n\
                  -- dash comment\n\
                  <!-- html comment -->\n\
                  x = 1\n";
    let mut failures = Vec::new();

    for language in db.languages() {
        let filters = db.filters(language).unwrap();
        // Pseudo-languages are expected to refuse; they are covered above.
        if filters.iter().any(|f| matches!(f, Filter::Die { .. })) {
            continue;
        }
        let path = std::path::PathBuf::from("sample.txt");
        let ctx = FilterContext {
            options: &options,
            file: &path,
            language,
            eol_continuation: db.eol_continuation(language),
        };
        let lines: Vec<String> = sample.lines().map(str::to_string).collect();
        let lines = match remove_blank_lines(lines, ctx.eol_continuation) {
            Ok(l) => l,
            Err(e) => {
                failures.push(format!("{language}: blank removal: {e}"));
                continue;
            }
        };
        if let Err(e) = apply_chain(lines, filters, &ctx) {
            failures.push(format!("{language}: {e}"));
        }
    }

    assert!(
        failures.is_empty(),
        "{} language chain(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
