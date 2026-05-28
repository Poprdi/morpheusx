#!/usr/bin/env python3
"""
Pass 2: strip formal markdown subsections inside `///` doc blocks.

Removes these section headings + their bodies:
  # Arguments, # Returns, # Examples, # Errors, # Panics,
  # Reference, # References, # See also, # Note, # Notes

Keeps:
  # Safety  (kernel discipline — every unsafe fn doc retains this)

Also:
  - Collapses runs of 2+ consecutive empty `///` / `//!` lines to 1
  - Removes trailing empty doc lines that precede non-doc code

Section terminates at next `/// #` heading or first non-doc line.

Usage: strip_doc_sections.py [REPO_ROOT]
"""

import os
import re
import sys

STRIP_HEADINGS = (
    "Arguments", "Returns", "Examples", "Example", "Errors",
    "Panics", "Reference", "References", "See also", "Note", "Notes",
)

HEADING_RE = re.compile(r"^\s*//[/!]\s*#\s+([A-Za-z][A-Za-z ]*)\s*$")
DOC_LINE_RE = re.compile(r"^\s*//[/!](.*)$")
EMPTY_DOC_RE = re.compile(r"^\s*//[/!]\s*$")


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


def is_doc_line(line):
    return bool(DOC_LINE_RE.match(line))


def strip_sections(lines):
    out, i, removed, n = [], 0, 0, len(lines)
    while i < n:
        m = HEADING_RE.match(lines[i])
        if m and m.group(1).strip() in STRIP_HEADINGS:
            removed += 1
            i += 1
            while i < n and is_doc_line(lines[i]):
                if HEADING_RE.match(lines[i]):
                    break
                removed += 1
                i += 1
            continue
        out.append(lines[i])
        i += 1
    return out, removed


def collapse_blank_doc(lines):
    out, removed, blank_run = [], 0, 0
    for line in lines:
        if EMPTY_DOC_RE.match(line):
            blank_run += 1
            if blank_run == 1:
                out.append(line)
            else:
                removed += 1
        else:
            blank_run = 0
            out.append(line)
    return out, removed


def trim_trailing_blank_doc(lines):
    out, removed, i, n = [], 0, 0, len(lines)
    while i < n:
        if EMPTY_DOC_RE.match(lines[i]):
            j = i + 1
            while j < n and (EMPTY_DOC_RE.match(lines[j]) or lines[j].lstrip().startswith("#[")):
                j += 1
            if j < n and not is_doc_line(lines[j]):
                removed += 1
                i += 1
                continue
        out.append(lines[i])
        i += 1
    return out, removed


def process_file(path):
    with open(path, "r", encoding="utf-8") as f:
        content = f.read()
    lines = content.split("\n")
    orig = lines[:]

    lines, r1 = strip_sections(lines)
    lines, r2 = collapse_blank_doc(lines)
    lines, r3 = trim_trailing_blank_doc(lines)

    total = r1 + r2 + r3
    if lines != orig:
        with open(path, "w", encoding="utf-8") as f:
            f.write("\n".join(lines))
        return total
    return 0


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else os.getcwd()
    total = 0
    files = 0
    for path in find_rs_files(root):
        rel = os.path.relpath(path, root)
        try:
            r = process_file(path)
            if r:
                files += 1
                total += r
                print(f"  {r:4d} {rel}")
        except Exception as e:
            print(f"  ERROR {rel}: {e}", file=sys.stderr)
    print(f"\nTotal: {files} files edited, {total} lines removed")


if __name__ == "__main__":
    main()
