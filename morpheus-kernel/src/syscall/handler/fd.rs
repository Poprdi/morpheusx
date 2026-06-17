// fd manipulation + cwd/chdir/syslog.

use super::common::*;
use crate::schedular::SCHEDULER;
use crate::serial::puts;

pub unsafe fn sys_dup(old_fd: u64) -> u64 {
    let fd_table = SCHEDULER.current_fd_table_mut();

    let src = match fd_table.get(old_fd as usize) {
        Some(desc) => *desc,
        None => return EBADF,
    };

    let new_fd = match fd_table.alloc() {
        Some(fd) => fd,
        None => return ENOMEM,
    };

    if !fd_table.set(new_fd, src) {
        return ENOMEM;
    }
    new_fd as u64
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
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    if path == "/" {
        let proc = SCHEDULER.current_process_mut();
        proc.set_cwd(path);
        return 0;
    }

    let is_dir = {
        let guard = crate::storage::lock();
        let g = &mut *guard.g;
        let (_, m, dev, rel) = match g.resolve_mut(path) {
            Some(t) => t,
            None => return ENOENT,
        };
        match m.fs.stat(dev, rel) {
            Ok(stat) => stat.is_dir,
            Err(_) => return ENOENT,
        }
    };
    if !is_dir {
        return ENOTDIR;
    }
    let proc = SCHEDULER.current_process_mut();
    proc.set_cwd(path);
    0
}
