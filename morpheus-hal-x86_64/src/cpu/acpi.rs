//! ACPI MADT parser for SMP topology. Walks RSDP -> RSDT/XSDT -> MADT;
//! ACPI tables assumed identity-mapped.

use crate::cpu::per_cpu::MAX_CPUS;
use crate::serial::{put_hex32, puts};
use core::sync::atomic::{AtomicU64, Ordering};

#[allow(dead_code)]
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

/// MADT entry type 9: Processor Local x2APIC.
#[repr(C, packed)]
struct MadtLocalX2Apic {
    entry_type: u8, // 9
    length: u8,     // 16
    _reserved: u16,
    x2apic_id: u32,
    flags: u32,
    _acpi_processor_uid: u32,
}

const LAPIC_ENABLED: u32 = 1 << 0;
const LAPIC_ONLINE_CAPABLE: u32 = 1 << 1;

/// AP LAPIC IDs discovered from the MADT.
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

#[inline(always)]
fn push_apic_id(result: &mut ApLapicIds, apic_id: u32, bsp_lapic_id: u32) {
    if apic_id == bsp_lapic_id || result.count >= MAX_CPUS {
        return;
    }
    for i in 0..result.count {
        if result.ids[i] == apic_id {
            return;
        }
    }
    result.ids[result.count] = apic_id;
    result.count += 1;
}

static RSDP_PHYS_OVERRIDE: AtomicU64 = AtomicU64::new(0);

pub fn set_rsdp_phys(rsdp_phys: u64) {
    RSDP_PHYS_OVERRIDE.store(rsdp_phys, Ordering::Release);
}

/// Legacy BIOS RSDP scan (diagnostics only). RSDP is 16-byte aligned;
/// first 20 bytes must sum to zero.
#[allow(dead_code)]
unsafe fn find_rsdp_in_range(start: u64, end: u64) -> u64 {
    let mut addr = start & !0xF;
    while addr + 20 <= end {
        let ptr = addr as *const [u8; 8];
        if *ptr == RSDP_SIG {
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

/// Scan EBDA (via BDA seg at 0x40E) then main BIOS area 0xE0000..0x100000.
#[allow(dead_code)]
unsafe fn find_rsdp() -> u64 {
    let ebda_seg = *(0x40E as *const u16) as u64;
    if ebda_seg != 0 {
        let ebda_base = ebda_seg << 4;
        let rsdp = find_rsdp_in_range(ebda_base, ebda_base + 1024);
        if rsdp != 0 {
            return rsdp;
        }
    }

    find_rsdp_in_range(0xE0000, 0x100000)
}

/// Sum every byte of `length` and check result is 0 (ACPI spec).
unsafe fn validate_sdt_checksum(table_phys: u64) -> bool {
    let header = table_phys as *const SdtHeader;
    let length = core::ptr::read_unaligned(core::ptr::addr_of!((*header).length)) as usize;
    if !(SDT_HEADER_SIZE..=0x10_0000).contains(&length) {
        return false;
    }
    let mut sum: u8 = 0;
    for i in 0..length {
        sum = sum.wrapping_add(*((table_phys + i as u64) as *const u8));
    }
    sum == 0
}
/// RSDT lookup (32-bit pointers).
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

/// XSDT lookup (64-bit pointers).
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

/// Extract enabled Local APIC IDs, excluding the BSP.
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

    // Cross-check MADT LAPIC addr vs IA32_APIC_BASE; mismatch = firmware lying.
    let probed_base = crate::cpu::apic::lapic_base() as u32;
    if reported_lapic_addr != 0 && reported_lapic_addr != probed_base {
        puts("[ACPI] WARN: MADT LAPIC addr ");
        put_hex32(reported_lapic_addr);
        puts(" != probed base ");
        put_hex32(probed_base);
        puts("\n");
    }

    let mut offset = fixed_offset;
    while offset + 2 <= total_len {
        let entry_ptr = (madt_phys + offset as u64) as *const u8;
        let entry_type = *entry_ptr;
        let entry_len = *entry_ptr.add(1) as usize;

        if entry_len < 2 || offset + entry_len > total_len {
            break;
        }

        // type 0: Processor Local APIC (xAPIC)
        if entry_type == 0 && entry_len >= 8 {
            let lapic_entry = entry_ptr as *const MadtLocalApic;
            let apic_id = (*lapic_entry).apic_id as u32;
            let flags = (*lapic_entry).flags;

            let usable = (flags & LAPIC_ENABLED) != 0 || (flags & LAPIC_ONLINE_CAPABLE) != 0;

            if usable {
                push_apic_id(&mut result, apic_id, bsp_lapic_id);
            }
        }

        // type 9: Processor Local x2APIC
        if entry_type == 9 && entry_len >= 16 {
            let x2_entry = entry_ptr as *const MadtLocalX2Apic;
            let apic_id = (*x2_entry).x2apic_id;
            let flags = (*x2_entry).flags;

            let usable = (flags & LAPIC_ENABLED) != 0 || (flags & LAPIC_ONLINE_CAPABLE) != 0;
            if usable {
                push_apic_id(&mut result, apic_id, bsp_lapic_id);
            }
        }

        offset += entry_len;
    }

    result
}

/// Static backing for the HAL trait method's `&'static [u32]` return slice.
/// Per the trait contract, each call overwrites the previous result. Access
/// is single-threaded (BSP, pre-SMP).
static mut DISCOVERED_LAPIC_IDS: [u32; MAX_CPUS] = [0; MAX_CPUS];
static mut DISCOVERED_LAPIC_COUNT: usize = 0;

/// HAL trait entry point: parse MADT into the static buffer and return a slice.
///
/// `rsdp_phys` overrides any cached pointer if non-zero, mirroring the legacy
/// `set_rsdp_phys` + `discover_ap_lapic_ids(bsp_lapic_id)` two-call sequence
/// the `start_aps` shim used to issue.
///
/// # Safety
/// BSP, single-threaded. ACPI tables identity-mapped + unreclaimed. Caller
/// must not retain the previous returned slice across another call.
pub unsafe fn discover_ap_lapic_ids_static(rsdp_phys: u64) -> &'static [u32] {
    if rsdp_phys != 0 {
        set_rsdp_phys(rsdp_phys);
    }
    let bsp_lapic_id = crate::cpu::apic::read_lapic_id();
    let result = discover_ap_lapic_ids(bsp_lapic_id);

    DISCOVERED_LAPIC_COUNT = result.count;
    // indexes two parallel slices; index form keeps the copy obviously correct
    #[allow(clippy::needless_range_loop)]
    for i in 0..result.count {
        DISCOVERED_LAPIC_IDS[i] = result.ids[i];
    }
    &DISCOVERED_LAPIC_IDS[..DISCOVERED_LAPIC_COUNT]
}

/// Discover AP LAPIC IDs from MADT, excluding the BSP. Empty list = caller
/// must fall back to CPUID.
///
/// # Safety
/// ACPI tables must be identity-mapped and unreclaimed.
pub unsafe fn discover_ap_lapic_ids(bsp_lapic_id: u32) -> ApLapicIds {
    let rsdp_phys = RSDP_PHYS_OVERRIDE.load(Ordering::Acquire);
    if rsdp_phys == 0 {
        crate::serial::log_warn(
            "ACPI",
            760,
            "UEFI RSDP pointer unavailable; MADT discovery unavailable",
        );
        return ApLapicIds::empty();
    }

    let _ = rsdp_phys;

    let rsdp = rsdp_phys as *const Rsdp;
    let revision = (*rsdp).revision;

    // XSDT (ACPI 2.0+) preferred; fall back to RSDT
    let madt_phys = if revision >= 2 {
        let rsdp2 = rsdp_phys as *const Rsdp2;
        let xsdt = (*rsdp2).xsdt_address;
        if xsdt != 0 {
            if !validate_sdt_checksum(xsdt) {
                crate::serial::log_warn("ACPI", 764, "XSDT checksum invalid; trying RSDT");
                let rsdt = (*rsdp).rsdt_address as u64;
                if rsdt != 0 && validate_sdt_checksum(rsdt) {
                    find_table_rsdt(rsdt, &MADT_SIG)
                } else {
                    0
                }
            } else {
                let found = find_table_xsdt(xsdt, &MADT_SIG);
                if found != 0 {
                    found
                } else {
                    find_table_rsdt((*rsdp).rsdt_address as u64, &MADT_SIG)
                }
            }
        } else {
            let rsdt = (*rsdp).rsdt_address as u64;
            if rsdt != 0 && validate_sdt_checksum(rsdt) {
                find_table_rsdt(rsdt, &MADT_SIG)
            } else {
                0
            }
        }
    } else {
        let rsdt = (*rsdp).rsdt_address as u64;
        if rsdt != 0 && validate_sdt_checksum(rsdt) {
            find_table_rsdt(rsdt, &MADT_SIG)
        } else {
            0
        }
    };

    if madt_phys == 0 {
        crate::serial::log_warn("ACPI", 761, "MADT not found in RSDT/XSDT");
        return ApLapicIds::empty();
    }

    if !validate_sdt_checksum(madt_phys) {
        crate::serial::log_warn("ACPI", 765, "MADT checksum invalid; skipping AP discovery");
        return ApLapicIds::empty();
    }

    let result = parse_madt(madt_phys, bsp_lapic_id);

    if result.count == 0 {
        crate::serial::log_warn("ACPI", 763, "MADT parsed but no usable AP LAPIC entries");
    }

    result
}
