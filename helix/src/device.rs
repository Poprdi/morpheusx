use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};

#[derive(Debug, Clone, Copy)]
pub struct MemIoError;

impl core::fmt::Display for MemIoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("memory block I/O error")
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RawIoError;

impl core::fmt::Display for RawIoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("block I/O error")
    }
}

/// Backend-agnostic block device via function pointers (avoids generics/dyn).
pub struct RawBlockDevice {
    /// Opaque driver context.
    ctx: *mut u8,
    sectors: u64,
    /// Sector size; power of 2, typically 512 or 4096.
    sector_size: u32,
    read_fn: unsafe fn(ctx: *mut u8, lba: u64, dst: *mut u8, len: usize) -> bool,
    write_fn: unsafe fn(ctx: *mut u8, lba: u64, src: *const u8, len: usize) -> bool,
    flush_fn: unsafe fn(ctx: *mut u8) -> bool,
}

unsafe impl Send for RawBlockDevice {}
unsafe impl Sync for RawBlockDevice {}

impl RawBlockDevice {
    /// SAFETY: `ctx` must outlive the device; fn pointers must be safe with it.
    pub const unsafe fn new(
        ctx: *mut u8,
        sectors: u64,
        sector_size: u32,
        read_fn: unsafe fn(*mut u8, u64, *mut u8, usize) -> bool,
        write_fn: unsafe fn(*mut u8, u64, *const u8, usize) -> bool,
        flush_fn: unsafe fn(*mut u8) -> bool,
    ) -> Self {
        Self {
            ctx,
            sectors,
            sector_size,
            read_fn,
            write_fn,
            flush_fn,
        }
    }

    pub fn total_bytes(&self) -> u64 {
        self.sectors * self.sector_size as u64
    }
}

impl BlockIo for RawBlockDevice {
    type Error = RawIoError;

    fn block_size(&self) -> BlockSize {
        BlockSize::new(self.sector_size).unwrap()
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.sectors)
    }

    fn read_blocks(&mut self, start_lba: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        let ok = unsafe { (self.read_fn)(self.ctx, start_lba.0, dst.as_mut_ptr(), dst.len()) };
        if ok {
            Ok(())
        } else {
            Err(RawIoError)
        }
    }

    fn write_blocks(&mut self, start_lba: Lba, src: &[u8]) -> Result<(), Self::Error> {
        let ok = unsafe { (self.write_fn)(self.ctx, start_lba.0, src.as_ptr(), src.len()) };
        if ok {
            Ok(())
        } else {
            Err(RawIoError)
        }
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        let ok = unsafe { (self.flush_fn)(self.ctx) };
        if ok {
            Ok(())
        } else {
            Err(RawIoError)
        }
    }
}

/// RAM-backed block device. Identity-mapped; memcpy I/O; never fails.
pub struct MemBlockDevice {
    base: *mut u8,
    sectors: u64,
    sector_size: u32,
}

unsafe impl Send for MemBlockDevice {}
unsafe impl Sync for MemBlockDevice {}

impl MemBlockDevice {
    /// SAFETY: `base..base+size` must be live, identity-mapped for the device's lifetime.
    pub unsafe fn new(base: *mut u8, size: usize, sector_size: u32) -> Self {
        Self {
            base,
            sectors: size as u64 / sector_size as u64,
            sector_size,
        }
    }

    pub fn base(&self) -> *mut u8 {
        self.base
    }
    pub fn total_bytes(&self) -> u64 {
        self.sectors * self.sector_size as u64
    }

    /// Wrap as `RawBlockDevice`. `mem` must outlive the returned device
    /// (usually held in a `static`).
    pub fn into_raw(mem: &mut MemBlockDevice) -> RawBlockDevice {
        unsafe fn mem_read(ctx: *mut u8, lba: u64, dst: *mut u8, len: usize) -> bool {
            let dev = &*(ctx as *const MemBlockDevice);
            let offset = lba * dev.sector_size as u64;
            core::ptr::copy_nonoverlapping(dev.base.add(offset as usize), dst, len);
            true
        }
        unsafe fn mem_write(ctx: *mut u8, lba: u64, src: *const u8, len: usize) -> bool {
            let dev = &*(ctx as *const MemBlockDevice);
            let offset = lba * dev.sector_size as u64;
            core::ptr::copy_nonoverlapping(src, dev.base.add(offset as usize), len);
            true
        }
        unsafe fn mem_flush(_ctx: *mut u8) -> bool {
            true
        }

        unsafe {
            RawBlockDevice::new(
                mem as *mut MemBlockDevice as *mut u8,
                mem.sectors,
                mem.sector_size,
                mem_read,
                mem_write,
                mem_flush,
            )
        }
    }
}

impl BlockIo for MemBlockDevice {
    type Error = MemIoError;

    fn block_size(&self) -> BlockSize {
        BlockSize::new(self.sector_size).unwrap()
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.sectors)
    }

    fn read_blocks(&mut self, start_lba: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        let offset = start_lba.0 * self.sector_size as u64;
        unsafe {
            core::ptr::copy_nonoverlapping(
                self.base.add(offset as usize),
                dst.as_mut_ptr(),
                dst.len(),
            );
        }
        Ok(())
    }

    fn write_blocks(&mut self, start_lba: Lba, src: &[u8]) -> Result<(), Self::Error> {
        let offset = start_lba.0 * self.sector_size as u64;
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), self.base.add(offset as usize), src.len());
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
