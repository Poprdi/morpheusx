// Linux kernel (bzImage) loader

const KERNEL_MAGIC: u32 = 0x53726448; // "HdrS"

#[derive(Debug)]
pub enum KernelError {
    InvalidMagic,
    InvalidFormat,
    UnsupportedVersion,
    AllocationFailed,
}

#[repr(C, packed)]
struct SetupHeader {
    setup_sects: u8,
    root_flags: u16,
    syssize: u32,
    ram_size: u16,
    vid_mode: u16,
    root_dev: u16,
    boot_flag: u16,
    jump: u16,
    header: u32,          // Magic "HdrS"
    version: u16,         // Boot protocol version
    realmode_swtch: u32,
    start_sys_seg: u16,
    kernel_version: u16,
    type_of_loader: u8,
    loadflags: u8,
    setup_move_size: u16,
    code32_start: u32,
    ramdisk_image: u32,
    ramdisk_size: u32,
    bootsect_kludge: u32,
    heap_end_ptr: u16,
    ext_loader_ver: u8,
    ext_loader_type: u8,
    cmd_line_ptr: u32,
    initrd_addr_max: u32,
    kernel_alignment: u32,
    relocatable_kernel: u8,
    min_alignment: u8,
    xloadflags: u16,
    cmdline_size: u32,
    hardware_subarch: u32,
    hardware_subarch_data: u64,
    payload_offset: u32,
    payload_length: u32,
    setup_data: u64,
    pref_address: u64,
    init_size: u32,
    handover_offset: u32,
}

pub struct KernelImage {
    setup_header: *const SetupHeader,
    kernel_base: *const u8,
    kernel_size: usize,
    protocol_version: u16,
}

impl KernelImage {
    // Parse a Linux bzImage from memory
    pub fn parse(data: &[u8]) -> Result<Self, KernelError> {
        if data.len() < 0x1000 {
            return Err(KernelError::InvalidFormat);
        }

        // Setup header is at offset 0x01f1
        let header_offset = 0x01f1;
        if data.len() < header_offset + core::mem::size_of::<SetupHeader>() {
            return Err(KernelError::InvalidFormat);
        }

        let setup_header = unsafe {
            &*(data.as_ptr().add(header_offset) as *const SetupHeader)
        };

        // Verify magic signature
        if setup_header.header != KERNEL_MAGIC {
            return Err(KernelError::InvalidMagic);
        }

        // Check protocol version (need at least 2.00)
        if setup_header.version < 0x0200 {
            return Err(KernelError::UnsupportedVersion);
        }

        // Calculate kernel offset
        // Setup sectors + 1 (boot sector) × 512 bytes
        let setup_size = ((setup_header.setup_sects as usize) + 1) * 512;
        
        if data.len() < setup_size {
            return Err(KernelError::InvalidFormat);
        }

        let kernel_base = unsafe { data.as_ptr().add(setup_size) };
        let kernel_size = data.len() - setup_size;

        Ok(KernelImage {
            setup_header,
            kernel_base,
            kernel_size,
            protocol_version: setup_header.version,
        })
    }

    pub fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    pub fn kernel_base(&self) -> *const u8 {
        self.kernel_base
    }

    pub fn kernel_size(&self) -> usize {
        self.kernel_size
    }

    pub fn is_relocatable(&self) -> bool {
        unsafe { (*self.setup_header).relocatable_kernel != 0 }
    }

    pub fn pref_address(&self) -> u64 {
        unsafe { (*self.setup_header).pref_address }
    }

    pub fn kernel_alignment(&self) -> u32 {
        unsafe { (*self.setup_header).kernel_alignment }
    }

    pub fn init_size(&self) -> u32 {
        unsafe { (*self.setup_header).init_size }
    }
    
    pub fn code32_start(&self) -> u32 {
        unsafe { (*self.setup_header).code32_start }
    }
    
    pub fn handover_offset(&self) -> u32 {
        unsafe { (*self.setup_header).handover_offset }
    }
    
    /// Get raw setup header bytes for detailed inspection
    pub fn setup_header_bytes(&self) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts(
                self.setup_header as *const u8,
                core::mem::size_of::<SetupHeader>()
            )
        }
    }
}
