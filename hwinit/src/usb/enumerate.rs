//! USB Device Enumeration Layer
//!
//! Handles dynamic USB device discovery and class binding during early boot.
//! This runs synchronously BEFORE the scheduler is enabled, ensuring all
//! input devices are ready before any user processes start.
//!
//! # Boot Sequence Constraint
//! All enumeration MUST complete before `init_scheduler()` is called.
//! Scheduling must not depend on asynchronous device discovery.
//!
//! # Device Detection Flow
//!
//! ```text
//! Enumerate USB Host Controller
//!     │
//!     ▼
//! Scan Ports
//!     │
//!     ▼
//! For each connected device:
//!     │
//!     ├── Get Device Descriptor
//!     │
//!     ├── Set Address
//!     │
//!     ├── Get Configuration Descriptor
//!     │
//!     ├── Parse Interface Descriptors
//!     │
//!     ├── Bind to supported class driver:
//!     │   ├── HID (Keyboard/Mouse)
//!     │   ├── Mass Storage
//!     │   └── Generic
//!     │
//!     └── Register with input subsystem
//! ```

use crate::usb::controller::{XhciController, XhciError};
use crate::usb::dma;
use crate::usb::hid::{self, HIDInterface};
use crate::usb::regs::*;

/// USB device handle after enumeration
#[derive(Debug)]
pub struct UsbInputDevice {
    pub interface_num: u8,
    pub protocol: u8,
    pub ep_in: u8,
    pub ep_out: u8,
    pub max_packet_size: u16,
}

/// Result of USB input device enumeration
#[derive(Debug)]
pub struct InputEnumerationResult {
    pub keyboard: Option<UsbInputDevice>,
    pub mouse: Option<UsbInputDevice>,
}

/// Enumerate all USB devices and bind input handlers.
/// This is the main entry point called during early boot.
///
/// # Boot Order Constraint
/// This function MUST be called AFTER xHCI controller initialization
/// but BEFORE `init_scheduler()`. It performs synchronous enumeration
/// and blocks until all devices are discovered and bound.
pub unsafe fn enumerate_and_bind_inputs(
    controller: &mut XhciController,
) -> Result<InputEnumerationResult, XhciError> {
    let mut result = InputEnumerationResult {
        keyboard: None,
        mouse: None,
    };

    let port_count = controller.max_ports;

    for port in 0..port_count {
        if let Ok(speed) = probe_port(controller, port) {
            match enumerate_port(controller, port, speed) {
                Ok(Some(device)) => match device.protocol {
                    hid::USB_PROTOCOL_KEYBOARD => {
                        result.keyboard = Some(device);
                    }
                    hid::USB_PROTOCOL_MOUSE => {
                        result.mouse = Some(device);
                    }
                    _ => {
                        if let Some(hid_iface) = detect_hid_interface(controller) {
                            if has_keyboard_interface(&hid_iface) {
                                result.keyboard = Some(UsbInputDevice {
                                    interface_num: hid_iface.interface_num,
                                    protocol: hid::USB_PROTOCOL_KEYBOARD,
                                    ep_in: hid_iface.ep_in,
                                    ep_out: hid_iface.ep_out,
                                    max_packet_size: hid_iface.max_packet_in,
                                });
                            }
                            if has_mouse_interface(&hid_iface) {
                                result.mouse = Some(UsbInputDevice {
                                    interface_num: hid_iface.interface_num,
                                    protocol: hid::USB_PROTOCOL_MOUSE,
                                    ep_in: hid_iface.ep_in,
                                    ep_out: hid_iface.ep_out,
                                    max_packet_size: hid_iface.max_packet_in,
                                });
                            }
                        }
                    }
                },
                Ok(None) => {}
                Err(_e) => {
                    crate::serial::log_warn("USB", 201, "port enumeration failed");
                }
            }
        }
    }

    Ok(result)
}

/// Probe a port to see if a device is connected.
unsafe fn probe_port(controller: &XhciController, port: u8) -> Result<u8, XhciError> {
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

/// Enumerate a single device on a port.
///
/// Each step logs a distinct WARN line on failure so real-hardware bring-up
/// can identify exactly which xHCI operation died, without numeric codes
/// (which the framebuffer mirror does not render).
unsafe fn enumerate_port(
    controller: &mut XhciController,
    port: u8,
    speed: u8,
) -> Result<Option<UsbInputDevice>, XhciError> {
    if let Err(e) = controller.port_reset(port) {
        crate::serial::log_warn("USB", 211, "step: port_reset failed");
        return Err(e);
    }

    let slot_id = match controller.enable_slot() {
        Ok(v) => v,
        Err(e) => {
            crate::serial::log_warn("USB", 212, "step: enable_slot failed");
            return Err(e);
        }
    };
    controller.slot_id = slot_id;

    if let Err(e) = controller.address_device(port, speed) {
        crate::serial::log_warn("USB", 213, "step: address_device failed");
        return Err(e);
    }

    // Pull device descriptor — 18 bytes, lands in OFF_DESC
    let desc_ptr = match controller.get_device_descriptor() {
        Ok(p) => p,
        Err(e) => {
            crate::serial::log_warn("USB", 214, "step: get_device_descriptor failed");
            return Err(e);
        }
    };

    // Pull config descriptor — first 9 bytes to get wTotalLength
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

    // Pull the full config descriptor
    if let Err(e) = controller.get_config_descriptor(total_len.min(512)) {
        crate::serial::log_warn("USB", 216, "step: get_config_descriptor(full) failed");
        return Err(e);
    }

    // desc_ptr still points into OFF_DESC; re-read cfg from same buffer
    let hid_iface = controller.find_hid_interface(desc_ptr);

    if let Some(hid) = hid_iface {
        crate::serial::log_ok("USB", 219, "step: HID interface located");

        // DCI = endpoint_number * 2 + direction (1=IN, 0=OUT)
        let dci_in = (hid.ep_in & 0x7F) * 2 + 1;
        let dci_out = if hid.ep_out != 0 {
            (hid.ep_out & 0x7F) * 2
        } else {
            0
        };

        if let Err(e) = controller.configure_endpoints(dci_in, dci_out, hid.max_packet_in, 0) {
            crate::serial::log_warn("USB", 217, "step: configure_endpoints failed");
            return Err(e);
        }
        // Ignore idle errors — some devices don't support it
        let _ = controller.set_hid_idle(hid.interface_num);

        Ok(Some(UsbInputDevice {
            interface_num: hid.interface_num,
            protocol: hid.protocol,
            ep_in: hid.ep_in,
            ep_out: hid.ep_out,
            max_packet_size: hid.max_packet_in,
        }))
    } else {
        // Successfully addressed and pulled descriptors, but no HID interface
        // present in the configuration. Most common cause on real hardware:
        // the device is a USB hub (class 0x09) acting as the intermediary
        // between the controller and the real keyboard/mouse downstream.
        crate::serial::log_info("USB", 218, "step: enumerated, no HID interface (hub?)");
        Ok(None)
    }
}

/// Detect HID interface from current configuration.
unsafe fn detect_hid_interface(controller: &mut XhciController) -> Option<HIDInterface> {
    let desc_ptr = controller.get_config_descriptor(512).ok()?;
    controller.find_hid_interface(desc_ptr)
}

fn has_keyboard_interface(hid: &HIDInterface) -> bool {
    hid.ep_in != 0 && hid.protocol == hid::USB_PROTOCOL_KEYBOARD
}

fn has_mouse_interface(hid: &HIDInterface) -> bool {
    hid.ep_in != 0 && hid.protocol == hid::USB_PROTOCOL_MOUSE
}
