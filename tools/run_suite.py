#!/usr/bin/env python3
"""Run cloc's own test corpus against cloc-rs.

tests/inputs holds one file per language and tests/outputs holds the counts
cloc produces for it, as YAML. Each input is counted and compared with its
expected output.

Usage:
    tools/run_suite.py            # summary
    tools/run_suite.py -v         # list every failure
    tools/run_suite.py --perl     # compare against a live run of the Perl
                                  # cloc instead of the recorded YAML
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INPUTS = ROOT / "tests" / "inputs"
OUTPUTS = ROOT / "tests" / "outputs"
RUST = ROOT / "target" / "release" / "cloc-rs"
PERL = ROOT / "cloc"


def parse_expected(path):
    """Read a recorded YAML report into {language: (blank, comment, code)}.

    The files are flat two-level YAML, so a hand-rolled reader avoids a
    dependency and is entirely adequate.
    """
    result = {}
    section = None
    for raw in path.read_text(errors="replace").splitlines():
        if not raw.strip() or raw.startswith(("---", "#")):
            continue
        if not raw.startswith(" "):
            section = raw.split(":")[0].strip().strip("'\"")
            continue
        if section in (None, "header", "SUM"):
            continue
        key, _, value = raw.strip().partition(":")
        value = value.strip()
        if key in ("blank", "comment", "code") and value.lstrip("-").isdigit():
            entry = result.setdefault(section, {})
            entry[key] = int(value)
    return {
        lang: (v.get("blank", 0), v.get("comment", 0), v.get("code", 0))
        for lang, v in result.items()
    }


def counts_from(cmd):
    proc = subprocess.run(cmd, capture_output=True, text=True)
    try:
        report = json.loads(proc.stdout or "{}")
    except json.JSONDecodeError:
        return None
    return {
        lang: (v.get("blank", 0), v.get("comment", 0), v.get("code", 0))
        for lang, v in report.items()
        if lang not in ("header", "SUM")
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("-v", "--verbose", action="store_true")
    ap.add_argument("--perl", action="store_true",
                    help="compare against a live Perl cloc run, not the YAML")
    ap.add_argument("--filter", help="only inputs whose name contains this")
    args = ap.parse_args()

    if not RUST.exists():
        sys.exit(f"{RUST} not found; run: cargo build --release")

    passed, failed, skipped = 0, [], []

    for src in sorted(INPUTS.iterdir()):
        if not src.is_file():
            continue
        if args.filter and args.filter not in src.name:
            continue

        expected_path = OUTPUTS / f"{src.name}.yaml"
        if args.perl:
            expected = counts_from(
                ["perl", str(PERL), "--json", "--quiet", str(src)])
        elif expected_path.exists():
            expected = parse_expected(expected_path)
        else:
            skipped.append((src.name, "no expected output"))
            continue

        actual = counts_from([str(RUST), "--json", "--quiet", str(src)])
        if actual is None:
            failed.append((src.name, expected, "cloc-rs produced no JSON"))
            continue

        if actual == expected:
            passed += 1
        else:
            failed.append((src.name, expected, actual))

    total = passed + len(failed)
    print(f"{passed}/{total} inputs match" + (f", {len(skipped)} skipped" if skipped else ""))

    if failed:
        # Group by the language the expectation names, so one broken filter
        # reads as a cluster.
        by_language = {}
        for name, expected, actual in failed:
            lang = next(iter(expected), "?") if isinstance(expected, dict) else "?"
            by_language.setdefault(lang, []).append(name)
        print("\nfailures by language:")
        for lang, names in sorted(by_language.items(), key=lambda kv: -len(kv[1])):
            shown = ", ".join(names[:4])
            more = f" (+{len(names) - 4})" if len(names) > 4 else ""
            print(f"  {lang:<32} {len(names):>3}  {shown}{more}")

    if args.verbose:
        for name, expected, actual in failed:
            print(f"\n{name}")
            print(f"  expected {expected}")
            print(f"  actual   {actual}")

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
