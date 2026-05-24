---
name: filesystem-design
description: |
  Design and implement HelixFS log-structured filesystem.
  Handle segment management, superblock consistency, and crash recovery.
author: MorpheusX Architecture Team
version: 2026.1
---

## Inputs Required
- HelixFS subsystem component
- Operation type (read/write/allocate/recover)
- Whether atomicity is required

## HelixFS Architecture

### Structure
- Circular 1 MB segments
- Dual superblock (primary + backup)
- Per-inode versions (append-only)
- Log-structured writes

### Segment Management
```rust
struct Segment {
    id: u32,
    base: PhysAddr,
    used: u32,
    checksum: u32,
}
```
- Segment allocation: round-robin with wear hints
- Segment reclaim: clean segment with valid checksum
- Write atomicity: full segment write or nothing

### Superblock
- Written first (before segment updates)
- Magic: `0x48454C58` ("HELX")
- Version: sequential counter
- Backup at offset +1 MB

### Crash Recovery
1. Read both superblocks
2. Pick highest version
3. Scan segments, validate checksums
4. Replay log entries
5. Truncate partial writes

## Quality Checklist
- [ ] Checksums on all segments
- [ ] Atomic superblock updates
- [ ] Crash-safe segment allocation
- [ ] No use-after-free after reclaim
- [ ] Bounded recursion in recovery

## References
- HelixFS design notes in `helix/`
- Log-structured filesystem papers