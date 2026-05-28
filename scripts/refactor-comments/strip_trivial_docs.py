#!/usr/bin/env python3
"""
Pass 3: strip trivial per-item single-line `///` docs.

Heuristic: a `/// X.` line is "trivial" when ALL of:
  - It is exactly one `///` line (no continuation above or below)
  - It is followed by `pub const NAME: ...` or `pub static NAME: ...` or
    `pub [unsafe] fn name(...)` inside an `extern` block
  - Doc text is ≤8 words
  - Doc tokens share ≥50% with the item name (loose match)
  - Doc text contains no safety/spec/RFC/datasheet tokens

Strict checks ensure spec refs ("§5.4.5"), errata notes ("e1000_disable_ulp"),
and SAFETY comments survive.

Usage: strip_trivial_docs.py [REPO_ROOT]
"""

import os
import re
import sys

SINGLE_DOC = re.compile(r"^(\s*)///\s*(.+?)\s*$")
PUB_CONST = re.compile(r"^\s*pub\s+(?:const|static)\s+([A-Z_][A-Z0-9_]*)\s*:")
EXTERN_FN = re.compile(r"^\s*pub\s+(?:unsafe\s+)?fn\s+([a-z_][a-zA-Z0-9_]*)\s*\(")
SAFETY_TOKENS = (
    "#", "§", "spec", "Spec", "SPEC", "RFC", "datasheet", "Datasheet",
    "Vol ", "TODO", "XXX", "FIXME", "HACK", "Linux", "kernel", "errata",
    "PERF", "SAFETY",
)


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


def shares_token(doc_text, name):
    name_l = name.lower().replace("_", "")
    tokens = re.split(r"[^a-zA-Z0-9]+", doc_text.lower())
    tokens = [t for t in tokens if len(t) >= 3]
    if not tokens:
        return False
    matches = sum(1 for t in tokens if t in name_l)
    return matches >= max(1, len(tokens) // 2)


def is_trivial(doc_text):
    if any(tok in doc_text for tok in SAFETY_TOKENS):
        return False
    if "`" in doc_text:
        return False
    return len(doc_text.split()) <= 8


def process_file(path):
    with open(path, "r", encoding="utf-8") as f:
        content = f.read()
    lines = content.split("\n")
    orig = lines[:]
    removed = 0

    out, i, n = [], 0, len(lines)
    while i < n:
        m = SINGLE_DOC.match(lines[i])
        if m:
            doc_text = m.group(2)
            prev_is_doc = len(out) > 0 and SINGLE_DOC.match(out[-1]) is not None
            next_idx = i + 1
            while next_idx < n and lines[next_idx].lstrip().startswith("#["):
                next_idx += 1
            next_line = lines[next_idx] if next_idx < n else ""
            next_is_doc = SINGLE_DOC.match(next_line) is not None

            if not prev_is_doc and not next_is_doc:
                target_name = None
                cm = PUB_CONST.match(next_line)
                fm = EXTERN_FN.match(next_line)
                if cm:
                    target_name = cm.group(1)
                elif fm:
                    target_name = fm.group(1)

                if target_name and is_trivial(doc_text) and shares_token(doc_text, target_name):
                    removed += 1
                    i += 1
                    continue

        out.append(lines[i])
        i += 1

    if out != orig:
        with open(path, "w", encoding="utf-8") as f:
            f.write("\n".join(out))
        return removed
    return 0


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else os.getcwd()
    total, files = 0, 0
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
