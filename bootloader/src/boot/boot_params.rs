// Linux boot parameters (zero page)
// Reference: https://www.kernel.org/doc/html/latest/x86/boot.html#linux-boot-parameters-linux-boot-param-structure
// this was a pain in the ass to get right.....


#[repr(C, packed)]
pub struct LinuxBootParams {
    screen_info: ScreenInfo,        // 0x000
    apm_bios_info: ApmBiosInfo,     // 0x040
    _pad2: [u8; 4],                 // 0x054
    tboot_addr: u64,                // 0x058
    ist_info: IstInfo,              // 0x060
    acpi_rsdp_addr: u64,            // 0x070
    _pad3: [u8; 8],                 // 0x078
    hd0_info: [u8; 16],             // 0x080 (obsolete)
    hd1_info: [u8; 16],             // 0x090 (obsolete)
    sys_desc_table: SysDescTable,   // 0x0a0 (obsolete)
    olpc_ofw_header: OlpcOfwHeader, // 0x0b0
    ext_ramdisk_image: u32,         // 0x0c0
    ext_ramdisk_size: u32,          // 0x0c4
    ext_cmd_line_ptr: u32,          // 0x0c8
    _pad4: [u8; 112],               // 0x0cc
    cc_blob_address: u32,           // 0x13c
    edid_info: EdidInfo,            // 0x140
    efi_info: EfiInfo,              // 0x1c0
    alt_mem_k: u32,                 // 0x1e0
    scratch: u32,                   // 0x1e4
    e820_entries: u8,               // 0x1e8
    eddbuf_entries: u8,             // 0x1e9
    edd_mbr_sig_buf_entries: u8,    // 0x1ea
    kbd_status: u8,                 // 0x1eb
    secure_boot: u8,                // 0x1ec
    _pad5: [u8; 2],                 // 0x1ed
    sentinel: u8,                   // 0x1ef
    _pad6: [u8; 1],                 // 0x1f0
    hdr: SetupHeader,               // 0x1f1
    _pad7: [u8; 0x290 - 0x1f1 - core::mem::size_of::<SetupHeader>()],
    edd_mbr_sig_buffer: [u32; 16], // 0x290
    e820_table: [E820Entry; 128],  // 0x2d0
    _pad8: [u8; 48],               // 0xcd0
    eddbuf: [EddInfo; 6],          // 0xd00
    _pad9: [u8; 276],              // 0xeec
}

#[repr(C, packed)]
struct ScreenInfo {
    orig_x: u8,
    orig_y: u8,
    ext_mem_k: u16,
    orig_video_page: u16,
    orig_video_mode: u8,
    orig_video_cols: u8,
    flags: u8,
    unused2: u8,
    orig_video_ega_bx: u16,
    unused3: u16,
    orig_video_lines: u8,
    orig_video_isVGA: u8,
    orig_video_points: u16,
    lfb_width: u16,
    lfb_height: u16,
    lfb_depth: u16,
    lfb_base: u32,
    lfb_size: u32,
    cl_magic: u16,
    cl_offset: u16,
    lfb_linelength: u16,
    red_size: u8,
    red_pos: u8,
    green_size: u8,
    green_pos: u8,
    blue_size: u8,
    blue_pos: u8,
    rsvd_size: u8,
    rsvd_pos: u8,
    vesapm_seg: u16,
    vesapm_off: u16,
    pages: u16,
    vesa_attributes: u16,
    capabilities: u32,
    ext_lfb_base: u32,
    _reserved: [u8; 2],
}

#[repr(C, packed)]
struct ApmBiosInfo {
    version: u16,
    cseg: u16,
    offset: u32,
    cseg_16: u16,
    dseg: u16,
    flags: u16,
    cseg_len: u16,
    cseg_16_len: u16,
    dseg_len: u16,
}

#[repr(C, packed)]
struct IstInfo {
    signature: u32,
    command: u32,
    event: u32,
    perf_level: u32,
}

#[repr(C, packed)]
struct SysDescTable {
    length: u16,
    table: [u8; 14],
}

#[repr(C, packed)]
struct OlpcOfwHeader {
    ofw_magic: u32,
    ofw_version: u32,
    cif_handler: u32,
    irq_desc_table: u32,
}

#[repr(C, packed)]
struct EdidInfo {
    dummy: [u8; 128],
}

#[repr(C, packed)]
struct EfiInfo {
    efi_loader_signature: u32,
    efi_systab: u32,
    efi_memdesc_size: u32,
    efi_memdesc_version: u32,
    efi_memmap: u32,
    efi_memmap_size: u32,
    efi_systab_hi: u32,
    efi_memmap_hi: u32,
}

#[repr(C, packed)]
pub struct SetupHeader {
    pub setup_sects: u8,
    root_flags: u16,
    syssize: u32,
    ram_size: u16,
    vid_mode: u16,
    root_dev: u16,
    boot_flag: u16,
    jump: u16,
    pub header: u32,
    pub version: u16,
    realmode_swtch: u32,
    start_sys_seg: u16,
    kernel_version: u16,
    type_of_loader: u8,
    loadflags: u8,
    setup_move_size: u16,
    pub code32_start: u32,
    ramdisk_image: u32,
    ramdisk_size: u32,
    bootsect_kludge: u32,
    heap_end_ptr: u16,
    ext_loader_ver: u8,
    ext_loader_type: u8,
    cmd_line_ptr: u32,
    pub initrd_addr_max: u32,
    pub kernel_alignment: u32,
    pub relocatable_kernel: u8,
    min_alignment: u8,
    pub xloadflags: u16,
    pub cmdline_size: u32,
    hardware_subarch: u32,
    hardware_subarch_data: u64,
    payload_offset: u32,
    payload_length: u32,
    setup_data: u64,
    pub pref_address: u64,
    pub init_size: u32,
    pub handover_offset: u32,
    pub kernel_info_offset: u32,
}

#[repr(C, packed)]
struct E820Entry {
    addr: u64,
    size: u64,
    entry_type: u32,
}

#[repr(C, packed)]
struct EddInfo {
    device: u8,
    version: u8,
    interface_support: u16,
    legacy_max_cylinder: u16,
    legacy_max_head: u8,
    legacy_sectors_per_track: u8,
    params: [u8; 74],
}

impl LinuxBootParams {
    pub fn new() -> Self {
        let mut params: Self = unsafe { core::mem::zeroed() };
        params.sentinel = 0;
        params
    }

    pub fn set_cmdline(&mut self, cmdline_addr: u64, cmdline_len: u32) {
        self.hdr.cmd_line_ptr = cmdline_addr as u32;
        self.ext_cmd_line_ptr = (cmdline_addr >> 32) as u32;
        self.hdr.cmdline_size = cmdline_len;
    }

    pub fn set_ramdisk(&mut self, initrd_addr: u64, initrd_size: u64) {
        self.hdr.ramdisk_image = initrd_addr as u32;
        self.hdr.ramdisk_size = initrd_size as u32;
        self.ext_ramdisk_image = (initrd_addr >> 32) as u32;
        self.ext_ramdisk_size = (initrd_size >> 32) as u32;
    }

    pub fn ramdisk_info(&self) -> (u64, u64) {
        let addr = (self.ext_ramdisk_image as u64) << 32 | self.hdr.ramdisk_image as u64;
        let size = (self.ext_ramdisk_size as u64) << 32 | self.hdr.ramdisk_size as u64;
        (addr, size)
    }

    pub fn set_loader_type(&mut self, loader_type: u8) {
        self.hdr.type_of_loader = loader_type;
    }

    pub fn header(&self) -> &SetupHeader {
        &self.hdr
    }

    /// Copy setup header from kernel image to boot params
    /// CRITICAL: The kernel expects its own setup header back
    pub unsafe fn copy_setup_header(&mut self, kernel_setup_header: *const SetupHeader) {
        core::ptr::copy_nonoverlapping(kernel_setup_header, &mut self.hdr as *mut SetupHeader, 1);
    }

    /// Set basic video mode (text mode fallback)
    pub fn set_video_mode(&mut self) {
        self.screen_info.orig_video_mode = 0x03; // 80x25 text mode
        self.screen_info.orig_video_cols = 80;
        self.screen_info.orig_video_lines = 25;
        self.screen_info.orig_video_isVGA = 1;
    }

    /// Add E820 memory map entry
    /// CRITICAL: Kernel needs memory map to initialize
    pub fn add_e820_entry(&mut self, addr: u64, size: u64, entry_type: u32) {
        let idx = self.e820_entries as usize;
        if idx < 128 {
            self.e820_table[idx] = E820Entry {
                addr,
                size,
                entry_type,
            };
            self.e820_entries += 1;
        }
    }

    pub fn set_acpi_rsdp(&mut self, rsdp_addr: u64) {
        self.acpi_rsdp_addr = rsdp_addr;
    }

    pub fn set_alt_mem_k(&mut self, kilobytes: u32) {
        self.alt_mem_k = kilobytes;
    }

    pub fn set_secure_boot_flag(&mut self, value: u8) {
        self.secure_boot = value;
    }

    pub fn set_efi_info(
        &mut self,
        loader_signature: u32,
        systab: u64,
        memmap_addr: u64,
        memmap_size: u32,
        descriptor_size: u32,
        descriptor_version: u32,
    ) {
        self.efi_info.efi_loader_signature = loader_signature;
        self.efi_info.efi_systab = systab as u32;
        self.efi_info.efi_systab_hi = (systab >> 32) as u32;
        self.efi_info.efi_memmap = memmap_addr as u32;
        self.efi_info.efi_memmap_hi = (memmap_addr >> 32) as u32;
        self.efi_info.efi_memmap_size = memmap_size;
        self.efi_info.efi_memdesc_size = descriptor_size;
        self.efi_info.efi_memdesc_version = descriptor_version;
    }
}
