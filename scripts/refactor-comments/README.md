# refactor-comments

Mechanical strippers for AI-generated comment bloat in Rust codebases.
Used by the `/refactor-comments` slash command.

## Scripts

- `strip_banners.py` — decorative dividers (`═══`, `---`, `***`, `===`) and
  banner triples (divider / title / divider).
- `strip_doc_sections.py` — formal markdown subsections inside `///` blocks
  (`# Arguments`, `# Returns`, `# Examples`, `# Errors`, `# Panics`,
  `# Reference`, `# Note`, `# See also`). **Preserves `# Safety`** per
  kernel discipline. Also collapses empty doc-line runs.
- `strip_trivial_docs.py` — single-line `///` docs above `pub const`/`pub fn`
  declarations where the doc tokens just paraphrase the symbol name.

## Usage

Each script takes an optional REPO_ROOT argument (default: cwd).

```sh
python3 scripts/refactor-comments/strip_banners.py
python3 scripts/refactor-comments/strip_doc_sections.py
python3 scripts/refactor-comments/strip_trivial_docs.py

# Or scope to a single subdir:
python3 scripts/refactor-comments/strip_banners.py hwinit/
```

Each script reports `N files edited, M lines removed`.

## What they preserve

- Lines containing `# Safety`, `§`, `RFC`, `Spec`, `Datasheet`, `Vol`,
  `TODO`/`FIXME`/`HACK`, `PERF`, `SAFETY` tokens
- Lines with backtick-quoted identifiers
- License/copyright headers
- All code (zero code modifications)
- Comments inside `asm!` blocks (untouched — these are runtime asm)

## What they DON'T do

- Nuanced rewrites (e.g., turning a 5-line doc into a one-liner with
  preserved meaning) — that's what the sub-agents in the slash command do
- Spec/quirk note compression
- Whole-module renaming or restructuring

For the full pipeline (mechanical + nuanced sub-agent edits across
subsystems), use `/refactor-comments` instead of running scripts directly.
