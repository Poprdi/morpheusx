//! ACPI MADT parser for SMP topology discovery.
//!
//! Scans legacy BIOS memory regions for the RSDP, follows RSDT/XSDT to
//! the MADT (signature "APIC"), and extracts enabled Local APIC entries.
//!
//! All memory is identity-mapped — no allocation, no page table walks,
//! just raw pointer arithmetic through firmware tables.

use crate::cpu::per_cpu::MAX_CPUS;
use crate::serial::{put_hex32, put_hex64, puts};

// ── RSDP ─────────────────────────────────────────────────────────────────

const RSDP_SIG: [u8; 8] = *b"RSD PTR ";

/// ACPI 1.0 RSDP (20 bytes).
#[repr(C, packed)]
struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32,
}

/// ACPI 2.0+ extended RSDP (36 bytes).
#[repr(C, packed)]
struct Rsdp2 {
    base: Rsdp,
    length: u32,
    xsdt_address: u64,
    extended_checksum: u8,
    _reserved: [u8; 3],
}

// ── SDT header ───────────────────────────────────────────────────────────

/// Common header for all ACPI System Description Tables.
#[repr(C, packed)]
struct SdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

const SDT_HEADER_SIZE: usize = core::mem::size_of::<SdtHeader>();

// ── MADT ─────────────────────────────────────────────────────────────────

const MADT_SIG: [u8; 4] = *b"APIC";

/// MADT fixed header (after SdtHeader).
#[repr(C, packed)]
struct MadtFixed {
    local_apic_addr: u32,
    flags: u32,
}

/// MADT entry type 0: Processor Local APIC.
#[repr(C, packed)]
struct MadtLocalApic {
    entry_type: u8, // 0
    length: u8,     // 8
    acpi_processor_uid: u8,
    apic_id: u8,
    flags: u32,
}

// MADT Local APIC flags
const LAPIC_ENABLED: u32 = 1 << 0;
const LAPIC_ONLINE_CAPABLE: u32 = 1 << 1;

// ── Result type ──────────────────────────────────────────────────────────

/// Set of AP LAPIC IDs discovered from the MADT.
pub struct ApLapicIds {
    pub ids: [u32; MAX_CPUS],
    pub count: usize,
}

impl ApLapicIds {
    const fn empty() -> Self {
        Self {
            ids: [0; MAX_CPUS],
            count: 0,
        }
    }
}

// ── RSDP scanning ────────────────────────────────────────────────────────

/// Scan a physical memory range for the RSDP signature.
/// Returns the physical address of the RSDP, or 0 on failure.
unsafe fn find_rsdp_in_range(start: u64, end: u64) -> u64 {
    // RSDP is always 16-byte aligned
    let mut addr = start & !0xF;
    while addr + 20 <= end {
        let ptr = addr as *const [u8; 8];
        if *ptr == RSDP_SIG {
            // validate checksum (first 20 bytes)
            let mut sum: u8 = 0;
            for i in 0..20 {
                sum = sum.wrapping_add(*((addr + i) as *const u8));
            }
            if sum == 0 {
                return addr;
            }
        }
        addr += 16;
    }
    0
}

/// Find the RSDP by scanning the traditional BIOS memory regions.
///
/// 1. EBDA (pointed to by BDA at physical 0x40E)
/// 2. Main BIOS area: 0xE0000 - 0xFFFFF
///
/// Works on UEFI systems because the firmware keeps the RSDP discoverable.
unsafe fn find_rsdp() -> u64 {
    // try EBDA first (BDA at 0x40E contains segment >> 4)
    let ebda_seg = *(0x40E as *const u16) as u64;
    if ebda_seg != 0 {
        let ebda_base = ebda_seg << 4;
        // EBDA is at most 1KiB
        let rsdp = find_rsdp_in_range(ebda_base, ebda_base + 1024);
        if rsdp != 0 {
            return rsdp;
        }
    }

    // scan main BIOS area
    find_rsdp_in_range(0xE0000, 0x100000)
}

// ── SDT traversal ────────────────────────────────────────────────────────

/// Find a table with the given 4-byte signature in the RSDT (32-bit pointers).
unsafe fn find_table_rsdt(rsdt_phys: u64, sig: &[u8; 4]) -> u64 {
    let header = rsdt_phys as *const SdtHeader;
    let total_len = (*header).length as usize;
    if total_len <= SDT_HEADER_SIZE {
        return 0;
    }

    let entry_count = (total_len - SDT_HEADER_SIZE) / 4;
    let entries = (rsdt_phys + SDT_HEADER_SIZE as u64) as *const u32;

    for i in 0..entry_count {
        let table_phys = *entries.add(i) as u64;
        if table_phys == 0 {
            continue;
        }
        let table_header = table_phys as *const SdtHeader;
        if (*table_header).signature == *sig {
            return table_phys;
        }
    }

    0
}

/// Find a table with the given 4-byte signature in the XSDT (64-bit pointers).
unsafe fn find_table_xsdt(xsdt_phys: u64, sig: &[u8; 4]) -> u64 {
    let header = xsdt_phys as *const SdtHeader;
    let total_len = (*header).length as usize;
    if total_len <= SDT_HEADER_SIZE {
        return 0;
    }

    let entry_count = (total_len - SDT_HEADER_SIZE) / 8;
    let entries = (xsdt_phys + SDT_HEADER_SIZE as u64) as *const u64;

    for i in 0..entry_count {
        let table_phys = *entries.add(i);
        if table_phys == 0 {
            continue;
        }
        let table_header = table_phys as *const SdtHeader;
        if (*table_header).signature == *sig {
            return table_phys;
        }
    }

    0
}

// ── MADT parsing ─────────────────────────────────────────────────────────

/// Parse the MADT and extract enabled Local APIC IDs, excluding the BSP.
unsafe fn parse_madt(madt_phys: u64, bsp_lapic_id: u32) -> ApLapicIds {
    let mut result = ApLapicIds::empty();

    let header = madt_phys as *const SdtHeader;
    let total_len = (*header).length as usize;
    let fixed_offset = SDT_HEADER_SIZE + core::mem::size_of::<MadtFixed>();

    if total_len <= fixed_offset {
        return result;
    }

    let madt_fixed = (madt_phys + SDT_HEADER_SIZE as u64) as *const MadtFixed;
    let reported_lapic_addr = (*madt_fixed).local_apic_addr;
    puts("[ACPI] MADT LAPIC addr: ");
    put_hex32(reported_lapic_addr);
    puts("\n");

    // walk variable-length entries after the fixed header
    let mut offset = fixed_offset;
    while offset + 2 <= total_len {
        let entry_ptr = (madt_phys + offset as u64) as *const u8;
        let entry_type = *entry_ptr;
        let entry_len = *entry_ptr.add(1) as usize;

        if entry_len < 2 || offset + entry_len > total_len {
            break; // malformed — bail
        }

        // type 0 = Processor Local APIC
        if entry_type == 0 && entry_len >= 8 {
            let lapic_entry = entry_ptr as *const MadtLocalApic;
            let apic_id = (*lapic_entry).apic_id as u32;
            let flags = (*lapic_entry).flags;

            // enabled or online-capable
            let usable = (flags & LAPIC_ENABLED) != 0 || (flags & LAPIC_ONLINE_CAPABLE) != 0;

            if usable && apic_id != bsp_lapic_id && result.count < MAX_CPUS {
                result.ids[result.count] = apic_id;
                result.count += 1;
            }
        }

        offset += entry_len;
    }

    result
}

// ── Public API ───────────────────────────────────────────────────────────

/// Discover AP LAPIC IDs from the ACPI MADT.
///
/// Returns a list of enabled LAPIC IDs (excluding `bsp_lapic_id`).
/// Returns an empty list if RSDP/MADT is not found (fall back to CPUID).
///
/// # Safety
/// Memory must be identity-mapped (true after UEFI ExitBootServices + our paging).
/// The ACPI tables must not have been overwritten. Call before reclaiming AcpiReclaim.
pub unsafe fn discover_ap_lapic_ids(bsp_lapic_id: u32) -> ApLapicIds {
    let rsdp_phys = find_rsdp();
    if rsdp_phys == 0 {
        puts("[ACPI] RSDP not found — MADT discovery unavailable\n");
        return ApLapicIds::empty();
    }

    puts("[ACPI] RSDP at ");
    put_hex64(rsdp_phys);
    puts("\n");

    let rsdp = rsdp_phys as *const Rsdp;
    let revision = (*rsdp).revision;

    // try XSDT first (ACPI 2.0+), fall back to RSDT
    let madt_phys = if revision >= 2 {
        let rsdp2 = rsdp_phys as *const Rsdp2;
        let xsdt = (*rsdp2).xsdt_address;
        if xsdt != 0 {
            puts("[ACPI] XSDT at ");
            put_hex64(xsdt);
            puts("\n");
            let found = find_table_xsdt(xsdt, &MADT_SIG);
            if found != 0 {
                found
            } else {
                // XSDT didn't have MADT, try RSDT
                find_table_rsdt((*rsdp).rsdt_address as u64, &MADT_SIG)
            }
        } else {
            find_table_rsdt((*rsdp).rsdt_address as u64, &MADT_SIG)
        }
    } else {
        find_table_rsdt((*rsdp).rsdt_address as u64, &MADT_SIG)
    };

    if madt_phys == 0 {
        puts("[ACPI] MADT not found in RSDT/XSDT\n");
        return ApLapicIds::empty();
    }

    puts("[ACPI] MADT at ");
    put_hex64(madt_phys);
    puts("\n");

    let result = parse_madt(madt_phys, bsp_lapic_id);

    puts("[ACPI] ");
    put_hex32(result.count as u32);
    puts(" APs in MADT\n");

    result
}
