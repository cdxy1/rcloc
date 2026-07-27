use super::*;
use std::io::Write;

/// Write `content` to a temp file named `name` and classify it.
fn classify_content(name: &str, content: &str) -> Classification {
    classify_with(name, content, &ClassifyOptions::default())
}

fn classify_with(name: &str, content: &str, opts: &ClassifyOptions) -> Classification {
    let dir = std::env::temp_dir().join(format!("cloc-classify-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    drop(f);
    let result = classify(&path, LangDb::default_db(), opts).unwrap();
    let _ = std::fs::remove_file(&path);
    result
}

fn lang_of(name: &str, content: &str) -> String {
    classify_content(name, content)
        .language()
        .unwrap_or(UNKNOWN)
        .to_string()
}

#[test]
fn plain_extensions() {
    assert_eq!(lang_of("a.rs", "fn main() {}"), "Rust");
    assert_eq!(lang_of("a.py", "x = 1"), "Python");
    assert_eq!(lang_of("a.cpp", "int main(){}"), "C++");
}

#[test]
fn whole_file_names_win_over_extensions() {
    assert_eq!(lang_of("Makefile", "all:\n\techo"), "make");
    assert_eq!(lang_of("CMakeLists.txt", "project(x)"), "CMake");
}

/// A longer extension is tried first, so `.tar.gz` is not read as `.gz`.
#[test]
fn longest_extension_wins() {
    let cands = extension_candidates("foo.tar.gz", false);
    assert_eq!(cands.first().map(String::as_str), Some("tar.gz"));
    assert!(cands.contains(&"gz".to_string()));
}

#[test]
fn not_code_extensions_are_ignored() {
    let c = classify_content("photo.jpg", "");
    assert!(matches!(c, Classification::Ignored { .. }));
}

#[test]
fn editor_backups_are_ignored() {
    let c = classify_content("main.c~", "int x;");
    assert!(matches!(c, Classification::Ignored { .. }));
}

#[test]
fn shebang_names_the_language() {
    assert_eq!(lang_of("script", "#!/bin/bash\necho hi"), "Bourne Again Shell");
    assert_eq!(lang_of("script2", "#!/usr/bin/python3\nx = 1"), "Python");
}

/// `env` shebangs name the interpreter in the following word.
#[test]
fn env_shebang_is_followed() {
    assert_eq!(lang_of("s", "#!/usr/bin/env perl\nmy $x;"), "Perl");
    assert_eq!(lang_of("s2", "#!/usr/bin/env python3\nx = 1"), "Python");
}

/// An unrecognised extension on an XML file is rescued by the declaration.
#[test]
fn xml_declaration_rescues_unknown_extensions() {
    assert_eq!(
        lang_of("thing.weirdext", "<?xml version=\"1.0\"?>\n<a/>"),
        "XML"
    );
}

#[test]
fn unknown_stays_unknown() {
    let c = classify_content("mystery.qqq", "blah blah");
    assert!(matches!(c, Classification::Ignored { .. }));
}

#[test]
fn dockerfile_prefix_match() {
    assert_eq!(lang_of("Dockerfile.dev", "FROM alpine"), "Dockerfile");
}

/// `--lang-no-ext` claims extensionless files that have no shebang.
#[test]
fn lang_no_ext_option() {
    let opts = ClassifyOptions {
        lang_no_ext: Some("Python".to_string()),
        ..Default::default()
    };
    let c = classify_with("noext", "x = 1\n", &opts);
    assert_eq!(c.language(), Some("Python"));
}

/// `--autoconf` looks through the `.in` suffix of a template.
#[test]
fn autoconf_strips_dot_in() {
    let opts = ClassifyOptions {
        autoconf: true,
        ..Default::default()
    };
    let c = classify_with("config.h.in", "#define X 1\n", &opts);
    assert_eq!(c.language(), Some("C/C++ Header"));
}

// --- collision resolvers -------------------------------------------------- {{{1

#[test]
fn dot_m_objective_c_vs_matlab() {
    assert_eq!(
        lang_of("a.m", "#import <Foundation/Foundation.h>\n@interface A\n@end\n"),
        "Objective-C"
    );
    assert_eq!(
        lang_of("b.m", "function y = f(x)\n% a comment\ny = [1 2 3];\n"),
        "MATLAB"
    );
}

/// A single `:-` line is decisive for Mercury.
#[test]
fn dot_m_mercury() {
    assert_eq!(lang_of("c.m", ":- module foo.\n:- interface.\n"), "Mercury");
}

#[test]
fn dot_pl_perl_vs_prolog() {
    assert_eq!(
        lang_of("a.pl", "#!/usr/bin/perl\nsub f { return 1; }\n"),
        "Perl"
    );
    assert_eq!(
        lang_of("b.pl", "parent(tom, bob).\ngrand(X,Y) :- parent(X,Z).\n"),
        "Prolog"
    );
}

#[test]
fn dot_jl_lisp_vs_julia() {
    assert_eq!(
        lang_of("a.jl", "function f(x)\n  println(x)\nend\n"),
        "Julia"
    );
    assert_eq!(
        lang_of("b.jl", "; a comment\n(defun f (x) x)\n(let ((a 1)) a)\n"),
        "Lisp"
    );
}

#[test]
fn dot_ts_typescript_vs_qt_linguist() {
    assert_eq!(
        lang_of("a.ts", "const x: number = 1;\nclass A {}\n"),
        "TypeScript"
    );
    assert_eq!(
        lang_of(
            "b.ts",
            "<TS>\n<message>\n<source>Hi</source>\n<translation>Privet</translation>\n</message>\n</TS>\n"
        ),
        "Qt Linguist"
    );
}

#[test]
fn dot_v_verilog_vs_coq() {
    assert_eq!(
        lang_of("a.v", "module top;\ninput clk;\nalways @(posedge clk)\n"),
        "Verilog-SystemVerilog"
    );
    assert_eq!(
        lang_of("b.v", "Theorem foo : forall n, n = n.\nProof.\nQed.\n"),
        "Coq"
    );
}

#[test]
fn dot_cs_csharp_vs_smalltalk() {
    assert_eq!(
        lang_of("a.cs", "using System;\nnamespace N { public class A { } }\n"),
        "C#"
    );
}

#[test]
fn dot_pp_pascal_vs_puppet() {
    assert_eq!(
        lang_of(
            "a.pp",
            "program Hello;\nbegin\n  writeln('hi');\nend.\n"
        ),
        "Pascal"
    );
    assert_eq!(
        lang_of(
            "b.pp",
            "class ntp {\n  package { 'ntp': ensure => installed }\n  service { 'ntp': }\n}\n"
        ),
        "Puppet"
    );
}

#[test]
fn dot_inc_php_vs_fortran() {
    assert_eq!(lang_of("a.inc", "<?php\n$x = 1;\n"), "PHP");
}

#[test]
fn dot_fs_forth_vs_fsharp() {
    assert_eq!(
        lang_of("a.fs", "module Foo\nlet x = 1\nopen System\n"),
        "F#"
    );
    assert_eq!(lang_of("b.fs", ": square dup * ;\n: cube dup square * ;\n"), "Forth");
}

/// `.pro` is claimed by four languages at once.
#[test]
fn dot_pro_four_way() {
    assert_eq!(
        lang_of("a.pro", "TEMPLATE = app\nSOURCES += main.cpp\nQT += core\n"),
        "Qt Project"
    );
    assert_eq!(
        lang_of("b.pro", "-keep class com.example.**\n-dontobfuscate\n"),
        "ProGuard"
    );
}

/// The pseudo-language itself must never reach the counter.
#[test]
fn resolvers_never_return_a_collision_name() {
    let db = LangDb::default_db();
    for name in [
        lang_of("x.m", "x = 1\n"),
        lang_of("x.pl", "foo.\n"),
        lang_of("x.jl", "x = 1\n"),
        lang_of("x.ts", "x = 1;\n"),
        lang_of("x.cl", "(defun f () 1)\n"),
        lang_of("x.fs", "let x = 1\n"),
        lang_of("x.v", "module m;\n"),
        lang_of("x.pp", "class a { }\n"),
        lang_of("x.p6", "say 1;\n"),
        lang_of("x.sls", "(define x 1)\n"),
        lang_of("x.il", "; comment\n"),
        lang_of("x.cj", "(ns foo)\n"),
    ] {
        assert!(
            !db.is_collision(&name),
            "resolver returned the pseudo-language {name:?}"
        );
    }
}

/// The Ant resolver is reached only through the exact name `build.xml`; any
/// other `.xml` file goes straight to XML by extension. Within `build.xml`, a
/// tie goes to XML, so a `<project>` element is what tips it to Ant.
#[test]
fn ant_needs_the_right_name_and_a_project_element() {
    assert_eq!(lang_of("build.xml", "<?xml version=\"1.0\"?>\n<a/>\n"), "XML");
    assert_eq!(
        lang_of(
            "build.xml",
            "<project name=\"x\" default=\"all\">\n<target name=\"all\"/>\n</project>\n"
        ),
        "Ant"
    );
    // Same content, ordinary name: still plain XML.
    assert_eq!(
        lang_of(
            "other.xml",
            "<project name=\"x\" default=\"all\">\n<target name=\"all\"/>\n</project>\n"
        ),
        "XML"
    );
}
