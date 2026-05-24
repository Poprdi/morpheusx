//! USB Device Enumeration Layer
//!
//! Dynamic USB device discovery during early boot. Synchronously enumerates
//! every connected root port, recursing through USB hubs to find HID devices.
//!
//! # Boot Sequence Constraint
//! All enumeration MUST complete before `init_scheduler()` is called.
//!
//! # Hub handling
//! For real hardware (especially Intel PCH boards) where the chassis USB ports
//! sit behind an internal hub, finding the keyboard requires walking through
//! that hub. We do this with a two-phase scheme per hub:
//!   1. All class requests on the hub's EP0 — descriptor, port power, port
//!      reset for every connected downstream port.
//!   2. Then for each connected downstream port: `enable_slot`, `address_device`,
//!      recurse. Once we move on to a child slot, we never come back to the
//!      hub's EP0 (its dequeue pointer would be left pointing at TRBs the
//!      child overwrote).
//!
//! `address_device` writes each new slot's EP0 dequeue to the current ring
//! position with the current cycle bit, so the shared EP0 ring stays
//! coherent across slot transitions.

use crate::usb::controller::{XhciController, XhciError};
use crate::usb::hid;
use crate::usb::hub::{HubInfo, PORT_STAT_CONNECTION, USB_CLASS_HUB};
use crate::usb::regs::*;

/// Identifies a parent hub for nested enumeration — passed into a child
/// device's `address_device` so the controller's TT routing can find the
/// upstream HS hub responsible for forwarding LS/FS traffic.
#[derive(Debug, Clone, Copy)]
struct HubParent {
    slot_id: u8,
    port_num: u8,
    /// Number of 4-bit groups already consumed in the route string by hubs
    /// above this one. The new port enters the route string at this offset.
    route_depth_bits: u8,
}

/// USB device handle after enumeration.
#[derive(Debug, Clone, Copy)]
pub struct UsbInputDevice {
    pub slot_id: u8,
    pub interface_num: u8,
    pub protocol: u8,
    pub ep_in: u8,
    pub ep_out: u8,
    pub max_packet_size: u16,
}

/// Result of USB input device enumeration.
#[derive(Debug)]
pub struct InputEnumerationResult {
    pub keyboard: Option<UsbInputDevice>,
    pub mouse: Option<UsbInputDevice>,
}

/// Walk every root port and recursively enumerate everything plugged in.
///
/// # Boot Order Constraint
/// Must run AFTER `XhciController::new` and BEFORE the scheduler starts.
pub unsafe fn enumerate_and_bind_inputs(
    controller: &mut XhciController,
) -> Result<InputEnumerationResult, XhciError> {
    let mut result = InputEnumerationResult {
        keyboard: None,
        mouse: None,
    };

    let port_count = controller.max_ports;

    for root_port in 0..port_count {
        let speed = match probe_root_port(controller, root_port) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if let Err(_e) = controller.port_reset(root_port) {
            crate::serial::log_warn("USB", 211, "step: port_reset failed");
            continue;
        }

        if let Err(_e) =
            enumerate_device(controller, root_port, speed, 0, 0, None, &mut result)
        {
            crate::serial::log_warn("USB", 201, "port enumeration failed");
        }
    }

    Ok(result)
}

/// Read PORTSC and decide whether a device is present + what speed it links at.
unsafe fn probe_root_port(controller: &XhciController, port: u8) -> Result<u8, XhciError> {
    let addr = controller.portsc(port);
    let ps = crate::cpu::mmio::read32(addr);

    if ps & PORTSC_CCS == 0 {
        return Err(XhciError::PortResetNoCCS);
    }

    let speed = ((ps >> PORTSC_SPEED_SHIFT) & 0xF) as u8;
    if speed == 0 {
        return Err(XhciError::PortResetNoLink);
    }

    Ok(speed)
}

/// Enumerate one device that has already been reset (root or hub-downstream).
///
/// If the device turns out to be a USB hub, this recurses into each connected
/// downstream port. Otherwise it inspects the config descriptor for HID
/// interfaces and stores them into `result`.
///
/// * `root_port` — 0-based root-hub port that this entire branch traverses.
///   Stays constant across the whole recursion (xHCI slot context records
///   the root-port, not intermediate hubs).
/// * `speed` — link speed of the device at THIS level (HS for the hub, the
///   downstream port's reset-detected speed for the child).
/// * `route` — xHCI 20-bit route string accumulated so far.
/// * `route_depth_bits` — number of 4-bit groups already consumed in `route`
///   (0 for root-port devices, 4 for a device one hub deep, 8 for two, etc.).
/// * `parent` — `None` for root-port devices; `Some(...)` for hub-downstream.
unsafe fn enumerate_device(
    controller: &mut XhciController,
    root_port: u8,
    speed: u8,
    route: u32,
    route_depth_bits: u8,
    parent: Option<HubParent>,
    result: &mut InputEnumerationResult,
) -> Result<(), XhciError> {
    // ── enable_slot ──
    let slot_id = match controller.enable_slot() {
        Ok(v) => v,
        Err(e) => {
            crate::serial::log_warn("USB", 212, "step: enable_slot failed");
            return Err(e);
        }
    };
    controller.slot_id = slot_id;

    // ── address_device ──
    let (parent_slot, parent_port) = match parent {
        Some(p) => (p.slot_id, p.port_num),
        None => (0, 0),
    };
    if let Err(e) =
        controller.address_device(root_port, speed, route, parent_slot, parent_port)
    {
        crate::serial::log_warn("USB", 213, "step: address_device failed");
        return Err(e);
    }

    // ── get device descriptor ──
    let desc_ptr = match controller.get_device_descriptor() {
        Ok(p) => p,
        Err(e) => {
            crate::serial::log_warn("USB", 214, "step: get_device_descriptor failed");
            return Err(e);
        }
    };
    let dev_class = core::ptr::read_volatile(desc_ptr.add(4));
    let dev_subclass = core::ptr::read_volatile(desc_ptr.add(5));
    let dev_proto = core::ptr::read_volatile(desc_ptr.add(6));

    if dev_class == USB_CLASS_HUB {
        crate::serial::log_ok("USB", 220, "step: hub detected, recursing");
        return enumerate_hub_downstream(
            controller,
            slot_id,
            root_port,
            route,
            route_depth_bits,
            dev_proto,
            result,
        );
    }

    // ── not a hub: pull config descriptor and look for HID ──
    let cfg_ptr = match controller.get_config_descriptor(9) {
        Ok(p) => p,
        Err(e) => {
            crate::serial::log_warn("USB", 215, "step: get_config_descriptor(9) failed");
            return Err(e);
        }
    };
    let total_len = u16::from_le_bytes([
        core::ptr::read_volatile(cfg_ptr.add(2)),
        core::ptr::read_volatile(cfg_ptr.add(3)),
    ]);
    if let Err(e) = controller.get_config_descriptor(total_len.min(512)) {
        crate::serial::log_warn("USB", 216, "step: get_config_descriptor(full) failed");
        return Err(e);
    }

    let hid_iface = controller.find_hid_interface(desc_ptr);

    if let Some(hid) = hid_iface {
        crate::serial::log_ok("USB", 219, "step: HID interface located");
        let dci_in = (hid.ep_in & 0x7F) * 2 + 1;
        let dci_out = if hid.ep_out != 0 {
            (hid.ep_out & 0x7F) * 2
        } else {
            0
        };

        if let Err(e) =
            controller.configure_endpoints(dci_in, dci_out, hid.max_packet_in, 0)
        {
            crate::serial::log_warn("USB", 217, "step: configure_endpoints failed");
            return Err(e);
        }
        let _ = controller.set_hid_idle(hid.interface_num);

        let dev = UsbInputDevice {
            slot_id,
            interface_num: hid.interface_num,
            protocol: hid.protocol,
            ep_in: hid.ep_in,
            ep_out: hid.ep_out,
            max_packet_size: hid.max_packet_in,
        };
        match dev.protocol {
            hid::USB_PROTOCOL_KEYBOARD => result.keyboard = Some(dev),
            hid::USB_PROTOCOL_MOUSE => result.mouse = Some(dev),
            _ => {}
        }
        Ok(())
    } else {
        crate::serial::log_info("USB", 218, "step: enumerated, no HID interface");
        crate::serial::puts("[USB-DBG] devclass=");
        crate::serial::puts_hex_u8(dev_class);
        crate::serial::puts(" devsub=");
        crate::serial::puts_hex_u8(dev_subclass);
        crate::serial::puts(" devproto=");
        crate::serial::puts_hex_u8(dev_proto);
        crate::serial::puts("\n");
        Ok(())
    }
}

/// Two-phase hub enumeration: finish all class-specific work on the hub's EP0
/// before any child takes over the shared EP0 ring.
unsafe fn enumerate_hub_downstream(
    controller: &mut XhciController,
    hub_slot: u8,
    root_port: u8,
    route: u32,
    route_depth_bits: u8,
    dev_proto: u8,
    result: &mut InputEnumerationResult,
) -> Result<(), XhciError> {
    // Hubs deeper than 5 levels can't be expressed in xHCI's 20-bit route string.
    if route_depth_bits >= 20 {
        crate::serial::log_warn("USB", 240, "hub: route string depth exceeded");
        return Ok(());
    }

    // ── Hub descriptor ──
    let hub_info: HubInfo = match controller.get_hub_descriptor() {
        Ok(h) => h,
        Err(e) => {
            crate::serial::log_warn("USB", 230, "hub: get_hub_descriptor failed");
            return Err(e);
        }
    };
    let multi_tt = dev_proto == 2;

    // ── Slot context update — set Hub bit, Number of Ports, TT Think Time ──
    if let Err(e) =
        controller.configure_hub_slot(hub_info.num_ports, multi_tt, hub_info.tt_think_time)
    {
        crate::serial::log_warn("USB", 231, "hub: configure_hub_slot failed");
        return Err(e);
    }

    // ── Phase 1a: power on every downstream port ──
    for p in 1..=hub_info.num_ports {
        if let Err(_e) = controller.hub_port_power_on(p) {
            crate::serial::log_warn("USB", 232, "hub: SET_FEATURE(PORT_POWER) failed");
            // continue — some ports may still come up
        }
    }
    controller.delay_ms((hub_info.pwr_on_2_pwr_good_ms as u64).saturating_add(100));

    // ── Phase 1b: discover which ports have devices and reset them ──
    // Records (port_number, link_speed). Capped at 15 (the route-string nibble
    // can't address a hub with more than 15 ports in any case).
    let mut connected: [(u8, u8); 15] = [(0, 0); 15];
    let mut n_connected = 0usize;

    for p in 1..=hub_info.num_ports.min(15) {
        let status = match controller.hub_port_get_status(p) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if status & PORT_STAT_CONNECTION == 0 {
            continue;
        }
        match controller.hub_port_reset(p) {
            Ok(dspeed) => {
                connected[n_connected] = (p, dspeed);
                n_connected += 1;
            }
            Err(_) => {
                crate::serial::log_warn("USB", 233, "hub: downstream port reset failed");
            }
        }
    }

    // ── Phase 2: enumerate each connected downstream device.
    // Each iteration takes over the EP0 ring; we never return to the hub
    // for more class requests after this point.
    for i in 0..n_connected {
        let (down_port, dspeed) = connected[i];
        let new_route = route | (((down_port as u32) & 0xF) << route_depth_bits);
        let new_depth = route_depth_bits + 4;
        let new_parent = HubParent {
            slot_id: hub_slot,
            port_num: down_port,
            route_depth_bits: new_depth,
        };

        if let Err(_e) = enumerate_device(
            controller,
            root_port,
            dspeed,
            new_route,
            new_depth,
            Some(new_parent),
            result,
        ) {
            crate::serial::log_warn("USB", 234, "hub: downstream enumeration failed");
            // Keep trying other ports — the keyboard might be the next one
        }
    }

    Ok(())
}
