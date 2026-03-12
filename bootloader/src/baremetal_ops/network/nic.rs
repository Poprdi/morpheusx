use morpheus_network::device::NetworkDevice;

use super::state;

pub(super) unsafe fn log_pci_network_candidates() {
    let mut found = 0u32;
    for bus in 0..=255u8 {
        for dev in 0..32u8 {
            for func in 0..8u8 {
                let addr = morpheus_hwinit::PciAddr::new(bus, dev, func);
                let vendor = morpheus_hwinit::pci_cfg_read16(addr, 0x00);
                if vendor == 0xFFFF {
                    if func == 0 {
                        break;
                    }
                    continue;
                }

                let class = morpheus_hwinit::pci_cfg_read8(addr, 0x0B);
                let subclass = morpheus_hwinit::pci_cfg_read8(addr, 0x0A);
                if class != 0x02 {
                    if func == 0 {
                        let header = morpheus_hwinit::pci_cfg_read8(addr, 0x0E);
                        if (header & 0x80) == 0 {
                            break;
                        }
                    }
                    continue;
                }

                let device = morpheus_hwinit::pci_cfg_read16(addr, 0x02);
                found = found.wrapping_add(1);
                morpheus_hwinit::serial::puts("[INFO] [NET] pci net candidate bdf=");
                morpheus_hwinit::serial::put_hex64(
                    ((bus as u64) << 16) | ((dev as u64) << 8) | func as u64,
                );
                morpheus_hwinit::serial::puts(" ven=");
                morpheus_hwinit::serial::put_hex64(vendor as u64);
                morpheus_hwinit::serial::puts(" dev=");
                morpheus_hwinit::serial::put_hex64(device as u64);
                morpheus_hwinit::serial::puts(" sub=");
                morpheus_hwinit::serial::put_hex64(subclass as u64);
                morpheus_hwinit::serial::puts("\n");

                if func == 0 {
                    let header = morpheus_hwinit::pci_cfg_read8(addr, 0x0E);
                    if (header & 0x80) == 0 {
                        break;
                    }
                }
            }
        }
    }

    if found == 0 {
        morpheus_hwinit::serial::log_warn("NET", 953, "pci scan found zero class-0x02 devices");
    } else {
        morpheus_hwinit::serial::log_info("NET", 954, "pci net candidates logged");
    }
}

pub(super) unsafe fn user_net_tx(frame: *const u8, len: usize) -> i64 {
    let Some(driver) = state::user_net_driver_mut() else {
        return -1;
    };
    let frame = core::slice::from_raw_parts(frame, len);
    if driver.transmit(frame).is_ok() {
        0
    } else {
        -1
    }
}

pub(super) unsafe fn user_net_rx(buf: *mut u8, buf_len: usize) -> i64 {
    let Some(driver) = state::user_net_driver_mut() else {
        return -1;
    };
    let buf = core::slice::from_raw_parts_mut(buf, buf_len);
    match driver.receive(buf) {
        Ok(Some(n)) => n as i64,
        Ok(None) => 0,
        Err(_) => -1,
    }
}

pub(super) unsafe fn user_net_link_up() -> i64 {
    let Some(driver) = state::user_net_driver_mut() else {
        return 0;
    };
    driver.link_up() as i64
}

pub(super) unsafe fn user_net_mac(out: *mut u8) -> i64 {
    let Some(driver) = state::user_net_driver_mut() else {
        return -1;
    };
    let mac = driver.mac_address();
    core::ptr::copy_nonoverlapping(mac.as_ptr(), out, 6);
    0
}

pub(super) unsafe fn user_net_refill() -> i64 {
    let Some(driver) = state::user_net_driver_mut() else {
        return -1;
    };
    driver.refill_rx_queue();
    driver.collect_tx_completions();
    0
}

pub(super) unsafe fn user_net_ctrl(_cmd: u32, _arg: u64) -> i64 {
    -1
}
