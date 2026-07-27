# cloc-rs

A Rust port of [cloc](https://github.com/AlDanial/cloc), Al Danial's line
counter. It recognises 422 languages across 1012 file extensions and produces
the same counts as the original.

```sh
cargo build --release
./target/release/cloc-rs .
```

## Layout

| Path | What it is |
| --- | --- |
| `data/languages.json` | 1012 extensions, 422 languages, 22 ambiguous extensions. |
| `crates/cloc-lang` | Language definitions: what language a file is, how its comments are written. |
| `crates/cloc-core` | The counting engine: filters, comment scanners, diffing, file selection. |
| `crates/cloc-cli` | Command line interface and output formats. |

## Parity with the original

Developed against cloc 2.11, which lived in this repository as `cloc` until it
was removed. At the `parity-verified` tag the two agreed exactly on:

| Corpus | Files |
| --- | ---: |
| cloc's own test corpus (`tests/inputs`) | 345 |
| This repository | 912 |
| `/usr/share/vim` | 1896 |
| Python 3.12 standard library | 578 |
| `/usr/share/perl5` | 479 |

To re-check against the original, check out that tag: it still has the Perl
script, its 352-file test corpus, and the two comparison harnesses —
`tools/parity.py`, which compares per-file counts and groups mismatches by
language, and `tools/run_suite.py`, which runs the corpus.

```sh
git checkout parity-verified
```

`data/languages.json` was extracted mechanically from the Perl source by
`tools/extract_lang_defs.pl`, also at that tag. It is now the canonical copy;
`cloc-lang` resolves every filter specification into a typed enum when it
loads, so a malformed entry fails the tests rather than surfacing as a wrong
count later.

On `/usr/share/perl5` this build runs in 0.025 s against the original's
0.786 s.

## Testing

```sh
cargo test
```

The engine is covered by unit tests next to the code; `crates/cloc-cli/tests/cli.rs`
runs the binary end to end. Every count asserted was taken from the original
while it was still present.

## Notes on fidelity

Several behaviours of the original look like bugs and are reproduced anyway,
because matching cloc's numbers is the point. Each of these was found by a
count disagreeing, not by reading the source:

- `rm_comments_in_strings` is skipped unless `--strip-str-comments`, and
  `remove_inline` unless `--inline`. Together that is over a third of all
  filter steps in the table.
- Comment scanners do not understand string literals, so `"/* x */"` inside a
  string is stripped. An unterminated `/*` removes nothing at all, because the
  generated regexes need a closing delimiter.
- A `//` comment ends at the newline even if the line ends in a backslash, and
  it consumes that newline — invisible in ordinary C++ because the caller
  supplies two, but not in a language whose filter chain strips them first.
- Perl's `$1` survives a *failed* match, is cleared by a *successful* one with
  no capture groups, and is restored to undefined when `next` unwinds the loop
  body. The `remove_between_*` family reads `$1` across lines and depends on
  all three.
- Filters see each line with its newline attached, so `\s`, `.` and `[^x]` can
  match it. A lone `#` is an Imba comment only for that reason.
- Files that are not valid UTF-8 are decoded byte for byte rather than
  lossily, since cloc works on bytes and its patterns are written against them.
- Files whose content duplicates another are dropped, and among duplicates the
  survivor is chosen by *string* sort order, not path order.

One difference the other way: `--git-diff-all` fails in the original on a
repository containing a non-ASCII filename, whose quoted path it does not
parse. This port reads them.

## Licence

GPL-2.0-or-later, following the original.
