//! Staging admission control (spec §6): the RAM-budget chokepoint every userland
//! staged mount passes *under `STORAGE_LOCK`* before a page is allocated
//! (check + reserve + alloc must be atomic, else two procs jointly overcommit).
//! Kernel/privileged callers skip the policy caps (2–4) but honor the
//! physical-reserve check (5–6).

use crate::global::hal;
use morpheus_block_types::{MemBlockDevice, RawBlockDevice};
use morpheus_foundation::errno::{EINVAL, ENOMEM, ENOSPC};
use morpheus_hal_api::{AllocKind, MemoryType};

/// Largest single staging request (DoS bound on one mount).
pub const STAGE_SINGLE_MAX: u64 = 512 * 1024 * 1024;
/// Per-process total staged bytes.
pub const STAGE_PROC_CAP: u64 = 128 * 1024 * 1024;
/// Total staged across all userland mounts.
pub const STAGE_GLOBAL_CAP_FRACTION_NUM: u64 = 1;
pub const STAGE_GLOBAL_CAP_FRACTION_DEN: u64 = 2; // ≤ 50% RAM
/// Physical RAM that must stay free after staging.
pub const STAGE_RESERVE_MIN: u64 = 64 * 1024 * 1024;

/// Per-pid + global accounting. Guarded by `STORAGE_LOCK` (lives inside
/// `StorageGlobal`), so all reads/writes are already serialized.
pub struct StageAccount {
    total: u64,
    /// Sparse per-pid tally; pids are small and bounded by the process table.
    by_pid: alloc::vec::Vec<(u32, u64)>,
}

impl StageAccount {
    pub const fn new() -> Self {
        Self {
            total: 0,
            by_pid: alloc::vec::Vec::new(),
        }
    }

    fn pid_total(&self, pid: u32) -> u64 {
        self.by_pid
            .iter()
            .find(|(p, _)| *p == pid)
            .map(|(_, b)| *b)
            .unwrap_or(0)
    }

    fn add_pid(&mut self, pid: u32, bytes: u64) {
        if let Some(slot) = self.by_pid.iter_mut().find(|(p, _)| *p == pid) {
            slot.1 = slot.1.saturating_add(bytes);
        } else {
            self.by_pid.push((pid, bytes));
        }
        self.total = self.total.saturating_add(bytes);
    }

    fn sub_pid(&mut self, pid: u32, bytes: u64) {
        if let Some(slot) = self.by_pid.iter_mut().find(|(p, _)| *p == pid) {
            slot.1 = slot.1.saturating_sub(bytes);
        }
        self.total = self.total.saturating_sub(bytes);
    }
}

impl Default for StageAccount {
    fn default() -> Self {
        Self::new()
    }
}

/// A reserved + allocated RAM region for a staged mount, plus its budget charge.
pub struct StagedRam {
    pub phys_addr: u64,
    pub pages: u64,
    pub bytes: u64,
    pub owner_pid: u32,
}

fn page_size() -> u64 {
    let ps = hal().phys().page_size();
    if ps == 0 {
        4096
    } else {
        ps
    }
}

fn stage_reserve() -> u64 {
    let total = hal().phys().total_memory();
    // max(64 MiB, 12.5% of total RAM)
    let twelve_pct = total / 8;
    STAGE_RESERVE_MIN.max(twelve_pct)
}

fn stage_global_cap() -> u64 {
    let total = hal().phys().total_memory();
    total.saturating_mul(STAGE_GLOBAL_CAP_FRACTION_NUM) / STAGE_GLOBAL_CAP_FRACTION_DEN
}

/// Round `bytes` up to a page multiple, rejecting overflow.
fn page_round(bytes: u64) -> Result<u64, u64> {
    let ps = page_size();
    let plus = bytes.checked_add(ps - 1).ok_or(EINVAL)?;
    Ok((plus / ps) * ps)
}

/// Admission per spec §6. `privileged` callers (boot root) skip the policy caps
/// (2–4) but still honor the physical-reserve + alloc check (5–6). On success the
/// budget is charged and pages allocated; the caller wraps the returned region in
/// a `MemBlockDevice`. On failure nothing is charged. Returns an errno.
pub fn admit(
    account: &mut StageAccount,
    pid: u32,
    size: u64,
    privileged: bool,
) -> Result<StagedRam, u64> {
    // 1. round + overflow guard
    let s = page_round(size)?;
    if s == 0 {
        return Err(EINVAL);
    }

    if !privileged {
        // 2. single-request ceiling
        if s > STAGE_SINGLE_MAX {
            return Err(EINVAL);
        }
        // 3. per-proc quota
        let proc_after = account.pid_total(pid).checked_add(s).ok_or(EINVAL)?;
        if proc_after > STAGE_PROC_CAP {
            return Err(ENOSPC);
        }
        // 4. global cap
        let global_after = account.total.checked_add(s).ok_or(EINVAL)?;
        if global_after > stage_global_cap() {
            return Err(ENOSPC);
        }
    }

    // 5. physical reserve: leave enough free for heap/DMA/other procs
    let free = hal().phys().free_memory();
    let need = s.checked_add(stage_reserve()).ok_or(EINVAL)?;
    if free < need {
        return Err(ENOMEM);
    }

    // 6. allocate
    let ps = page_size();
    let pages = s / ps;
    let phys_addr = hal()
        .phys()
        .allocate_pages(AllocKind::AnyPages, MemoryType::Allocated, pages)
        .map_err(|_| ENOMEM)?;

    account.add_pid(pid, s);
    Ok(StagedRam {
        phys_addr,
        pages,
        bytes: s,
        owner_pid: pid,
    })
}

/// Release a staged region: free its pages and uncharge the budget. Used on
/// mount-unwind, umount, and reap.
pub fn release(account: &mut StageAccount, ram: &StagedRam) {
    let _ = hal().phys().free_pages(ram.phys_addr, ram.pages);
    account.sub_pid(ram.owner_pid, ram.bytes);
}

/// Build a `MemBlockDevice` over a staged region. The returned box must be kept
/// alive (in the device registry) for as long as the `RawBlockDevice` derived
/// from it; `RawBlockDevice::ctx` points into it.
///
/// # Safety
/// `ram.phys_addr` must be identity-mapped and live for the device's lifetime.
pub unsafe fn mem_device(
    ram: &StagedRam,
    block_size: u32,
) -> (alloc::boxed::Box<MemBlockDevice>, RawBlockDevice) {
    let size = (ram.pages * page_size()) as usize;
    // SAFETY: caller guarantees the region is identity-mapped and live.
    let mut mem = alloc::boxed::Box::new(MemBlockDevice::new(
        ram.phys_addr as *mut u8,
        size,
        block_size,
    ));
    let raw = MemBlockDevice::into_raw(&mut mem);
    (mem, raw)
}
