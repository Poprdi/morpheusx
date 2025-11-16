// Linux kernel (bzImage) loader

use super::boot_params::SetupHeader;

const KERNEL_MAGIC: u32 = 0x53726448; // "HdrS"
const XLF_CAN_BE_LOADED_ABOVE_4G: u16 = 1 << 1;
const XLF_EFI_HANDOVER_64: u16 = 1 << 3;

#[derive(Debug)]
pub enum KernelError {
    InvalidMagic,
    InvalidFormat,
    UnsupportedVersion,
    AllocationFailed,
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

        let setup_header = unsafe { &*(data.as_ptr().add(header_offset) as *const SetupHeader) };

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

    pub fn xloadflags(&self) -> u16 {
        unsafe { (*self.setup_header).xloadflags }
    }

    pub fn can_load_above_4g(&self) -> bool {
        (self.xloadflags() & XLF_CAN_BE_LOADED_ABOVE_4G) != 0
    }

    pub fn supports_efi_handover_64(&self) -> bool {
        (self.xloadflags() & XLF_EFI_HANDOVER_64) != 0 && self.handover_offset() != 0
    }

    pub fn initrd_addr_max(&self) -> u32 {
        unsafe { (*self.setup_header).initrd_addr_max }
    }

    pub fn cmdline_limit(&self) -> u32 {
        unsafe { (*self.setup_header).cmdline_size }
    }

    /// Get pointer to setup header for copying to boot params
    pub fn setup_header_ptr(&self) -> *const SetupHeader {
        self.setup_header
    }

    /// Get raw setup header bytes for detailed inspection
    pub fn setup_header_bytes(&self) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts(
                self.setup_header as *const u8,
                core::mem::size_of::<SetupHeader>(),
            )
        }
    }
}
