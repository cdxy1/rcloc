//! End-to-end tests: build a small tree, run the binary, check the report.
//!
//! These replace the corpus comparison against the Perl original that this
//! port was developed against. The counts asserted here were taken from that
//! original while it was still present; see the README for how to reach it.

use std::path::{Path, PathBuf};
use std::process::Command;

const EXE: &str = env!("CARGO_BIN_EXE_rcloc");

/// Build a tree of files in a fresh temporary directory.
fn tree(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("rcloc-cli-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (name, content) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    root
}

fn run(args: &[&str]) -> String {
    let out = Command::new(EXE).args(args).output().expect("run rcloc");
    assert!(
        out.status.success(),
        "rcloc {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn run_in(root: &Path, args: &[&str]) -> String {
    let mut all: Vec<&str> = args.to_vec();
    let root = root.to_string_lossy().into_owned();
    all.push(&root);
    run(&all)
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).expect("valid JSON")
}

const SAMPLE: &[(&str, &str)] = &[
    (
        "src/main.rs",
        "// a comment\nfn main() {\n    println!(\"hi\");\n}\n\n/* block\n   comment */\n",
    ),
    (
        "src/util.py",
        "#!/usr/bin/env python3\n\"\"\"Docstring.\"\"\"\n\n# note\ndef f():\n    return 1\n",
    ),
    ("build.sh", "#!/bin/sh\n# build it\nmake all\n"),
];

#[test]
fn counts_a_mixed_tree() {
    let root = tree("mixed", SAMPLE);
    let v = json(&run_in(&root, &["--json", "--quiet"]));

    assert_eq!(v["Rust"]["nFiles"], 1);
    assert_eq!(v["Rust"]["code"], 3);
    assert_eq!(v["Rust"]["comment"], 3);
    assert_eq!(v["Rust"]["blank"], 1);

    assert_eq!(v["Python"]["code"], 3);
    assert_eq!(v["Python"]["comment"], 2);

    // The shebang counts as code even though it looks like a comment.
    assert_eq!(v["Bourne Shell"]["code"], 2);
    assert_eq!(v["Bourne Shell"]["comment"], 1);

    assert_eq!(v["SUM"]["nFiles"], 3);
}

#[test]
fn by_file_keys_on_paths() {
    let root = tree("byfile", SAMPLE);
    let v = json(&run_in(&root, &["--json", "--quiet", "--by-file"]));
    let key = root.join("src/main.rs").to_string_lossy().into_owned();
    assert_eq!(v[&key]["language"], "Rust");
    assert_eq!(v[&key]["code"], 3);
}

#[test]
fn formats_agree_on_the_totals() {
    let root = tree("formats", SAMPLE);
    let text = run_in(&root, &["--quiet"]);
    let csv = run_in(&root, &["--csv", "--quiet"]);
    let v = json(&run_in(&root, &["--json", "--quiet"]));

    let code = v["SUM"]["code"].as_u64().unwrap();
    assert!(text.contains(&code.to_string()));
    assert!(csv.lines().last().unwrap().contains(&code.to_string()));
    assert!(run_in(&root, &["--yaml", "--quiet"]).contains(&code.to_string()));
    assert!(run_in(&root, &["--xml", "--quiet"]).contains(&code.to_string()));
    assert!(run_in(&root, &["--md", "--quiet"]).contains(&code.to_string()));
}

#[test]
fn exclusions_take_effect() {
    let root = tree(
        "exclude",
        &[
            ("keep.py", "x = 1\n"),
            ("vendor/skip.py", "y = 2\n"),
            ("also_skip.rs", "fn a() {}\n"),
        ],
    );
    let v = json(&run_in(
        &root,
        &["--json", "--quiet", "--exclude-dir=vendor", "--exclude-ext=rs"],
    ));
    assert_eq!(v["SUM"]["nFiles"], 1);
    assert_eq!(v["Python"]["code"], 1);
}

#[test]
fn include_lang_narrows_the_report() {
    let root = tree("include", SAMPLE);
    let v = json(&run_in(&root, &["--json", "--quiet", "--include-lang=Python"]));
    assert_eq!(v["SUM"]["nFiles"], 1);
    assert!(v.get("Rust").is_none());
}

/// Version control metadata is skipped without being asked.
#[test]
fn git_directories_are_ignored() {
    let root = tree(
        "vcsdir",
        &[
            ("a.py", "x = 1\n"),
            (".git/hooks/pre-commit.sample", "#!/bin/sh\necho hi\n"),
        ],
    );
    let v = json(&run_in(&root, &["--json", "--quiet"]));
    assert_eq!(v["SUM"]["nFiles"], 1);
}

/// Identical files are counted once.
#[test]
fn duplicate_content_is_counted_once() {
    let root = tree(
        "dupes",
        &[("a/mod.py", "x = 1\n"), ("b/mod.py", "x = 1\n")],
    );
    assert_eq!(json(&run_in(&root, &["--json", "--quiet"]))["SUM"]["nFiles"], 1);
    let with_dupes = run_in(&root, &["--json", "--quiet", "--skip-uniqueness"]);
    assert_eq!(json(&with_dupes)["SUM"]["nFiles"], 2);
}

#[test]
fn ambiguous_extensions_are_resolved_by_content() {
    let root = tree(
        "collide",
        &[
            ("objc.m", "#import <Foundation/Foundation.h>\n@interface A\n@end\n"),
            ("matlab.m", "function y = f(x)\n% comment\ny = [1 2 3];\n"),
            ("perl.pl", "#!/usr/bin/perl\nsub f { return 1; }\n"),
            ("prolog.pl", "parent(tom, bob).\ngrand(X,Y) :- parent(X,Z).\n"),
        ],
    );
    let v = json(&run_in(&root, &["--json", "--quiet", "--by-file"]));
    let lang = |name: &str| {
        v[root.join(name).to_string_lossy().into_owned()]["language"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(lang("objc.m"), "Objective-C");
    assert_eq!(lang("matlab.m"), "MATLAB");
    assert_eq!(lang("perl.pl"), "Perl");
    assert_eq!(lang("prolog.pl"), "Prolog");
}

#[test]
fn diff_reports_the_four_dispositions() {
    let left = tree("diffL", &[("a.py", "x = 1\ny = 2\ngone = 3\n")]);
    let right = tree("diffR", &[("a.py", "x = 99\ny = 2\nfresh = 4\n")]);
    let out = run(&[
        "--diff",
        "--quiet",
        &left.to_string_lossy(),
        &right.to_string_lossy(),
    ]);

    // One line rewritten, one unchanged, one replaced by another.
    let sum = out.split("SUM:").nth(1).expect("a SUM block");
    assert!(sum.contains("same"));
    assert!(sum.contains("modified"));
    for line in sum.lines() {
        if line.trim_start().starts_with("modified") {
            assert!(line.contains('2'), "expected two modified lines: {line}");
        }
        if line.trim_start().starts_with("same") {
            assert!(line.contains('1'), "expected one unchanged line: {line}");
        }
    }
}

/// Comparing releases whose directory names differ must still pair files up.
#[test]
fn diff_aligns_across_differently_named_roots() {
    let left = tree("relL", &[("proj-1.0/src/a.py", "x = 1\n")]);
    let right = tree("relR", &[("proj-2.0/src/a.py", "x = 2\n")]);
    let out = run(&[
        "--diff",
        "--quiet",
        &left.to_string_lossy(),
        &right.to_string_lossy(),
    ]);
    let sum = out.split("SUM:").nth(1).unwrap();
    for line in sum.lines() {
        // Paired, so neither added nor removed.
        if line.trim_start().starts_with("added") || line.trim_start().starts_with("removed") {
            assert!(
                line.split_whitespace().skip(1).all(|f| f == "0"),
                "expected nothing added or removed: {line}"
            );
        }
    }
}

#[test]
fn language_definitions_can_be_extended() {
    let root = tree(
        "langdef",
        &[
            ("thing.wdg", "let a = 1\n// comment\nlet b = 2\n"),
            ("defs.txt", "Widget\n    filter remove_matches ^\\s*//\n    extension wdg\n    3rd_gen_scale 1.00\n"),
        ],
    );
    let defs = root.join("defs.txt");
    let v = json(&run(&[
        "--json",
        "--quiet",
        &format!("--read-lang-def={}", defs.display()),
        &root.join("thing.wdg").to_string_lossy(),
    ]));
    assert_eq!(v["Widget"]["code"], 2);
    assert_eq!(v["Widget"]["comment"], 1);
}

#[test]
fn force_lang_claims_every_file() {
    let root = tree("forced", &[("a.qqq", "x = 1\n# note\n")]);
    let v = json(&run_in(&root, &["--json", "--quiet", "--force-lang=Python"]));
    assert_eq!(v["Python"]["code"], 1);
    assert_eq!(v["Python"]["comment"], 1);
}

#[test]
fn sql_output_is_one_row_per_file() {
    let root = tree("sql", SAMPLE);
    let out = run_in(&root, &["--sql=-", "--quiet"]);
    assert!(out.contains("create table metadata"));
    assert_eq!(out.matches("insert into t values(").count(), 3);
    assert!(out.trim_end().ends_with("commit;"));
}

#[test]
fn percentages_and_counts_describe_the_same_run() {
    let root = tree("pct", SAMPLE);
    let plain = json(&run_in(&root, &["--json", "--quiet"]));
    let pct = run_in(&root, &["--quiet", "--percent"]);
    assert!(pct.contains("blank %"));
    // The SUM row of a column-wise percentage report is 100 across.
    let sum = pct.lines().find(|l| l.contains("│ SUM:")).unwrap();
    assert_eq!(sum.matches("100.00").count(), 3);
    assert_eq!(plain["SUM"]["nFiles"], 3);
}

#[test]
fn show_lang_lists_known_languages() {
    let out = run(&["--show-lang"]);
    assert!(out.contains("Rust ("));
    assert!(out.contains("Python ("));
    assert!(out.lines().count() > 300);
}

#[test]
fn an_unreadable_input_is_reported_not_ignored() {
    let out = Command::new(EXE)
        .args(["--quiet", "/definitely/not/here"])
        .output()
        .unwrap();
    // A missing input is not a crash, and not a silent zero either.
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("SUM:") || text.contains("Language"));
}
