extern crate alloc;

use alloc::string::String;

use libmorpheus::process::{self, PsEntry};
use libmorpheus::sys::{self, SysInfo};

use super::{parse_u32, parse_u64};

pub fn ps() -> i32 {
    let mut entries: [PsEntry; 64] = core::array::from_fn(|_| PsEntry::zeroed());
    let count = process::ps(&mut entries);

    libmorpheus::println!("{:<5} {:<5} {:<10} {:<10} {}", "PID", "PPID", "STATE", "TICKS", "NAME");
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
    libmorpheus::println!("Memory:    {} / {} KiB free", info.free_mem / 1024, info.total_mem / 1024);
    libmorpheus::println!("Heap:      {} / {} KiB used", info.heap_used / 1024, info.heap_total / 1024);
    libmorpheus::println!("TSC freq:  {} MHz", info.tsc_freq / 1_000_000);
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
