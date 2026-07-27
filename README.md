<div align="center">

# ⚡ rcloc

### Your codebase, counted at Rust speed.

**A fast, modern source-code counter with familiar `cloc` accuracy and a cleaner CLI.**

[![Rust](https://img.shields.io/badge/Rust-1.75%2B-f74c00?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License: GPL v2+](https://img.shields.io/badge/license-GPL--2.0%2B-7c3aed.svg)](LICENSE)
[![Languages](https://img.shields.io/badge/languages-422-06b6d4.svg)](data/languages.json)
[![No runtime](https://img.shields.io/badge/runtime_dependencies-none-22c55e.svg)](#install)

```console
$ rcloc --hide-rate --thousands-delimiter=, crates
◉ rcloc 0.1.0
  Fast source insights, powered by Rust

┌──────────────────────┬───────────┬────────────────┬────────────────┬────────────────┐
│ Language             │     files │          blank │        comment │           code │
├──────────────────────┼───────────┼────────────────┼────────────────┼────────────────┤
│ Rust                 │        26 │            864 │          1,471 │          8,208 │
│ HTML                 │         1 │              6 │             48 │            284 │
│ TOML                 │         3 │              5 │              0 │             46 │
├──────────────────────┼───────────┼────────────────┼────────────────┼────────────────┤
│ SUM:                 │        30 │            875 │          1,519 │          8,538 │
└──────────────────────┴───────────┴────────────────┴────────────────┴────────────────┘
```

[Install](#install) · [Usage](#usage) · [Output formats](#output-formats) · [Why rcloc?](#why-rcloc) · [License](#license-and-origin)

</div>

## Why rcloc?

`rcloc` answers a simple question—*what is this codebase made of?*—without making you wait or decipher a wall of text.

- **Fast by default.** Native Rust, parallel counting, no interpreter and no runtime dependencies.
- **Broad language support.** 422 languages across 1,012 extensions, including content-based detection for ambiguous extensions.
- **Battle-tested counts.** Developed for count parity with the original [`cloc`](https://github.com/AlDanial/cloc) 2.11.
- **Made for humans and scripts.** A polished terminal table plus JSON, YAML, CSV, Markdown, XML and SQL.
- **More than a counter.** Compare directories or Git revisions, inspect individual files, scan archives and customize language definitions.
- **Works with real repositories.** Respects VCS file lists, filters generated/binary files and removes duplicate content.

In one local benchmark over `/usr/share/perl5`, `rcloc` completed in **0.025 s** versus **0.786 s** for the Perl implementation. Results depend on hardware, filesystem cache and corpus, so treat this as a data point—not a universal promise.

## Install

### Build from source

You need [Rust 1.75 or newer](https://www.rust-lang.org/tools/install).

```bash
git clone https://github.com/cdxy1/rcloc.git
cd rcloc
cargo build --release
./target/release/rcloc .
```

To put the binary in Cargo's bin directory:

```bash
cargo install --path crates/cloc-cli
rcloc .
```

## Usage

Count the current project:

```bash
rcloc .
```

See every file, not just language totals:

```bash
rcloc --by-file src
```

Count only selected languages and format large numbers:

```bash
rcloc --include-lang=Rust,TypeScript --thousands-delimiter=, .
```

Compare two source trees:

```bash
rcloc --diff release-1.0 release-2.0
```

Compare Git revisions:

```bash
rcloc --git-diff-all v1.0.0 v2.0.0
```

Use Git's tracked-file list so build artifacts and ignored files stay out:

```bash
rcloc --vcs=git .
```

Run `rcloc --help` for the complete option reference.

## Output formats

The default report is designed for a terminal. Structured formats keep their stable, decoration-free schemas for pipelines.

| Format | Flag | Great for |
|:--|:--|:--|
| Text | default | terminals and quick checks |
| JSON | `--json` | APIs, `jq`, CI dashboards |
| YAML | `--yaml` | configuration-oriented workflows |
| CSV | `--csv` | spreadsheets and data tools |
| Markdown | `--md` | pull requests and documentation |
| XML | `--xml` | legacy integrations and XSL |
| SQL | `--sql=FILE` | storing historical measurements |

Write any report directly to a file with `--report-file FILE`. Use `--quiet` to remove the text/structured header.

## Accuracy and compatibility

At the `parity-verified` tag, `rcloc` matched `cloc` 2.11 exactly across cloc's 345-file test corpus, this repository, `/usr/share/vim`, the Python 3.12 standard library and `/usr/share/perl5`.

```bash
git checkout parity-verified
```

That tag contains the upstream reference script and the comparison harnesses used for verification. The current language database lives in [`data/languages.json`](data/languages.json); malformed filter definitions fail tests at load time instead of silently producing incorrect counts.

Some surprising upstream counting behavior is intentionally preserved for compatibility—for example, comment scanners do not parse string literals unless the matching compatibility option is enabled. One deliberate improvement is support for non-ASCII filenames in `--git-diff-all`.

## Development

The workspace is intentionally split into small, reusable layers:

| Path | Responsibility |
|:--|:--|
| `crates/cloc-lang` | language definitions and comment-filter parsing |
| `crates/cloc-core` | walking, classification, counting, deduplication and diffs |
| `crates/cloc-cli` | command-line interface and report renderers |
| `data/languages.json` | canonical language and extension database |

Run the complete suite with:

```bash
cargo test --workspace
```

## License and origin

`rcloc` is an **independent Rust port** of [Al Danial's cloc](https://github.com/AlDanial/cloc). It is not affiliated with or endorsed by the upstream author.

The project is licensed under **GNU GPL 2.0 or later**, matching the upstream licensing requirements. The full terms are in [`LICENSE`](LICENSE), and attribution plus provenance details are recorded in [`NOTICE`](NOTICE). If you distribute a binary, GPL obligations include making the corresponding source available under the same license.

---

<div align="center">

**Less waiting. Better signal. Know your codebase.**

</div>
