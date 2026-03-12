use core::net::Ipv4Addr;

use morpheus_network::device::UnifiedNetDevice;
use morpheus_network::stack::{DnsQueryHandle, NetInterface, SocketHandle};

pub(super) static mut USER_NET_DRIVER: Option<UnifiedNetDevice> = None;
pub(super) static mut USER_NET_STACK: Option<NetInterface<UnifiedNetDevice>> = None;
static mut USER_NET_DMA: Option<morpheus_network::dma::DmaRegion> = None;
static mut USER_NET_TSC_FREQ: u64 = 0;
static mut USER_NET_HOSTNAME: [u8; 64] = [0; 64];
static mut USER_NET_HOSTNAME_LEN: usize = 0;

const MAX_TCP_HANDLES: usize = 128;
const MAX_UDP_HANDLES: usize = 128;
const MAX_DNS_QUERIES: usize = 64;

static mut USER_TCP_HANDLES: [Option<SocketHandle>; MAX_TCP_HANDLES] = [None; MAX_TCP_HANDLES];
static mut USER_UDP_HANDLES: [Option<SocketHandle>; MAX_UDP_HANDLES] = [None; MAX_UDP_HANDLES];
static mut USER_DNS_QUERIES: [Option<DnsQueryHandle>; MAX_DNS_QUERIES] = [None; MAX_DNS_QUERIES];

#[inline(always)]
pub(super) fn ip_from_nbo(ip: u32) -> Ipv4Addr {
    let [a, b, c, d] = ip.to_be_bytes();
    Ipv4Addr::new(a, b, c, d)
}

#[inline(always)]
pub(super) fn ip_to_nbo(ip: Ipv4Addr) -> u32 {
    u32::from_be_bytes(ip.octets())
}

#[inline(always)]
fn slot_to_user_handle(slot: usize) -> i64 {
    (slot as i64) + 1
}

#[inline(always)]
fn user_handle_to_slot(handle: i64, max: usize) -> Option<usize> {
    if handle <= 0 {
        return None;
    }
    let idx = (handle - 1) as usize;
    if idx < max {
        Some(idx)
    } else {
        None
    }
}

pub(super) unsafe fn set_activation_context(dma: morpheus_network::dma::DmaRegion, tsc_freq: u64) {
    USER_NET_DMA = Some(dma);
    USER_NET_TSC_FREQ = tsc_freq;
}

pub(super) unsafe fn activation_context() -> Option<(&'static morpheus_network::dma::DmaRegion, u64)> {
    let dma = USER_NET_DMA.as_ref()?;
    Some((dma, USER_NET_TSC_FREQ))
}

pub(super) unsafe fn user_net_driver_mut() -> Option<&'static mut UnifiedNetDevice> {
    if let Some(stack) = USER_NET_STACK.as_mut() {
        return Some(stack.device_mut());
    }
    USER_NET_DRIVER.as_mut()
}

pub(super) unsafe fn user_net_stack_mut() -> Option<&'static mut NetInterface<UnifiedNetDevice>> {
    USER_NET_STACK.as_mut()
}

pub(super) unsafe fn set_stack(stack: NetInterface<UnifiedNetDevice>) {
    USER_NET_STACK = Some(stack);
    USER_NET_DRIVER = None;
}

pub(super) unsafe fn has_driver() -> bool {
    USER_NET_DRIVER.is_some()
}

pub(super) unsafe fn clear_net_handle_tables() {
    USER_TCP_HANDLES.fill(None);
    USER_UDP_HANDLES.fill(None);
    USER_DNS_QUERIES.fill(None);
}

pub(super) unsafe fn alloc_tcp_slot(handle: SocketHandle) -> Option<i64> {
    for idx in 0..MAX_TCP_HANDLES {
        if USER_TCP_HANDLES[idx].is_none() {
            USER_TCP_HANDLES[idx] = Some(handle);
            return Some(slot_to_user_handle(idx));
        }
    }
    None
}

pub(super) unsafe fn get_tcp_slot(handle: i64) -> Option<SocketHandle> {
    let idx = user_handle_to_slot(handle, MAX_TCP_HANDLES)?;
    USER_TCP_HANDLES[idx]
}

pub(super) unsafe fn take_tcp_slot(handle: i64) -> Option<SocketHandle> {
    let idx = user_handle_to_slot(handle, MAX_TCP_HANDLES)?;
    USER_TCP_HANDLES[idx].take()
}

pub(super) unsafe fn set_tcp_slot(handle: i64, socket: SocketHandle) -> bool {
    let Some(idx) = user_handle_to_slot(handle, MAX_TCP_HANDLES) else {
        return false;
    };
    USER_TCP_HANDLES[idx] = Some(socket);
    true
}

pub(super) unsafe fn tcp_active_count() -> u32 {
    USER_TCP_HANDLES.iter().filter(|h| h.is_some()).count() as u32
}

pub(super) unsafe fn alloc_udp_slot(handle: SocketHandle) -> Option<i64> {
    for idx in 0..MAX_UDP_HANDLES {
        if USER_UDP_HANDLES[idx].is_none() {
            USER_UDP_HANDLES[idx] = Some(handle);
            return Some(slot_to_user_handle(idx));
        }
    }
    None
}

pub(super) unsafe fn get_udp_slot(handle: i64) -> Option<SocketHandle> {
    let idx = user_handle_to_slot(handle, MAX_UDP_HANDLES)?;
    USER_UDP_HANDLES[idx]
}

pub(super) unsafe fn take_udp_slot(handle: i64) -> Option<SocketHandle> {
    let idx = user_handle_to_slot(handle, MAX_UDP_HANDLES)?;
    USER_UDP_HANDLES[idx].take()
}

pub(super) unsafe fn alloc_dns_query_slot(handle: DnsQueryHandle) -> Option<i64> {
    for idx in 0..MAX_DNS_QUERIES {
        if USER_DNS_QUERIES[idx].is_none() {
            USER_DNS_QUERIES[idx] = Some(handle);
            return Some(slot_to_user_handle(idx));
        }
    }
    None
}

pub(super) unsafe fn get_dns_query_slot(handle: i64) -> Option<DnsQueryHandle> {
    let idx = user_handle_to_slot(handle, MAX_DNS_QUERIES)?;
    USER_DNS_QUERIES[idx]
}

pub(super) unsafe fn clear_dns_query_slot(handle: i64) {
    if let Some(idx) = user_handle_to_slot(handle, MAX_DNS_QUERIES) {
        USER_DNS_QUERIES[idx] = None;
    }
}

pub(super) unsafe fn set_hostname(name: *const u8, len: usize) -> i64 {
    if name.is_null() || len == 0 || len > 63 {
        return -1;
    }
    USER_NET_HOSTNAME_LEN = len;
    core::ptr::copy_nonoverlapping(name, USER_NET_HOSTNAME.as_mut_ptr(), len);
    USER_NET_HOSTNAME[len] = 0;
    0
}

pub(super) unsafe fn write_hostname_to(out: &mut morpheus_hwinit::NetConfigInfo) {
    if USER_NET_HOSTNAME_LEN > 0 {
        let n = USER_NET_HOSTNAME_LEN.min(63);
        out.hostname[..n].copy_from_slice(&USER_NET_HOSTNAME[..n]);
        out.hostname[n] = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_slot_roundtrip() {
        for i in 0..32usize {
            let h = slot_to_user_handle(i);
            assert_eq!(user_handle_to_slot(h, 64), Some(i));
        }
    }

    #[test]
    fn handle_slot_rejects_invalid_values() {
        assert_eq!(user_handle_to_slot(0, 8), None);
        assert_eq!(user_handle_to_slot(-5, 8), None);
        assert_eq!(user_handle_to_slot(999, 8), None);
    }

    #[test]
    fn ipv4_nbo_roundtrip() {
        let ip = Ipv4Addr::new(10, 0, 2, 15);
        let nbo = ip_to_nbo(ip);
        assert_eq!(nbo, 0x0A00_020F);
        assert_eq!(ip_from_nbo(nbo), ip);
    }
}
