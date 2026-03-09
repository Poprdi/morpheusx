
// PERSISTENCE — Key-Value store backed by HelixFS /persist/ directory
//
// The persistent KV store maps keys to files under `/persist/<key>`.
// Backend is HelixFS today, but the `morpheus-persistent` crate's
// `PersistenceBackend` trait allows swapping to ESP/TPM/NVRAM later.
//
// This gives userland apps a dead-simple "survive reboots" mechanism:
//   persist_put("settings", &config_bytes);
//   persist_get("settings", &mut buf);

/// Persistence subsystem info.
/// Must match `libmorpheus::persist::PersistInfo` exactly.
#[repr(C)]
pub struct PersistInfo {
    /// Bitmask of active backends: bit 0 = HelixFS
    pub backend_flags: u32,
    pub _pad0: u32,
    /// Number of keys currently stored
    pub num_keys: u64,
    /// Total bytes used by values
    pub used_bytes: u64,
}

/// Binary format info returned by `SYS_PE_INFO`.
/// Must match `libmorpheus::persist::BinaryInfo` exactly.
#[repr(C)]
pub struct BinaryInfo {
    /// Format: 0=unknown, 1=ELF64, 2=PE32+
    pub format: u32,
    /// Architecture: 0=unknown, 1=x86_64, 2=aarch64, 3=arm
    pub arch: u32,
    /// Entry point address (RVA for PE, virtual for ELF)
    pub entry_point: u64,
    /// PE ImageBase (0 for ELF)
    pub image_base: u64,
    /// Total file size in bytes
    pub image_size: u64,
    /// Number of sections (PE) or program headers (ELF)
    pub num_sections: u32,
    pub _pad0: u32,
}

/// Build `/persist/<key>` path in a stack buffer.
/// Returns the path as `&str`, or `None` if the key is invalid.
///
/// Keys must be 1-255 bytes, no `/` or `\0`.
unsafe fn persist_path<'a>(key: &str, buf: &'a mut [u8; 272]) -> Option<&'a str> {
    const PREFIX: &[u8] = b"/persist/";
    if key.is_empty() || key.len() > 255 || key.contains('/') || key.contains('\0') {
        return None;
    }
    buf[..PREFIX.len()].copy_from_slice(PREFIX);
    buf[PREFIX.len()..PREFIX.len() + key.len()].copy_from_slice(key.as_bytes());
    core::str::from_utf8(&buf[..PREFIX.len() + key.len()]).ok()
}

/// Ensure the `/persist` directory exists. Idempotent — ignores AlreadyExists.
unsafe fn ensure_persist_dir() {
    if let Some(mut _vfs_guard) = vfs_lock() {
        let fs = &mut *_vfs_guard.fs;
        let ts = crate::cpu::tsc::read_tsc();
        let _ = morpheus_helix::vfs::vfs_mkdir(&mut fs.mount_table, "/persist", ts);
    }
}

/// `SYS_PERSIST_PUT(key_ptr, key_len, data_ptr, data_len) → 0`
///
/// Store a named blob to persistent storage (`/persist/<key>`).
/// Max key: 255 bytes (no `/` or NUL). Max value: 4 MiB.
/// Overwrites if key already exists. Data is fsynced to disk.
pub unsafe fn sys_persist_put(key_ptr: u64, key_len: u64, data_ptr: u64, data_len: u64) -> u64 {
    if !validate_user_buf(key_ptr, key_len) {
        return EFAULT;
    }
    if data_len > 0 && !validate_user_buf(data_ptr, data_len) {
        return EFAULT;
    }
    if data_len > 4 * 1024 * 1024 {
        return EINVAL;
    }

    let key = match user_path(key_ptr, key_len) {
        Some(k) => k,
        None => return EINVAL,
    };

    let mut path_buf = [0u8; 272];
    let path = match persist_path(key, &mut path_buf) {
        Some(p) => p,
        None => return EINVAL,
    };

    ensure_persist_dir();

    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;
    let fd_table = SCHEDULER.current_fd_table_mut();
    let ts = crate::cpu::tsc::read_tsc();

    // O_WRITE | O_CREATE | O_TRUNC
    let flags: u32 = 0x02 | 0x04 | 0x10;
    let fd = match morpheus_helix::vfs::vfs_open(
        &mut fs.device,
        &mut fs.mount_table,
        fd_table,
        path,
        flags,
        ts,
    ) {
        Ok(fd) => fd,
        Err(e) => return helix_err_to_errno(e),
    };

    if data_len > 0 {
        let data = core::slice::from_raw_parts(data_ptr as *const u8, data_len as usize);
        if let Err(e) = morpheus_helix::vfs::vfs_write(
            &mut fs.device,
            &mut fs.mount_table,
            fd_table,
            fd,
            data,
            ts,
        ) {
            let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
            return helix_err_to_errno(e);
        }
    }

    let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
    let _ = morpheus_helix::vfs::vfs_sync(&mut fs.device, &mut fs.mount_table);
    0
}

/// `SYS_PERSIST_GET(key_ptr, key_len, buf_ptr, buf_len) → bytes_read`
///
/// Load a named blob from persistent storage.
/// If `buf_len` is 0, returns the value's size without reading.
/// Returns `-ENOENT` if the key doesn't exist.
pub unsafe fn sys_persist_get(key_ptr: u64, key_len: u64, buf_ptr: u64, buf_len: u64) -> u64 {
    if !validate_user_buf(key_ptr, key_len) {
        return EFAULT;
    }
    if buf_len > 0 && !validate_user_buf(buf_ptr, buf_len) {
        return EFAULT;
    }

    let key = match user_path(key_ptr, key_len) {
        Some(k) => k,
        None => return EINVAL,
    };

    let mut path_buf = [0u8; 272];
    let path = match persist_path(key, &mut path_buf) {
        Some(p) => p,
        None => return EINVAL,
    };

    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;

    // buf_len == 0 → just return file size (stat only).
    if buf_len == 0 {
        return match morpheus_helix::vfs::vfs_stat(&fs.mount_table, path) {
            Ok(stat) => stat.size,
            Err(e) => helix_err_to_errno(e),
        };
    }

    let fd_table = SCHEDULER.current_fd_table_mut();
    let ts = crate::cpu::tsc::read_tsc();

    let fd = match morpheus_helix::vfs::vfs_open(
        &mut fs.device,
        &mut fs.mount_table,
        fd_table,
        path,
        0x01, // O_READ
        ts,
    ) {
        Ok(fd) => fd,
        Err(e) => return helix_err_to_errno(e),
    };

    let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len as usize);
    let bytes =
        match morpheus_helix::vfs::vfs_read(&mut fs.device, &fs.mount_table, fd_table, fd, buf) {
            Ok(n) => n as u64,
            Err(e) => {
                let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
                return helix_err_to_errno(e);
            }
        };

    let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
    bytes
}

/// `SYS_PERSIST_DEL(key_ptr, key_len) → 0`
///
/// Delete a key from persistent storage. Fsynced to disk.
pub unsafe fn sys_persist_del(key_ptr: u64, key_len: u64) -> u64 {
    if !validate_user_buf(key_ptr, key_len) {
        return EFAULT;
    }

    let key = match user_path(key_ptr, key_len) {
        Some(k) => k,
        None => return EINVAL,
    };

    let mut path_buf = [0u8; 272];
    let path = match persist_path(key, &mut path_buf) {
        Some(p) => p,
        None => return EINVAL,
    };

    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;
    let ts = crate::cpu::tsc::read_tsc();

    match morpheus_helix::vfs::vfs_unlink(&mut fs.mount_table, path, ts) {
        Ok(()) => {
            let _ = morpheus_helix::vfs::vfs_sync(&mut fs.device, &mut fs.mount_table);
            0
        }
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_PERSIST_LIST(buf_ptr, buf_len, offset) → count`
///
/// List keys in persistent storage. Writes NUL-separated key names
/// into `buf_ptr`. Returns the number of keys written. Pass `offset`
/// to skip that many entries (for pagination).
///
/// If `buf_len` is 0, returns the total number of keys.
pub unsafe fn sys_persist_list(buf_ptr: u64, buf_len: u64, offset: u64) -> u64 {
    if buf_len > 0 && !validate_user_buf(buf_ptr, buf_len) {
        return EFAULT;
    }

    let _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &*_vfs_guard.fs;

    let entries = match morpheus_helix::vfs::vfs_readdir(&fs.mount_table, "/persist") {
        Ok(e) => e,
        Err(_) => return 0, // directory doesn't exist → 0 keys
    };

    // Filter out "." and ".." and count real entries.
    let real_count = entries
        .iter()
        .filter(|e| {
            let n = &e.name[..e.name_len as usize];
            n != b"." && n != b".."
        })
        .count();

    if buf_len == 0 || buf_ptr == 0 {
        return real_count as u64;
    }

    let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len as usize);
    let mut pos = 0usize;
    let mut count = 0u64;
    let mut skipped = 0u64;

    for entry in entries.iter() {
        let name_bytes = &entry.name[..entry.name_len as usize];
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }
        if skipped < offset {
            skipped += 1;
            continue;
        }
        let need = name_bytes.len() + 1; // name + NUL terminator
        if pos + need > buf.len() {
            break; // buffer full
        }
        buf[pos..pos + name_bytes.len()].copy_from_slice(name_bytes);
        buf[pos + name_bytes.len()] = 0;
        pos += need;
        count += 1;
    }

    count
}

/// `SYS_PERSIST_INFO(info_ptr) → 0`
///
/// Fill a `PersistInfo` struct with backend status and usage statistics.
pub unsafe fn sys_persist_info(info_ptr: u64) -> u64 {
    let size = core::mem::size_of::<PersistInfo>() as u64;
    if !validate_user_buf(info_ptr, size) {
        return EFAULT;
    }

    let _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &*_vfs_guard.fs;

    let mut num_keys = 0u64;
    let mut used_bytes = 0u64;

    if let Ok(entries) = morpheus_helix::vfs::vfs_readdir(&fs.mount_table, "/persist") {
        for entry in entries.iter() {
            let name_bytes = &entry.name[..entry.name_len as usize];
            if name_bytes == b"." || name_bytes == b".." {
                continue;
            }
            // Build path to stat each file.
            let mut path_buf = [0u8; 272];
            let prefix = b"/persist/";
            if name_bytes.len() > 255 {
                continue;
            }
            path_buf[..prefix.len()].copy_from_slice(prefix);
            path_buf[prefix.len()..prefix.len() + name_bytes.len()].copy_from_slice(name_bytes);
            if let Ok(p) = core::str::from_utf8(&path_buf[..prefix.len() + name_bytes.len()]) {
                if let Ok(stat) = morpheus_helix::vfs::vfs_stat(&fs.mount_table, p) {
                    num_keys += 1;
                    used_bytes += stat.size;
                }
            }
        }
    }

    let info = PersistInfo {
        backend_flags: 1, // bit 0 = HelixFS active
        _pad0: 0,
        num_keys,
        used_bytes,
    };

    core::ptr::write(info_ptr as *mut PersistInfo, info);
    0
}

// SYS_PE_INFO — Binary introspection (PE + ELF)
//
// Uses `morpheus_persistent::pe::header::PeHeaders` for PE/COFF parsing
// and inline ELF64 header parsing for ELF binaries.

/// `SYS_PE_INFO(path_ptr, path_len, info_ptr) → 0`
///
/// Read a binary file from the VFS, detect its format (PE32+ or ELF64),
/// parse the headers, and fill a `BinaryInfo` struct.
///
/// Max file read for headers: 64 KiB.
pub unsafe fn sys_pe_info(path_ptr: u64, path_len: u64, info_ptr: u64) -> u64 {
    let info_size = core::mem::size_of::<BinaryInfo>() as u64;
    if !validate_user_buf(info_ptr, info_size) {
        return EFAULT;
    }

    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;

    // Stat to get file size.
    let file_size = match morpheus_helix::vfs::vfs_stat(&fs.mount_table, path) {
        Ok(s) => s.size as usize,
        Err(e) => return helix_err_to_errno(e),
    };

    if file_size < 64 {
        return EINVAL; // too small to be any known binary format
    }

    // Read at most 64 KB for header parsing.
    let read_size = file_size.min(65536);
    let pages_needed = read_size.div_ceil(4096) as u64;

    // drop registry before disk I/O — holding it blocks every other core from allocating.
    let buf_phys = {
        let mut registry = crate::memory::global_registry_mut();
        match registry.allocate_pages(
            crate::memory::AllocateType::AnyPages,
            crate::memory::MemoryType::Allocated,
            pages_needed,
        ) {
            Ok(addr) => addr,
            Err(_) => return ENOMEM,
        }
        // registry dropped here. lock released.
    };

    let fd_table = SCHEDULER.current_fd_table_mut();
    let ts = crate::cpu::tsc::read_tsc();

    let fd = match morpheus_helix::vfs::vfs_open(
        &mut fs.device,
        &mut fs.mount_table,
        fd_table,
        path,
        0x01,
        ts,
    ) {
        Ok(fd) => fd,
        Err(e) => {
            // re-acquire to free
            let mut registry = crate::memory::global_registry_mut();
            let _ = registry.free_pages(buf_phys, pages_needed);
            return helix_err_to_errno(e);
        }
    };

    let buf = core::slice::from_raw_parts_mut(buf_phys as *mut u8, read_size);
    let bytes_read =
        match morpheus_helix::vfs::vfs_read(&mut fs.device, &fs.mount_table, fd_table, fd, buf) {
            Ok(n) => n,
            Err(e) => {
                let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
                let mut registry = crate::memory::global_registry_mut();
                let _ = registry.free_pages(buf_phys, pages_needed);
                return helix_err_to_errno(e);
            }
        };
    let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);

    let data = core::slice::from_raw_parts(buf_phys as *const u8, bytes_read);

    let mut info = BinaryInfo {
        format: 0,
        arch: 0,
        entry_point: 0,
        image_base: 0,
        image_size: file_size as u64,
        num_sections: 0,
        _pad0: 0,
    };

    // detect elf
    if bytes_read >= 64 && data[0] == 0x7f && data[1] == b'E' && data[2] == b'L' && data[3] == b'F'
    {
        info.format = 1; // ELF64
        let ei_class = data[4];
        if ei_class == 2 {
            // 64-bit
            let e_machine = u16::from_le_bytes([data[18], data[19]]);
            info.arch = match e_machine {
                0x3E => 1, // EM_X86_64
                0xB7 => 2, // EM_AARCH64
                0x28 => 3, // EM_ARM
                _ => 0,
            };
            info.entry_point = u64::from_le_bytes([
                data[24], data[25], data[26], data[27], data[28], data[29], data[30], data[31],
            ]);
            info.num_sections = u16::from_le_bytes([data[60], data[61]]) as u32;
        }
    }
    // detect pe/mz
    else if bytes_read >= 256 && data[0] == b'M' && data[1] == b'Z' {
        info.format = 2; // PE32+
        if let Ok(pe) =
            morpheus_persistent::pe::header::PeHeaders::parse(buf_phys as *const u8, bytes_read)
        {
            info.image_base = pe.optional.image_base;
            info.entry_point = pe.optional.address_of_entry_point as u64;
            info.num_sections = pe.coff.number_of_sections as u32;
            match pe.arch() {
                Ok(morpheus_persistent::pe::PeArch::X64) => info.arch = 1,
                Ok(morpheus_persistent::pe::PeArch::ARM64) => info.arch = 2,
                Ok(morpheus_persistent::pe::PeArch::ARM) => info.arch = 3,
                _ => info.arch = 0,
            }
        }
    }

    // re-acquire to free temp buffer
    {
        let mut registry = crate::memory::global_registry_mut();
        let _ = registry.free_pages(buf_phys, pages_needed);
    }

    core::ptr::write(info_ptr as *mut BinaryInfo, info);
    0
}
