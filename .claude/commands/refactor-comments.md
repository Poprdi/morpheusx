---
description: Refactor AI-generated verbose comments across Rust source. Mechanical strip + nuanced sub-agent passes. Reduces comment volume ~10x while preserving spec refs, SAFETY blocks, and hardware quirk notes.
argument-hint: "[scope] (default: whole repo; can be a subdir like 'hwinit/' or 'network/src/driver/')"
---

# /refactor-comments

You are running the MorpheusX comment-refactor pipeline. This is a multi-stage
workflow that cuts AI-generated comment bloat by roughly 10x while *lifting*
quality — replacing narration with kernel-developer brevity.

## Arguments

`$ARGUMENTS` — optional scope. Default is the whole repo (current working
directory). Can be a subdirectory like `hwinit/`, `network/src/driver/`,
`bootloader/src/`.

## Pipeline

### Stage 0 — Permissions check

Verify `.claude/settings.local.json` allows `Edit(**/*.rs)`. If not, write or
update it to include:

```json
{
  "permissions": {
    "allow": [
      "Edit(/home/explo1t/Documents/repos/morpheusx/**/*.rs)",
      "Bash(python3:*)",
      "Bash(find:*)",
      "Bash(grep:*)"
    ]
  }
}
```

Without this, sub-agents will get blanket Edit denials.

### Stage 1 — Baseline

Run `git diff --shortstat` to record the starting state. Note the count of
already-modified files (`git diff --name-only | wc -l`).

### Stage 2 — Mechanical passes (one Bash approval each)

Run the three strippers in `scripts/refactor-comments/` against the scope:

```sh
python3 scripts/refactor-comments/strip_banners.py [SCOPE]
python3 scripts/refactor-comments/strip_doc_sections.py [SCOPE]
python3 scripts/refactor-comments/strip_trivial_docs.py [SCOPE]
```

These handle:
- Decorative `═══` / `---` / `***` / `===` dividers and banner triples
- `# Arguments` / `# Returns` / `# Examples` / `# Errors` / `# Panics` /
  `# Reference` sections inside `///` doc blocks (keeps `# Safety`)
- Trivial single-line `/// X.` docs above `pub const NAME` / `pub fn name`
  where the doc just paraphrases the symbol name

Report combined stats after all three pass.

### Stage 3 — Dispatch nuanced sub-agents

After the mechanical passes, dispatch parallel sub-agents on the residual
files per subsystem. Each agent gets:
- The style guide below (verbatim)
- Its assigned subsystem directories
- An explicit `git diff --name-only`–based filter to skip already-touched
  files (avoids redoing work)

**Subsystem buckets** (adjust for the scope arg):

If `$ARGUMENTS` is empty or `.` (whole repo), use these 6 buckets:

1. **hwinit/** — kernel HAL + scheduler + syscalls + USB; **conservative**
   on `hwinit/src/usb/` (spec-backed comments are load-bearing)
2. **network/** — drivers, smoltcp stack, HTTP, USB MSD
3. **core/ + helix/ + bootloader/** — disk formats, log-FS, UEFI entry
4. **ui/ + display/ + compd/** — graphics + compositor
5. **iso9660/ + persistent/** — ISO reader + PE storage
6. **gfx3d/ + shell/ + shelld/ + libmorpheus/ + settings/ + tests/ + misc** —
   3D math, shell, userspace API, settings UI, integration tests

If `$ARGUMENTS` is a specific subdirectory, dispatch one or two agents
appropriately scoped (don't over-shard).

Dispatch ALL agents in a SINGLE message with multiple `Agent` tool calls so
they run in true parallel. Use `run_in_background: true`.

### Stage 4 — Wait, consolidate, report

Each agent reports `N files edited, M comments removed`. Wait for all to
complete (notifications arrive automatically — do not poll).

Final report:
- `git diff --shortstat`
- Per-subsystem file counts
- Net comment-line reduction (target: 5-10x reduction)
- Anything skipped or blocked

**Never commit.** User reviews via `git diff`.

---

## STYLE GUIDE (verbatim — embed in every sub-agent prompt)

### REMOVE

- Comments that restate the code (`// probe controller`, `// loop over ports`,
  `// command ring`)
- Banner blocks: `===`, `---`, `***`, `═══`
- "This function does X, Y, Z" docstrings that paraphrase the signature
- Tutorial explanations of standard idioms (`// Iterate through ...`,
  `// Check if ...`)
- Multi-paragraph doc comments where one sentence does the job
- Vague TODO/FIXME without specifics (keep precise ones)
- Closing-block markers (`// end of function`), decorative section dividers
- File/line refs in doc text that will rot ("mostly used by hub.rs",
  "see foo.rs line 42")
- Marketing language: "robust", "elegant", "comprehensive",
  "for clarity", "for readability"
- "# Examples" sections that show only trivial usage
- Param/return lists that repeat what `&self` and the type already say

### KEEP / SHARPEN

- Spec references with section numbers (xHCI §5.4.5, AMD64 Vol 2 §3.2,
  UEFI 2.10 §, ISO 9660 §, RFC numbers, datasheet citations)
- Why-this-hack-exists notes: hardware errata, firmware quirks, past bugs
  (Intel xHCI quirks, PSCEC, BIOS handoff weirdness, OVMF scrub, VirtIO
  avail->idx desync, CR0.WP & FreeNode-in-PT-page, etc.)
- Non-obvious invariants, ordering constraints, memory barriers,
  cycle-bit discipline
- SAFETY comments on unsafe blocks (required by project rules) — terse
- Why constants have non-obvious values (DMA offsets, ring sizes, magic
  numbers from spec)
- Why a function has odd attributes (`#[inline(never)]`, `#[no_mangle]`,
  `#[link_section]`)

### TONE

- Brief. Technical. One line where possible.
- Imperative or declarative — not "this code will..."
- Dry kernel-dev tone; occasionally wry/bitter about hardware/firmware
  misbehavior is fine (sparingly — don't force it)
- No emojis. No exclamation points.

### DOC COMMENTS (`///`, `//!`)

- One sentence preferred; max 2-3 lines on public API
- Module headers (`//!`): one sentence of purpose
- Skip param/return lists unless truly non-obvious

### DO NOT TOUCH

- License/copyright headers at file top
- String literals (those are runtime output, not comments)
- Comments inside `asm!` blocks (runtime assembly)
- Code itself — zero code changes, not even whitespace
- `#[derive]`, `#[cfg]`, other attributes
- Test function names
- Files in `git diff --name-only` (already done)

### WHEN IN DOUBT — KEEP IT.

Err toward preservation. If a comment looks like it might carry load-bearing
context (spec ref, errata, ordering rule), do not remove it.

---

## Example transforms

BEFORE:
```rust
/// This function probes the xHCI controller by reading the capability
/// length register at the MMIO base address. If the read returns zero
/// then the controller is considered dead and we return an error.
///
/// # Arguments
/// * `mmio_base` - The MMIO base address of the controller
///
/// # Returns
/// The capability length on success, or an error on failure.
pub fn probe(mmio_base: u64) -> Result<u8, XhciError> {
    // Read the capability length from the controller
    let cap = read32(mmio_base);
    // If zero, the BAR isn't mapped or the device is dead
    if cap == 0 {
        return Err(XhciError::ProbeFailed);
    }
    Ok((cap & 0xFF) as u8)
}
```

AFTER:
```rust
/// Returns CAPLENGTH. Zero read-back means dead BAR.
pub fn probe(mmio_base: u64) -> Result<u8, XhciError> {
    let cap = read32(mmio_base);
    if cap == 0 {
        return Err(XhciError::ProbeFailed);
    }
    Ok((cap & 0xFF) as u8)
}
```

BEFORE:
```rust
//===============================================================
// SECTION: VirtIO Net Driver
//===============================================================

//! This module implements a VirtIO network driver for the MorpheusX
//! exokernel. It provides a complete implementation of the VirtIO
//! 1.1 specification for network devices, including packet send and
//! receive, queue management, and feature negotiation.
```

AFTER:
```rust
//! VirtIO 1.1 net driver: tx/rx queues, feature negotiation.
```

---

## Final notes

- Sub-agents inherit the project's permission profile; if Edit gets denied,
  stop and instruct the user to widen `.claude/settings.local.json`.
- Pre-existing clippy errors (like the `morpheus-ping` unsafe-block) are
  NOT this command's job to fix; ignore them.
- If `cargo check` is desired post-refactor, use
  `CARGO_TARGET_DIR=/tmp/morpheusx-check` (the user's `target/` has
  root-owned files).
- This command never commits.
