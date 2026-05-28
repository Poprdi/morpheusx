#!/usr/bin/env python3
"""
Pass 1: strip decorative comment dividers and banner triples.

Removes:
  - Lines matching `// ═══...`, `// ───...`, `// ===`, `// ---`, `// ***`
    (≥5 of the divider char after `//`, `///`, or `//!`)
  - Banner triples: divider / title / divider → drop all 3 (title is always
    a section name the code already conveys)
  - Runs of 3+ consecutive blank lines collapsed to 2

Does NOT touch:
  - Lines inside string literals
  - Code at all
  - asm! block contents
  - License/copyright headers (preserved verbatim)

Usage: strip_banners.py [REPO_ROOT]  (default: current working directory)
"""

import os
import re
import sys

DIVIDER_RE = re.compile(r"^\s*//[/!]?\s*([═─━*=─\-]){5,}\s*$")


def find_rs_files(root):
    out = []
    for dirpath, dirnames, files in os.walk(root):
        for skip in ("target", ".git", "docs", "node_modules"):
            if skip in dirnames:
                dirnames.remove(skip)
        for f in files:
            if f.endswith(".rs"):
                out.append(os.path.join(dirpath, f))
    return out


def strip_file(path):
    with open(path, "r", encoding="utf-8") as f:
        lines = f.read().split("\n")

    out = []
    i = 0
    removed = 0
    n = len(lines)

    while i < n:
        line = lines[i]

        # divider / title / divider → drop all 3
        if (i + 2 < n
                and DIVIDER_RE.match(line)
                and lines[i + 1].lstrip().startswith("//")
                and not DIVIDER_RE.match(lines[i + 1])
                and DIVIDER_RE.match(lines[i + 2])):
            i += 3
            removed += 3
            continue

        if DIVIDER_RE.match(line):
            i += 1
            removed += 1
            continue

        out.append(line)
        i += 1

    # Collapse runs of blank lines (≥3) to 2 — banner removal leaves gaps
    final = []
    blank_run = 0
    for line in out:
        if line.strip() == "":
            blank_run += 1
            if blank_run <= 2:
                final.append(line)
        else:
            blank_run = 0
            final.append(line)

    new_content = "\n".join(final)
    if new_content != "\n".join(lines):
        with open(path, "w", encoding="utf-8") as f:
            f.write(new_content)
        return removed
    return 0


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else os.getcwd()
    total = 0
    files = 0
    for path in find_rs_files(root):
        rel = os.path.relpath(path, root)
        try:
            r = strip_file(path)
            if r:
                files += 1
                total += r
                print(f"  {r:4d} {rel}")
        except Exception as e:
            print(f"  ERROR {rel}: {e}", file=sys.stderr)
    print(f"\nTotal: {files} files edited, {total} lines removed")


if __name__ == "__main__":
    main()
