# cloc-rs

A Rust port of [cloc](https://github.com/AlDanial/cloc), Al Danial's line counter.

The goal is feature parity with the Perl original (cloc 2.11, kept here as
`cloc` for reference) and identical counts. Output formatting may differ; the
numbers may not.

## Layout

| Path | What it is |
| --- | --- |
| `cloc` | The Perl original, 19.5k lines. Reference implementation and parity oracle. |
| `tools/extract_lang_defs.pl` | Lifts the language tables out of the Perl source into JSON. |
| `data/languages.json` | 1012 extensions, 422 languages, generated — do not hand-edit. |
| `crates/cloc-lang` | Language definitions: what language a file is, how its comments are written. |
| `crates/cloc-core` | The counting engine: filters, comment scanners, regex layer. |
| `crates/cloc-cli` | Command line interface and output formats. |

## Status

Counting works end to end and agrees with the original on the corpora below.
Not yet built: `--diff` and its relatives, git/VCS input, archive extraction,
`--sql` and `--html` output, and the `--*-lang-def` family.

| Corpus | Files | Mismatches |
| --- | ---: | ---: |
| This project | 21 | 0 |
| Three cloned repositories | 160 | 0 |
| `/usr/share/perl5` | 479 | 0 |
| `/usr/share/vim` | 1896 | 0 |
| Python 3.12 standard library | 578 | 0 |
| `/usr/share/doc`, `/etc`, git-core | 4452 | 2 |
| `tests/inputs` (cloc's own corpus) | 345 | 9 |

On `/usr/share/perl5` the Rust build runs in 0.025 s against the original's
0.786 s.

## Checking against the original

```sh
cargo build --release
tools/parity.py PATH...          # per-file counts, grouped by language
tools/run_suite.py --perl        # cloc's own test corpus
```

Both compare numbers only, not formatting. `parity.py` groups mismatches by
language so a systematic filter bug reads as a cluster rather than a list of
unrelated files.

## Regenerating the language tables

The tables are extracted rather than retyped, so a new upstream cloc can be
picked up mechanically:

```sh
perl tools/extract_lang_defs.pl cloc data/languages.json
cargo test -p cloc-lang    # size assertions catch a truncated extraction
```

`cloc-lang` resolves every filter spec into a typed enum at load time, so an
unknown filter name or a changed argument count fails the build's tests rather
than surfacing as a wrong count later.

## Notes on fidelity

Several behaviours of the original look like bugs and are reproduced anyway,
because matching cloc's numbers is the point:

- `rm_comments_in_strings` is skipped unless `--strip-str-comments`, and
  `remove_inline` unless `--inline`. Together that is over a third of all
  filter steps in the table.
- Comment scanners do not understand string literals, so `"/* x */"` inside a
  string is stripped.
- Perl's `$1` survives a *failed* match, is cleared by a *successful* one with
  no capture groups, and is restored to undefined when `next` unwinds the loop
  body. The `remove_between_*` family reads `$1` across lines and depends on
  all three.
- An unterminated `/*` removes nothing, because the generated regexes need a
  closing delimiter.
- A `//` comment ends at the newline even if the line ends in a backslash.
- Files whose content duplicates another are dropped, and among duplicates the
  survivor is chosen by *string* sort order, not path order.

## Licence

GPL-2.0-or-later, following the original.
