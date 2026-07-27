#!/usr/bin/env python3
"""Compare cloc-rs against the Perl cloc, file by file.

Both are run with --by-file --json over the same paths and their per-file
counts are matched up. Only the numbers are compared; formatting is allowed
to differ.

Usage:
    tools/parity.py PATH [PATH ...]
    tools/parity.py --lang Rust PATH      # only report files of one language
    tools/parity.py -v PATH               # show every mismatching file
"""

import argparse
import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PERL = ROOT / "cloc"
RUST = ROOT / "target" / "release" / "cloc-rs"

# Keys the report uses for bookkeeping rather than for a file.
META = {"header", "SUM"}


def run(cmd):
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        sys.exit(f"command failed: {' '.join(str(c) for c in cmd)}\n{proc.stderr}")
    try:
        return json.loads(proc.stdout or "{}")
    except json.JSONDecodeError as e:
        sys.exit(f"could not parse output of {' '.join(str(c) for c in cmd)}: {e}")


def by_file(report):
    """Map path -> (language, blank, comment, code)."""
    out = {}
    for key, value in report.items():
        if key in META:
            continue
        out[key] = (
            value.get("language", "?"),
            value.get("blank", 0),
            value.get("comment", 0),
            value.get("code", 0),
        )
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("paths", nargs="+")
    ap.add_argument("--lang", help="only report files of this language")
    ap.add_argument("-v", "--verbose", action="store_true",
                    help="list every mismatching file, not just a summary")
    ap.add_argument("--extra", nargs="*", default=[],
                    help="extra flags to pass to both implementations")
    args = ap.parse_args()

    if not RUST.exists():
        sys.exit(f"{RUST} not found; run: cargo build --release")

    common = ["--by-file", "--json", "--quiet", *args.extra, *args.paths]
    perl = by_file(run(["perl", str(PERL), *common]))
    rust = by_file(run([str(RUST), *common]))

    only_perl = sorted(set(perl) - set(rust))
    only_rust = sorted(set(rust) - set(perl))
    shared = sorted(set(perl) & set(rust))

    # Mismatches grouped by language, so a systematic filter bug shows up as
    # a cluster rather than a list of unrelated files.
    per_language = defaultdict(lambda: {"files": 0, "bad": 0, "delta": [0, 0, 0]})
    mismatches = []

    for path in shared:
        p, r = perl[path], rust[path]
        lang = p[0]
        if args.lang and lang != args.lang:
            continue
        stats = per_language[lang]
        stats["files"] += 1
        if p == r:
            continue
        stats["bad"] += 1
        for i in range(3):
            stats["delta"][i] += r[i + 1] - p[i + 1]
        mismatches.append((path, p, r))

    print(f"files: {len(shared)} compared, "
          f"{len(only_perl)} only in perl, {len(only_rust)} only in rust")

    if only_perl:
        print("\nclassified by perl but not by rust:")
        for path in only_perl[:20]:
            print(f"  {path}  ({perl[path][0]})")
        if len(only_perl) > 20:
            print(f"  ... and {len(only_perl) - 20} more")

    if only_rust:
        print("\nclassified by rust but not by perl:")
        for path in only_rust[:20]:
            print(f"  {path}  ({rust[path][0]})")
        if len(only_rust) > 20:
            print(f"  ... and {len(only_rust) - 20} more")

    bad_languages = {k: v for k, v in per_language.items() if v["bad"]}
    if bad_languages:
        print("\nmismatches by language (delta = rust - perl):")
        header = f"  {'language':<28}{'files':>7}{'bad':>6}" \
                 f"{'blank':>9}{'comment':>9}{'code':>9}"
        print(header)
        for lang, s in sorted(bad_languages.items(),
                              key=lambda kv: -abs(kv[1]["delta"][2])):
            d = s["delta"]
            print(f"  {lang:<28}{s['files']:>7}{s['bad']:>6}"
                  f"{d[0]:>+9}{d[1]:>+9}{d[2]:>+9}")

    if args.verbose and mismatches:
        print("\nper-file mismatches (blank/comment/code):")
        for path, p, r in mismatches:
            print(f"  {path}  [{p[0]}]")
            print(f"      perl {p[1]:>6} {p[2]:>6} {p[3]:>6}")
            print(f"      rust {r[1]:>6} {r[2]:>6} {r[3]:>6}")

    total_bad = sum(v["bad"] for v in per_language.values())
    total = sum(v["files"] for v in per_language.values())
    if total_bad == 0 and not only_perl and not only_rust:
        print("\nidentical")
        return 0
    print(f"\n{total_bad}/{total} files differ")
    return 1


if __name__ == "__main__":
    sys.exit(main())
