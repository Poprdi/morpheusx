#![no_std]
#![no_main]

use libmorpheus::entry;
use libmorpheus::io::{print, println};
use libmorpheus::net::{self, Ipv4Addr, TcpState, TcpStream};
use libmorpheus::time;

entry!(main);

fn main() -> i32 {
    println("[netcheck] start");

    match net::net_activate() {
        Ok(rc) => {
            if rc == 0 {
                println("[netcheck] network activated");
            } else {
                println("[netcheck] network already active");
            }
        }
        Err(_) => {
            println("[netcheck] activation failed");
            return 1;
        }
    }

    if net::net_dhcp().is_err() {
        println("[netcheck] dhcp request failed");
        return 2;
    }

    let ip = match wait_for_lease(6000) {
        Some(ip) => ip,
        None => {
            println("[netcheck] no dhcp lease");
            return 3;
        }
    };
    println("[netcheck] lease acquired");
    print("[netcheck] ip=");
    print_ipv4(ip);
    println("");

    let pre = net::net_stats().ok();

    let dns_ip = match resolve_with_timeout("example.com", 6000) {
        Some(ip) => ip,
        None => {
            println("[netcheck] dns resolve failed");
            return 4;
        }
    };
    print("[netcheck] dns example.com=");
    print_ipv4(dns_ip);
    println("");

    let stream = match TcpStream::connect(Ipv4Addr::from_nbo(dns_ip), 80) {
        Ok(s) => s,
        Err(_) => {
            println("[netcheck] tcp connect failed");
            return 5;
        }
    };

    if !wait_connected(&stream, 5000) {
        println("[netcheck] tcp handshake timeout");
        return 6;
    }

    let req = b"GET / HTTP/1.0\r\nHost: example.com\r\nConnection: close\r\n\r\n";
    if stream.send_all(req).is_err() {
        println("[netcheck] tcp send failed");
        return 7;
    }

    let mut buf = [0u8; 512];
    let n = match recv_with_timeout(&stream, &mut buf, 5000) {
        Some(n) if n > 0 => n,
        _ => {
            println("[netcheck] tcp recv timeout");
            return 8;
        }
    };

    if n >= 4 && &buf[0..4] == b"HTTP" {
        println("[netcheck] http response received");
    } else {
        println("[netcheck] recv data but no http prefix");
    }

    if let (Some(a), Ok(b)) = (pre, net::net_stats()) {
        let tx = b.tx_packets.saturating_sub(a.tx_packets);
        let rx = b.rx_packets.saturating_sub(a.rx_packets);
        print("[netcheck] delta tx=");
        print_u64(tx);
        print(" rx=");
        print_u64(rx);
        println("");
    }

    println("[netcheck] PASS");
    0
}

fn wait_for_lease(timeout_ms: u64) -> Option<u32> {
    let start = time::uptime_ms();
    loop {
        let now = time::uptime_ms();
        let _ = net::nic_refill();
        let _ = net::net_poll_drive(now);
        if let Ok(cfg) = net::net_config() {
            if cfg.ipv4_addr != 0 {
                return Some(cfg.ipv4_addr);
            }
        }
        if now.saturating_sub(start) >= timeout_ms {
            return None;
        }
        libmorpheus::process::sleep(10);
    }
}

fn resolve_with_timeout(host: &str, timeout_ms: u64) -> Option<u32> {
    let q = net::dns_start(host).ok()?;
    let start = time::uptime_ms();
    loop {
        let now = time::uptime_ms();
        let _ = net::nic_refill();
        let _ = net::net_poll_drive(now);
        match net::dns_poll(q) {
            Ok(Some(ip)) => return Some(ip),
            Ok(None) => {}
            Err(_) => return None,
        }
        if now.saturating_sub(start) >= timeout_ms {
            return None;
        }
        libmorpheus::process::sleep(10);
    }
}

fn wait_connected(stream: &TcpStream, timeout_ms: u64) -> bool {
    let start = time::uptime_ms();
    loop {
        let now = time::uptime_ms();
        let _ = net::net_poll_drive(now);
        match stream.state() {
            Ok(TcpState::Established) => return true,
            Ok(TcpState::Closed) => return false,
            _ => {}
        }
        if now.saturating_sub(start) >= timeout_ms {
            return false;
        }
        libmorpheus::process::sleep(10);
    }
}

fn recv_with_timeout(stream: &TcpStream, buf: &mut [u8], timeout_ms: u64) -> Option<usize> {
    let start = time::uptime_ms();
    loop {
        let now = time::uptime_ms();
        let _ = net::net_poll_drive(now);
        match net::tcp_recv(stream.handle(), buf) {
            Ok(n) if n > 0 => return Some(n),
            Ok(_) => {}
            Err(_) => return None,
        }
        if now.saturating_sub(start) >= timeout_ms {
            return None;
        }
        libmorpheus::process::sleep(10);
    }
}

fn print_u64(v: u64) {
    if v == 0 {
        print("0");
        return;
    }
    let mut n = v;
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    if let Ok(s) = core::str::from_utf8(&buf[i..]) {
        print(s);
    }
}

fn print_u8(v: u8) {
    print_u64(v as u64);
}

fn print_ipv4(nbo: u32) {
    let ip = Ipv4Addr::from_nbo(nbo).octets();
    print_u8(ip[0]);
    print(".");
    print_u8(ip[1]);
    print(".");
    print_u8(ip[2]);
    print(".");
    print_u8(ip[3]);
}
