extern crate alloc;

use alloc::format;
use alloc::string::String;

use libmorpheus::process::{self, PsEntry};
use libmorpheus::sys::{self, SysInfo};

use crate::console::Console;
use crate::fb::Framebuffer;

use super::{parse_u32, parse_u64};

pub fn ps() -> i32 {
    let mut entries: [PsEntry; 64] = core::array::from_fn(|_| PsEntry::zeroed());
    let count = process::ps(&mut entries);

    libmorpheus::println!(
        "{:<5} {:<5} {:<10} {:<10} {}",
        "PID",
        "PPID",
        "STATE",
        "TICKS",
        "NAME"
    );
    for entry in &entries[..count] {
        let state = match entry.state {
            0 => "Ready",
            1 => "Running",
            2 => "Blocked",
            3 => "Zombie",
            4 => "Dead",
            _ => "?",
        };
        libmorpheus::println!(
            "{:<5} {:<5} {:<10} {:<10} {}",
            entry.pid,
            entry.ppid,
            state,
            entry.cpu_ticks,
            entry.name_str()
        );
    }
    0
}

pub fn ps_fb(fb: &Framebuffer, con: &mut Console) -> i32 {
    let mut entries: [PsEntry; 64] = core::array::from_fn(|_| PsEntry::zeroed());
    let count = process::ps(&mut entries);

    con.write_str(
        fb,
        &format!(
            "{:<5} {:<5} {:<10} {:<10} {}\n",
            "PID", "PPID", "STATE", "TICKS", "NAME"
        ),
    );
    for entry in &entries[..count] {
        let state = match entry.state {
            0 => "Ready",
            1 => "Running",
            2 => "Blocked",
            3 => "Zombie",
            4 => "Dead",
            _ => "?",
        };
        con.write_str(
            fb,
            &format!(
                "{:<5} {:<5} {:<10} {:<10} {}\n",
                entry.pid,
                entry.ppid,
                state,
                entry.cpu_ticks,
                entry.name_str()
            ),
        );
    }
    0
}

pub fn kill(args: &[String]) -> i32 {
    let Some(pid_str) = args.first() else {
        libmorpheus::eprintln!("kill: need <pid> [signal]");
        return 1;
    };
    let Some(pid) = parse_u32(pid_str) else {
        libmorpheus::eprintln!("kill: invalid pid: {}", pid_str);
        return 1;
    };

    let sig: u8 = if let Some(s) = args.get(1) {
        match parse_u32(s) {
            Some(n) if n <= 255 => n as u8,
            _ => {
                libmorpheus::eprintln!("kill: invalid signal: {}", s);
                return 1;
            }
        }
    } else {
        process::signal::SIGTERM
    };

    match process::kill(pid, sig) {
        Ok(()) => 0,
        Err(e) => {
            libmorpheus::eprintln!("kill: pid {}: error 0x{:x}", pid, e);
            1
        }
    }
}

pub fn kill_fb(args: &[String], fb: &Framebuffer, con: &mut Console) -> i32 {
    let Some(pid_str) = args.first() else {
        con.write_colored(fb, "kill: need <pid> [signal]\n", (170, 0, 0));
        return 1;
    };
    let Some(pid) = parse_u32(pid_str) else {
        con.write_colored(
            fb,
            &format!("kill: invalid pid: {}\n", pid_str),
            (170, 0, 0),
        );
        return 1;
    };

    let sig: u8 = if let Some(s) = args.get(1) {
        match parse_u32(s) {
            Some(n) if n <= 255 => n as u8,
            _ => {
                con.write_colored(
                    fb,
                    &format!("kill: invalid signal: {}\n", s),
                    (170, 0, 0),
                );
                return 1;
            }
        }
    } else {
        process::signal::SIGTERM
    };

    match process::kill(pid, sig) {
        Ok(()) => 0,
        Err(e) => {
            con.write_colored(
                fb,
                &format!("kill: pid {}: error 0x{:x}\n", pid, e),
                (170, 0, 0),
            );
            1
        }
    }
}

pub fn sysinfo() -> i32 {
    let mut info = SysInfo::zeroed();
    if sys::sysinfo(&mut info).is_err() {
        libmorpheus::eprintln!("sysinfo: failed");
        return 1;
    }

    let uptime_s = info.uptime_ms() / 1000;
    let hours = uptime_s / 3600;
    let mins = (uptime_s % 3600) / 60;
    let secs = uptime_s % 60;

    libmorpheus::println!("Uptime:    {}h {:02}m {:02}s", hours, mins, secs);
    libmorpheus::println!("Processes: {}", info.num_procs);
    libmorpheus::println!(
        "Memory:    {} / {} KiB free",
        info.free_mem / 1024,
        info.total_mem / 1024
    );
    libmorpheus::println!(
        "Heap:      {} / {} KiB used",
        info.heap_used / 1024,
        info.heap_total / 1024
    );
    libmorpheus::println!("TSC freq:  {} MHz", info.tsc_freq / 1_000_000);
    0
}

pub fn sysinfo_fb(fb: &Framebuffer, con: &mut Console) -> i32 {
    let mut info = SysInfo::zeroed();
    if sys::sysinfo(&mut info).is_err() {
        con.write_colored(fb, "sysinfo: failed\n", (170, 0, 0));
        return 1;
    }

    let uptime_s = info.uptime_ms() / 1000;
    let hours = uptime_s / 3600;
    let mins = (uptime_s % 3600) / 60;
    let secs = uptime_s % 60;

    con.write_str(
        fb,
        &format!("Uptime:    {}h {:02}m {:02}s\n", hours, mins, secs),
    );
    con.write_str(fb, &format!("Processes: {}\n", info.num_procs));
    con.write_str(
        fb,
        &format!(
            "Memory:    {} / {} KiB free\n",
            info.free_mem / 1024,
            info.total_mem / 1024
        ),
    );
    con.write_str(
        fb,
        &format!(
            "Heap:      {} / {} KiB used\n",
            info.heap_used / 1024,
            info.heap_total / 1024
        ),
    );
    con.write_str(
        fb,
        &format!("TSC freq:  {} MHz\n", info.tsc_freq / 1_000_000),
    );
    0
}

pub fn sleep(args: &[String]) -> i32 {
    let Some(ms_str) = args.first() else {
        libmorpheus::eprintln!("sleep: need <milliseconds>");
        return 1;
    };
    let Some(ms) = parse_u64(ms_str) else {
        libmorpheus::eprintln!("sleep: invalid duration: {}", ms_str);
        return 1;
    };
    process::sleep(ms);
    0
}

pub fn sleep_fb(args: &[String], fb: &Framebuffer, con: &mut Console) -> i32 {
    let Some(ms_str) = args.first() else {
        con.write_colored(fb, "sleep: need <milliseconds>\n", (170, 0, 0));
        return 1;
    };
    let Some(ms) = parse_u64(ms_str) else {
        con.write_colored(
            fb,
            &format!("sleep: invalid duration: {}\n", ms_str),
            (170, 0, 0),
        );
        return 1;
    };
    process::sleep(ms);
    0
}
