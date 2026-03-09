
// SYS_DUP — duplicate a file descriptor

/// `SYS_DUP(old_fd) → new_fd`
pub unsafe fn sys_dup(old_fd: u64) -> u64 {
    let fd_table = SCHEDULER.current_fd_table_mut();

    // Validate old_fd is open.
    let src = match fd_table.get(old_fd as usize) {
        Ok(desc) => *desc,
        Err(_) => return EBADF,
    };

    // Allocate new fd.
    let new_fd = match fd_table.alloc() {
        Ok(fd) => fd,
        Err(_) => return ENOMEM,
    };

    fd_table.fds[new_fd] = src;
    new_fd as u64
}

// SYS_SYSLOG — write to kernel serial log

/// `SYS_SYSLOG(ptr, len) → len`
///
/// Writes a message directly to the kernel serial log (bypassing the
/// console/window system).  Useful for debugging.
pub unsafe fn sys_syslog(ptr: u64, len: u64) -> u64 {
    if !validate_user_buf(ptr, len) {
        return EFAULT;
    }
    if len > (1 << 20) {
        return EINVAL;
    }
    let bytes = core::slice::from_raw_parts(ptr as *const u8, len as usize);
    if let Ok(s) = core::str::from_utf8(bytes) {
        puts("[USR] ");
        puts(s);
        if !s.ends_with('\n') {
            puts("\n");
        }
    } else {
        // Non-UTF8: write raw bytes.
        for &b in bytes {
            crate::serial::putc(b);
        }
    }
    len
}

// SYS_GETCWD — get current working directory

/// `SYS_GETCWD(buf_ptr, buf_len) → cwd_len`
///
/// Copies the current working directory into the user buffer.
/// Returns the length of the CWD string.
pub unsafe fn sys_getcwd(buf_ptr: u64, buf_len: u64) -> u64 {
    if !validate_user_buf(buf_ptr, buf_len) {
        return EFAULT;
    }
    let proc = SCHEDULER.current_process_mut();
    let cwd = proc.cwd_str();
    let copy_len = cwd.len().min(buf_len as usize);
    let dst = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, copy_len);
    dst.copy_from_slice(&cwd.as_bytes()[..copy_len]);
    cwd.len() as u64
}

// SYS_CHDIR — change current working directory

/// `SYS_CHDIR(path_ptr, path_len) → 0`
///
/// Changes the calling process's working directory to the given path.
/// Returns `-ENOENT` if the path does not exist in the VFS.
pub unsafe fn sys_chdir(path_ptr: u64, path_len: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    // Root always exists.
    if path == "/" {
        let proc = SCHEDULER.current_process_mut();
        proc.set_cwd(path);
        return 0;
    }

    // Verify path exists and is a directory via VFS stat.
    let _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &*_vfs_guard.fs;
    match morpheus_helix::vfs::vfs_stat(&fs.mount_table, path) {
        Ok(stat) => {
            if !stat.is_dir {
                return ENOTDIR;
            }
            let proc = SCHEDULER.current_process_mut();
            proc.set_cwd(path);
            0
        }
        Err(_) => ENOENT,
    }
}
