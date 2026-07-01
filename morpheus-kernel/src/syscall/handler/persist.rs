// KV persistence under /persist/<key>; HelixFS-backed for now, swappable via
// the morpheus-persistent::PersistenceBackend trait (ESP/TPM/NVRAM later).

use super::common::*;
use crate::hal;
use crate::schedular::SCHEDULER;
use crate::storage::{self, vfs_err_to_errno};
use morpheus_foundation::PAGE_SIZE;
use morpheus_hal_api::{AllocKind, MemoryType};

pub use morpheus_foundation::types::{BinaryInfo, PersistInfo};

/// Keys: 1-255 bytes, no `/` or NUL.
unsafe fn persist_path<'a>(key: &str, buf: &'a mut [u8; 272]) -> Option<&'a str> {
    const PREFIX: &[u8] = b"/persist/";
    if key.is_empty() || key.len() > 255 || key.contains('/') || key.contains('\0') {
        return None;
    }
    buf[..PREFIX.len()].copy_from_slice(PREFIX);
    buf[PREFIX.len()..PREFIX.len() + key.len()].copy_from_slice(key.as_bytes());
    core::str::from_utf8(&buf[..PREFIX.len() + key.len()]).ok()
}

/// Idempotent — swallows AlreadyExists.
unsafe fn ensure_persist_dir() {
    let ts = hal().timer().read_tsc();
    let _ = storage::mkdir_root("/persist", ts);
}

/// Open `path` through the resolved mount, registering the fd in the per-process
/// table (bumps the mount refcount). Mirrors `sys_fs_open` for the persist layer.
unsafe fn persist_open(path: &str, flags: u32, ts: u64) -> Result<usize, u64> {
    let fd_table = SCHEDULER.current_fd_table_mut();
    let fd = fd_table.alloc().ok_or(EMFILE)?;

    let guard = storage::lock();
    let g = &mut *guard.g;
    let (mount_id, m, dev, rel) = g.resolve_mut(path).ok_or(ENOENT)?;
    let opened = m.fs.open(dev, rel, flags, ts).map_err(vfs_err_to_errno)?;

    let mut state = storage::fs_api::FdState::empty();
    state.mount_id = mount_id;
    state.flags = flags;
    let pb = rel.as_bytes();
    let n = pb.len().min(state.path.len());
    state.path[..n].copy_from_slice(&pb[..n]);
    state.path_len = n as u16;
    state.cookie = opened.cookie;
    m.open_fds = m.open_fds.saturating_add(1);

    if !fd_table.set(fd, state) {
        if let Some((m, _)) = g.mount_dev_mut(mount_id) {
            m.open_fds = m.open_fds.saturating_sub(1);
        }
        return Err(EMFILE);
    }
    Ok(fd)
}

/// Close a persist fd: run the backend close, drop the mount refcount, free slot.
unsafe fn persist_close(fd: usize) {
    let fd_table = SCHEDULER.current_fd_table_mut();
    let desc = match fd_table.get(fd) {
        Some(d) => *d,
        None => return,
    };
    {
        let guard = storage::lock();
        let g = &mut *guard.g;
        if let Some((m, dev)) = g.mount_dev_mut(desc.mount_id) {
            let _ = m.fs.close(dev, &desc);
            m.open_fds = m.open_fds.saturating_sub(1);
        }
    }
    let _ = fd_table.free(fd);
}

/// `SYS_PERSIST_PUT(key_ptr, key_len, data_ptr, data_len) → 0`
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

    let ts = hal().timer().read_tsc();

    // O_WRITE | O_CREATE | O_TRUNC
    let flags: u32 = 0x02 | 0x04 | 0x10;
    let fd = match persist_open(path, flags, ts) {
        Ok(fd) => fd,
        Err(e) => return e,
    };

    if data_len > 0 {
        let data = core::slice::from_raw_parts(data_ptr as *const u8, data_len as usize);
        let fd_table = SCHEDULER.current_fd_table_mut();
        let mut desc = match fd_table.get(fd) {
            Some(d) => *d,
            None => return EBADF,
        };
        let guard = storage::lock();
        let g = &mut *guard.g;
        let res = match g.mount_dev_mut(desc.mount_id) {
            Some((m, dev)) => m.fs.write(dev, &mut desc, data, ts),
            None => Err(storage::fs_api::VfsError::BadFd),
        };
        if let Some(d) = fd_table.get_mut(fd) {
            d.offset = desc.offset;
        }
        drop(guard);
        if let Err(e) = res {
            persist_close(fd);
            return vfs_err_to_errno(e);
        }
    }

    persist_close(fd);
    {
        let guard = storage::lock();
        let g = &mut *guard.g;
        if let Some((_, m, dev, _)) = g.resolve_mut(path) {
            let _ = m.fs.sync(dev);
        }
    }
    0
}

/// `SYS_PERSIST_GET(key_ptr, key_len, buf_ptr, buf_len) → bytes_read`
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

    let ts = hal().timer().read_tsc();

    // buf_len == 0 → just return file size (stat only).
    if buf_len == 0 {
        let guard = storage::lock();
        let g = &mut *guard.g;
        let (_, m, dev, rel) = match g.resolve_mut(path) {
            Some(t) => t,
            None => return ENOENT,
        };
        return match m.fs.stat(dev, rel) {
            Ok(stat) => stat.size,
            Err(e) => vfs_err_to_errno(e),
        };
    }

    let fd = match persist_open(path, 0x01 /* O_READ */, ts) {
        Ok(fd) => fd,
        Err(e) => return e,
    };

    let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len as usize);
    let fd_table = SCHEDULER.current_fd_table_mut();
    let desc = match fd_table.get(fd) {
        Some(d) => *d,
        None => return EBADF,
    };
    let bytes = {
        let guard = storage::lock();
        let g = &mut *guard.g;
        let res = match g.mount_dev_mut(desc.mount_id) {
            Some((m, dev)) => m.fs.read(dev, &desc, buf),
            None => Err(storage::fs_api::VfsError::BadFd),
        };
        match res {
            Ok(n) => {
                if let Some(d) = fd_table.get_mut(fd) {
                    d.offset = d.offset.saturating_add(n as u64);
                }
                n as u64
            },
            Err(e) => {
                drop(guard);
                persist_close(fd);
                return vfs_err_to_errno(e);
            },
        }
    };

    persist_close(fd);
    bytes
}

/// `SYS_PERSIST_DEL(key_ptr, key_len) → 0`
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

    let ts = hal().timer().read_tsc();
    let guard = storage::lock();
    let g = &mut *guard.g;
    let (_, m, dev, rel) = match g.resolve_mut(path) {
        Some(t) => t,
        None => return ENOENT,
    };
    match m.fs.unlink(dev, rel, ts) {
        Ok(()) => {
            let _ = m.fs.sync(dev);
            0
        },
        Err(e) => vfs_err_to_errno(e),
    }
}

/// `SYS_PERSIST_LIST(buf_ptr, buf_len, offset) → count`
pub unsafe fn sys_persist_list(buf_ptr: u64, buf_len: u64, offset: u64) -> u64 {
    if buf_len > 0 && !validate_user_buf(buf_ptr, buf_len) {
        return EFAULT;
    }

    let guard = storage::lock();
    let g = &mut *guard.g;
    let entries = match g.resolve_mut("/persist") {
        // rel == "/persist" while persist lives on the root mount; the persist
        // layer is absolute-path-keyed on root by design.
        Some((_, m, dev, rel)) => match m.fs.readdir(dev, rel) {
            Ok(e) => e,
            Err(_) => return 0, // directory doesn't exist → 0 keys
        },
        None => return 0,
    };

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
        let need = name_bytes.len() + 1;
        if pos + need > buf.len() {
            break;
        }
        buf[pos..pos + name_bytes.len()].copy_from_slice(name_bytes);
        buf[pos + name_bytes.len()] = 0;
        pos += need;
        count += 1;
    }

    count
}

/// `SYS_PERSIST_INFO(info_ptr) → 0`
pub unsafe fn sys_persist_info(info_ptr: u64) -> u64 {
    let size = core::mem::size_of::<PersistInfo>() as u64;
    if !validate_user_buf(info_ptr, size) {
        return EFAULT;
    }

    let guard = storage::lock();
    let g = &mut *guard.g;

    let mut num_keys = 0u64;
    let mut used_bytes = 0u64;

    if let Some((_, m, dev, rel)) = g.resolve_mut("/persist") {
        if let Ok(entries) = m.fs.readdir(dev, rel) {
            for entry in entries.iter() {
                let name_bytes = &entry.name[..entry.name_len as usize];
                if name_bytes == b"." || name_bytes == b".." {
                    continue;
                }
                let mut path_buf = [0u8; 272];
                let prefix = b"/persist/";
                if name_bytes.len() > 255 {
                    continue;
                }
                path_buf[..prefix.len()].copy_from_slice(prefix);
                path_buf[prefix.len()..prefix.len() + name_bytes.len()].copy_from_slice(name_bytes);
                if let Ok(p) = core::str::from_utf8(&path_buf[..prefix.len() + name_bytes.len()]) {
                    if let Ok(stat) = m.fs.stat(dev, p) {
                        num_keys += 1;
                        used_bytes += stat.size;
                    }
                }
            }
        }
    }

    let info = PersistInfo {
        backend_flags: 1,
        _pad0: 0,
        num_keys,
        used_bytes,
    };

    core::ptr::write(info_ptr as *mut PersistInfo, info);
    0
}

// SYS_PE_INFO — Binary introspection (PE + ELF).
pub unsafe fn sys_pe_info(path_ptr: u64, path_len: u64, info_ptr: u64) -> u64 {
    let info_size = core::mem::size_of::<BinaryInfo>() as u64;
    if !validate_user_buf(info_ptr, info_size) {
        return EFAULT;
    }

    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    let file_size = {
        let guard = storage::lock();
        let g = &mut *guard.g;
        let (_, m, dev, rel) = match g.resolve_mut(path) {
            Some(t) => t,
            None => return ENOENT,
        };
        match m.fs.stat(dev, rel) {
            Ok(s) => s.size as usize,
            Err(e) => return vfs_err_to_errno(e),
        }
    };

    if file_size < 64 {
        return EINVAL;
    }

    let read_size = file_size.min(65536);
    let pages_needed = read_size.div_ceil(PAGE_SIZE as usize) as u64;

    let buf_phys =
        match hal()
            .phys()
            .allocate_pages(AllocKind::AnyPages, MemoryType::Allocated, pages_needed)
        {
            Ok(addr) => addr,
            Err(_) => return ENOMEM,
        };

    let ts = hal().timer().read_tsc();

    // Internal read: open the backend directly with a transient FdState (no
    // process fd slot), read into the page buffer, close. One lock span.
    let buf = core::slice::from_raw_parts_mut(buf_phys as *mut u8, read_size);
    let bytes_read = {
        let guard = storage::lock();
        let g = &mut *guard.g;
        let (mount_id, m, dev, rel) = match g.resolve_mut(path) {
            Some(t) => t,
            None => {
                let _ = hal().phys().free_pages(buf_phys, pages_needed);
                return ENOENT;
            },
        };
        let opened = match m.fs.open(dev, rel, 0x01, ts) {
            Ok(o) => o,
            Err(e) => {
                let _ = hal().phys().free_pages(buf_phys, pages_needed);
                return vfs_err_to_errno(e);
            },
        };
        let mut fdstate = storage::fs_api::FdState::empty();
        fdstate.mount_id = mount_id;
        fdstate.cookie = opened.cookie;
        // Backend reads key off the (mount-relative) path, not just the cookie.
        let pb = rel.as_bytes();
        let pn = pb.len().min(fdstate.path.len());
        fdstate.path[..pn].copy_from_slice(&pb[..pn]);
        fdstate.path_len = pn as u16;
        let n = match m.fs.read(dev, &fdstate, buf) {
            Ok(n) => n,
            Err(e) => {
                let _ = m.fs.close(dev, &fdstate);
                let _ = hal().phys().free_pages(buf_phys, pages_needed);
                return vfs_err_to_errno(e);
            },
        };
        let _ = m.fs.close(dev, &fdstate);
        n
    };

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

    if bytes_read >= 64 && data[0] == 0x7f && data[1] == b'E' && data[2] == b'L' && data[3] == b'F'
    {
        info.format = 1; // ELF64
        let ei_class = data[4];
        if ei_class == 2 {
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
    } else if bytes_read >= 256 && data[0] == b'M' && data[1] == b'Z' {
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

    let _ = hal().phys().free_pages(buf_phys, pages_needed);

    core::ptr::write(info_ptr as *mut BinaryInfo, info);
    0
}
