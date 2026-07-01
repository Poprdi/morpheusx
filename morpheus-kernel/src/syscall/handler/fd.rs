// fd manipulation + cwd/chdir/syslog.

use super::common::*;
use crate::schedular::SCHEDULER;
use crate::serial::puts;

/// `dup`: the new fd SHARES the old fd's open-file-description (one cursor), not a
/// snapshot copy — this is the dup-copies-offset fix. The new fd is `FD_CLOEXEC`
/// clear (POSIX).
pub unsafe fn sys_dup(old_fd: u64) -> u64 {
    let fd_table = SCHEDULER.current_fd_table_mut();
    match fd_table.dup(old_fd as usize) {
        Ok(new) => new as u64,
        Err(crate::storage::fs_api::VfsError::BadFd) => EBADF,
        Err(_) => EMFILE,
    }
}

/// Bypass console/WM; straight to serial.
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
        for &b in bytes {
            crate::serial::putc(b);
        }
    }
    len
}

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

pub unsafe fn sys_chdir(path_ptr: u64, path_len: u64) -> u64 {
    // Resolve against the current cwd first so `chdir("..")`/`chdir("sub")` work;
    // the stored cwd is always the canonical absolute path.
    let path = match resolve_user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    if path == "/" {
        let proc = SCHEDULER.current_process_mut();
        proc.set_cwd(&path);
        return 0;
    }

    let is_dir = {
        let guard = crate::storage::lock();
        let g = &mut *guard.g;
        let (_, m, dev, rel) = match g.resolve_mut(&path) {
            Some(t) => t,
            None => return ENOENT,
        };
        match m.fs.stat(dev, rel) {
            Ok(stat) => {
                use morpheus_foundation::flags::mode;
                stat.mode & mode::S_IFMT == mode::S_IFDIR
            },
            Err(_) => return ENOENT,
        }
    };
    if !is_dir {
        return ENOTDIR;
    }
    let proc = SCHEDULER.current_process_mut();
    proc.set_cwd(&path);
    0
}

/// SYS_FCNTL: `fd,cmd,arg -> ret | -errno`. The std-required subset:
/// `F_GETFD/F_SETFD` (FD_CLOEXEC, per-fd), `F_GETFL/F_SETFL` (O_NONBLOCK on the
/// shared OFD), and `F_DUPFD/F_DUPFD_CLOEXEC` (backs `try_clone`, sharing the OFD).
pub unsafe fn sys_fcntl(fd: u64, cmd: u64, arg: u64) -> u64 {
    use morpheus_foundation::flags::open_flags::O_NONBLOCK;
    use morpheus_foundation::flags::{
        FD_CLOEXEC, F_DUPFD, F_DUPFD_CLOEXEC, F_GETFD, F_GETFL, F_SETFD, F_SETFL,
    };

    let fd_table = SCHEDULER.current_fd_table_mut();
    let fd = fd as usize;
    if fd_table.get(fd).is_none() {
        return EBADF;
    }

    match cmd {
        F_GETFD => {
            if fd_table.get_cloexec(fd).unwrap_or(false) {
                FD_CLOEXEC
            } else {
                0
            }
        },
        F_SETFD => {
            fd_table.set_cloexec(fd, arg & FD_CLOEXEC != 0);
            0
        },
        F_GETFL => fd_table.status_flags(fd).unwrap_or(0) as u64,
        F_SETFL => {
            // Only the mutable status bit (O_NONBLOCK) is honoured; access mode and
            // creation flags are immutable post-open per POSIX.
            let cur = fd_table.status_flags(fd).unwrap_or(0);
            let next = (cur & !O_NONBLOCK) | (arg as u32 & O_NONBLOCK);
            fd_table.set_status_flags(fd, next);
            0
        },
        F_DUPFD => match fd_table.dup_from(fd, arg as usize, false) {
            Ok(new) => new as u64,
            Err(crate::storage::fs_api::VfsError::BadFd) => EBADF,
            Err(_) => EMFILE,
        },
        F_DUPFD_CLOEXEC => match fd_table.dup_from(fd, arg as usize, true) {
            Ok(new) => new as u64,
            Err(crate::storage::fs_api::VfsError::BadFd) => EBADF,
            Err(_) => EMFILE,
        },
        _ => EINVAL,
    }
}
