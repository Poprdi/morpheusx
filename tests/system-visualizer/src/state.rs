use alloc::boxed::Box;
use libmorpheus::process::{self, PsEntry};
use libmorpheus::sys::{self, SysInfo};
use libmorpheus::time;

const MAX_PROCS: usize = 64;
const POLL_INTERVAL_NS: u64 = 250_000_000;
const HISTORY_LEN: usize = 120;

#[derive(Clone, Copy)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub state: u32,
    pub priority: u32,
    pub cpu_ticks: u64,
    pub pages_alloc: u64,
    pub name: [u8; 32],
    pub cpu_pct: f32,
    pub mem_kb: u64,
}

impl ProcessInfo {
    const ZEROED: Self = Self {
        pid: 0, ppid: 0, state: 0, priority: 0,
        cpu_ticks: 0, pages_alloc: 0, name: [0; 32],
        cpu_pct: 0.0, mem_kb: 0,
    };

    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..len]).unwrap_or("???")
    }

    pub fn state_str(&self) -> &'static str {
        match self.state {
            0 => "READY",
            1 => "RUN",
            2 => "BLOCK",
            3 => "ZOMBIE",
            4 => "DEAD",
            _ => "?",
        }
    }
}

pub struct SystemState {
    procs: [ProcessInfo; MAX_PROCS],
    prev_ticks: [u64; MAX_PROCS],
    pub proc_count: usize,
    pub total_mem: u64,
    pub free_mem: u64,
    pub heap_total: u64,
    pub heap_used: u64,
    pub heap_free: u64,
    pub uptime_ms: u64,
    pub tsc_freq: u64,
    last_poll_ns: u64,
    prev_poll_ns: u64,
    pub load_history: [u8; HISTORY_LEN],
    pub cpu_history: [u8; HISTORY_LEN],
    load_head: usize,
    pub ready_count: u32,
    pub run_count: u32,
    pub blocked_count: u32,
    pub total_cpu_pct: f32,
}

impl SystemState {
    pub fn new() -> Self {
        Self {
            procs: [ProcessInfo::ZEROED; MAX_PROCS],
            prev_ticks: [0; MAX_PROCS],
            proc_count: 0,
            total_mem: 0,
            free_mem: 0,
            heap_total: 0,
            heap_used: 0,
            heap_free: 0,
            uptime_ms: 0,
            tsc_freq: 0,
            last_poll_ns: 0,
            prev_poll_ns: 0,
            load_history: [0; HISTORY_LEN],
            cpu_history: [0; HISTORY_LEN],
            load_head: 0,
            ready_count: 0,
            run_count: 0,
            blocked_count: 0,
            total_cpu_pct: 0.0,
        }
    }

    pub fn should_poll(&self, now_ns: u64) -> bool {
        now_ns.saturating_sub(self.last_poll_ns) >= POLL_INTERVAL_NS
    }

    pub fn poll(&mut self) {
        let now = time::clock_gettime();
        let _elapsed_ns = now.saturating_sub(self.prev_poll_ns).max(1);

        let mut info = SysInfo::zeroed();
        let _ = sys::sysinfo(&mut info);
        self.total_mem = info.total_mem;
        self.free_mem = info.free_mem;
        self.heap_total = info.heap_total;
        self.heap_used = info.heap_used;
        self.heap_free = info.heap_free;
        self.uptime_ms = info.uptime_ms();
        self.tsc_freq = info.tsc_freq;

        let mut raw = Box::new([const { PsEntry::zeroed() }; MAX_PROCS]);
        let count = process::ps(&mut *raw).min(MAX_PROCS);

        let mut total_delta: u64 = 0;

        for i in 0..count {
            let r = &raw[i];
            let prev = self.find_prev_ticks(r.pid);
            let delta = r.cpu_ticks.saturating_sub(prev);
            total_delta = total_delta.saturating_add(delta);

            self.procs[i] = ProcessInfo {
                pid: r.pid,
                ppid: r.ppid,
                state: r.state,
                priority: r.priority,
                cpu_ticks: r.cpu_ticks,
                pages_alloc: r.pages_alloc,
                name: r.name,
                cpu_pct: 0.0,
                mem_kb: r.pages_alloc * 4,
            };
        }

        if total_delta > 0 {
            for i in 0..count {
                let prev = self.find_prev_ticks(self.procs[i].pid);
                let delta = self.procs[i].cpu_ticks.saturating_sub(prev);
                self.procs[i].cpu_pct = (delta as f32 / total_delta as f32) * 100.0;
            }
        }

        for i in 0..count {
            self.prev_ticks[i] = self.procs[i].cpu_ticks;
        }

        self.proc_count = count;
        self.prev_poll_ns = self.last_poll_ns;
        self.last_poll_ns = now;

        let mut ready = 0u32;
        let mut run = 0u32;
        let mut blocked = 0u32;
        let mut cpu_sum = 0.0f32;
        for i in 0..count {
            match self.procs[i].state {
                0 => ready += 1,
                1 => run += 1,
                2 => blocked += 1,
                _ => {}
            }
            cpu_sum += self.procs[i].cpu_pct;
        }
        self.ready_count = ready;
        self.run_count = run;
        self.blocked_count = blocked;
        self.total_cpu_pct = cpu_sum.min(100.0);

        let used_pct = if self.total_mem > 0 {
            ((self.total_mem - self.free_mem) * 100 / self.total_mem).min(100) as u8
        } else {
            0
        };
        self.load_history[self.load_head % HISTORY_LEN] = used_pct;
        self.cpu_history[self.load_head % HISTORY_LEN] = (self.total_cpu_pct as u8).min(100);
        self.load_head = self.load_head.wrapping_add(1);
    }

    pub fn process(&self, idx: usize) -> Option<&ProcessInfo> {
        if idx < self.proc_count {
            Some(&self.procs[idx])
        } else {
            None
        }
    }

    pub fn processes(&self) -> &[ProcessInfo] {
        &self.procs[..self.proc_count]
    }

    pub fn find_index_by_pid(&self, pid: u32) -> Option<usize> {
        self.procs[..self.proc_count].iter().position(|p| p.pid == pid)
    }

    fn find_prev_ticks(&self, pid: u32) -> u64 {
        for i in 0..self.proc_count {
            if self.procs[i].pid == pid {
                return self.prev_ticks[i];
            }
        }
        0
    }

    pub fn load_history_sample(&self, age: usize) -> u8 {
        if age >= HISTORY_LEN { return 0; }
        let idx = (self.load_head.wrapping_sub(1).wrapping_sub(age)) % HISTORY_LEN;
        self.load_history[idx]
    }

    pub fn cpu_history_sample(&self, age: usize) -> u8 {
        if age >= HISTORY_LEN { return 0; }
        let idx = (self.load_head.wrapping_sub(1).wrapping_sub(age)) % HISTORY_LEN;
        self.cpu_history[idx]
    }

    pub fn mem_used_mb(&self) -> u32 {
        ((self.total_mem.saturating_sub(self.free_mem)) / (1024 * 1024)) as u32
    }

    pub fn mem_total_mb(&self) -> u32 {
        (self.total_mem / (1024 * 1024)) as u32
    }
}
