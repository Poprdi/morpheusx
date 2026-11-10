// host-side buddy tests. the pool is a real host buffer handed to the
// registry as a one-entry uefi map, so freenode writes land in memory we own.

use morpheus_hal_x86_64::memory::{AllocateType, MemoryError, MemoryRegistry, MemoryType};
use std::alloc::{alloc_zeroed, Layout};

const PAGE: u64 = 4096;
const POOL_PAGES: u64 = 4096; // 16 mib
const DESC: usize = 48;

const CONVENTIONAL: u32 = 7;
const BS_DATA: u32 = 4;
const MMIO: u32 = 11;

struct Pool {
    base: u64,
    pages: u64,
}

fn host_region(pages: u64) -> Pool {
    let bytes = (pages * PAGE) as usize;
    // natural alignment so the whole region is one buddy block
    let layout = Layout::from_size_align(bytes, bytes).unwrap();
    let p = unsafe { alloc_zeroed(layout) };
    assert!(!p.is_null());
    Pool {
        base: p as u64,
        pages,
    }
}

fn desc(buf: &mut Vec<u8>, ty: u32, phys: u64, pages: u64) {
    let mut d = [0u8; DESC];
    d[0..4].copy_from_slice(&ty.to_le_bytes());
    d[8..16].copy_from_slice(&phys.to_le_bytes());
    d[24..32].copy_from_slice(&pages.to_le_bytes());
    d[32..40].copy_from_slice(&8u64.to_le_bytes()); // wb
    buf.extend_from_slice(&d);
}

fn registry(map: &[u8]) -> Box<MemoryRegistry> {
    let mut reg = Box::new(MemoryRegistry::new());
    unsafe {
        reg.import_uefi_map(map.as_ptr(), map.len(), DESC, 1, 0, 0, 0, 0, &[]);
    }
    reg
}

fn simple() -> (Pool, Box<MemoryRegistry>) {
    let pool = host_region(POOL_PAGES);
    let mut map = Vec::new();
    desc(&mut map, CONVENTIONAL, pool.base, pool.pages);
    let reg = registry(&map);
    assert_eq!(reg.free_memory(), pool.pages * PAGE);
    (pool, reg)
}

fn any(reg: &mut MemoryRegistry, pages: u64) -> u64 {
    reg.allocate_pages(AllocateType::AnyPages, MemoryType::Allocated, pages)
        .unwrap()
}

#[test]
fn nonpow2_alloc_then_free_restores_free_pages() {
    let (_pool, mut reg) = simple();
    let before = reg.free_memory();
    let a = any(&mut reg, 3);
    assert_eq!(reg.free_memory(), before - 3 * PAGE);
    reg.free_pages(a, 3).unwrap();
    assert_eq!(reg.free_memory(), before);
}

#[test]
fn nonpow2_max_address_alloc_then_free_restores_free_pages() {
    let (pool, mut reg) = simple();
    let before = reg.free_memory();
    let limit = pool.base + pool.pages * PAGE - 1;
    let a = reg
        .allocate_pages(AllocateType::MaxAddress(limit), MemoryType::AllocatedDma, 5)
        .unwrap();
    assert_eq!(reg.free_memory(), before - 5 * PAGE);
    reg.free_pages(a, 5).unwrap();
    assert_eq!(reg.free_memory(), before);
}

#[test]
fn allocated_memory_drops_after_free() {
    let (_pool, mut reg) = simple();
    let a = any(&mut reg, 4);
    assert_eq!(reg.allocated_memory(), 4 * PAGE);
    reg.free_pages(a, 4).unwrap();
    assert_eq!(reg.allocated_memory(), 0);
}

#[test]
fn address_alloc_of_taken_page_fails() {
    let (_pool, mut reg) = simple();
    let a = any(&mut reg, 1);
    let free = reg.free_memory();
    let r = reg.allocate_pages(AllocateType::Address(a), MemoryType::AllocatedHeap, 1);
    assert_eq!(r, Err(MemoryError::AlreadyAllocated));
    assert_eq!(reg.free_memory(), free);
}

#[test]
fn address_alloc_outside_pool_fails() {
    let (_pool, mut reg) = simple();
    let free = reg.free_memory();
    // below the pool floor, never imported
    let r = reg.allocate_pages(
        AllocateType::Address(0x0100_0000),
        MemoryType::AllocatedHeap,
        1,
    );
    assert!(r.is_err());
    assert_eq!(reg.free_memory(), free);
}

#[test]
fn address_alloc_partial_conflict_rolls_back() {
    let (pool, mut reg) = simple();
    // take the third page of the first 4-page run
    let hole = pool.base + 2 * PAGE;
    reg.allocate_pages(AllocateType::Address(hole), MemoryType::Allocated, 1)
        .unwrap();
    let free = reg.free_memory();
    let r = reg.allocate_pages(
        AllocateType::Address(pool.base),
        MemoryType::AllocatedHeap,
        4,
    );
    assert_eq!(r, Err(MemoryError::AlreadyAllocated));
    assert_eq!(reg.free_memory(), free);
    // the pages the failed call touched must still be allocatable
    reg.allocate_pages(AllocateType::Address(pool.base), MemoryType::Allocated, 2)
        .unwrap();
    reg.allocate_pages(
        AllocateType::Address(pool.base + 3 * PAGE),
        MemoryType::Allocated,
        1,
    )
    .unwrap();
}

#[test]
fn max_address_alloc_splits_a_block_straddling_the_limit() {
    let (pool, mut reg) = simple();
    // whole pool is one block; the limit cuts through it
    let limit = pool.base + PAGE - 1;
    let a = reg
        .allocate_pages(AllocateType::MaxAddress(limit), MemoryType::AllocatedDma, 1)
        .unwrap();
    assert_eq!(a, pool.base);
}

#[test]
fn oversized_request_is_invalid_parameter() {
    let (_pool, mut reg) = simple();
    let r = reg.allocate_pages(AllocateType::AnyPages, MemoryType::Allocated, (1 << 26) + 1);
    assert_eq!(r, Err(MemoryError::InvalidParameter));
    let r = reg.allocate_pages(AllocateType::AnyPages, MemoryType::Allocated, u64::MAX);
    assert_eq!(r, Err(MemoryError::InvalidParameter));
}

#[test]
fn free_pages_rejects_overflowing_range() {
    let (pool, mut reg) = simple();
    let r = reg.free_pages(pool.base, u64::MAX);
    assert_eq!(r, Err(MemoryError::InvalidParameter));
}

#[test]
fn address_alloc_rejects_overflowing_range() {
    let (_pool, mut reg) = simple();
    let r = reg.allocate_pages(
        AllocateType::Address(0xFFFF_FFFF_FFFF_F000),
        MemoryType::Allocated,
        2,
    );
    assert_eq!(r, Err(MemoryError::InvalidParameter));
}

#[test]
fn type_query_of_remaining_region_survives_allocation() {
    let (pool, mut reg) = simple();
    let a = any(&mut reg, 1);
    assert_eq!(reg.memory_type_at(a), MemoryType::Allocated);
    let other = if a == pool.base { a + PAGE } else { pool.base };
    assert_eq!(reg.memory_type_at(other), MemoryType::Conventional);
    reg.free_pages(a, 1).unwrap();
    assert_eq!(reg.memory_type_at(a), MemoryType::Conventional);
}

#[test]
fn snapshot_does_not_grow_across_alloc_free_cycles() {
    let (_pool, mut reg) = simple();
    let (_, n0) = reg.get_memory_map();
    for _ in 0..2000 {
        let a = any(&mut reg, 1);
        reg.free_pages(a, 1).unwrap();
    }
    let (_, n1) = reg.get_memory_map();
    assert_eq!(n0, n1);
}

#[test]
fn reclaim_adds_boot_services_and_retags_snapshot() {
    let pool = host_region(POOL_PAGES);
    let bs = host_region(1024);
    let mut map = Vec::new();
    // keep ascending address order like a real map
    if pool.base < bs.base {
        desc(&mut map, CONVENTIONAL, pool.base, pool.pages);
        desc(&mut map, BS_DATA, bs.base, bs.pages);
    } else {
        desc(&mut map, BS_DATA, bs.base, bs.pages);
        desc(&mut map, CONVENTIONAL, pool.base, pool.pages);
    }
    let mut reg = registry(&map);
    assert_eq!(reg.free_memory(), pool.pages * PAGE);
    assert_eq!(reg.memory_type_at(bs.base), MemoryType::BootServicesData);

    unsafe { reg.reclaim_boot_services(&[]) };

    assert_eq!(reg.free_memory(), (pool.pages + bs.pages) * PAGE);
    assert_eq!(reg.memory_type_at(bs.base), MemoryType::Conventional);
    assert_eq!(reg.validate_free_lists(), 0);
}

#[test]
fn total_memory_counts_ram_not_mmio() {
    let pool = host_region(POOL_PAGES);
    let mut map = Vec::new();
    desc(&mut map, CONVENTIONAL, pool.base, pool.pages);
    desc(&mut map, MMIO, 0xE000_0000, 1 << 18); // 1 gib of device space
    let reg = registry(&map);
    assert_eq!(reg.total_memory(), pool.pages * PAGE);
}

#[test]
fn free_lists_stay_consistent_under_churn() {
    let (_pool, mut reg) = simple();
    let before = reg.free_memory();
    let mut live: Vec<(u64, u64)> = Vec::new();
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..4000 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let pages = 1 + (seed % 13);
        if seed & 1 == 0 || live.is_empty() {
            if let Ok(a) = reg.allocate_pages(AllocateType::AnyPages, MemoryType::Allocated, pages)
            {
                live.push((a, pages));
            }
        } else {
            let i = (seed as usize >> 8) % live.len();
            let (a, p) = live.swap_remove(i);
            reg.free_pages(a, p).unwrap();
        }
    }
    for (a, p) in live.drain(..) {
        reg.free_pages(a, p).unwrap();
    }
    assert_eq!(reg.free_memory(), before);
    assert_eq!(reg.allocated_memory(), 0);
    assert_eq!(reg.validate_free_lists(), 0);
}
