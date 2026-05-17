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

use crate::cpu::mmio;
use crate::usb::controller::{XhciController, XhciError};
use crate::usb::dma;
use crate::usb::enum_::{self, UsbDevice};
use crate::usb::hid::{self, HIDInterface};
use crate::usb::regs::*;

/// USB device handle after enumeration
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

    // Scan all ports for connected devices
    let port_count = controller.max_ports;
    
    for port in 0..port_count {
        // Check if port has a device connected
        if let Ok(speed) = probe_port(controller, port) {
            // Device detected - enumerate it
            match enumerate_port(controller, port, speed) {
                Ok(Some(device)) => {
                    // Bind based on device class
                    match device.protocol {
                        hid::USB_PROTOCOL_KEYBOARD => {
                            result.keyboard = Some(device);
                        }
                        hid::USB_PROTOCOL_MOUSE => {
                            result.mouse = Some(device);
                        }
                        _ => {
                            // Try to detect HID interface from device
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
                    }
                }
                Ok(None) => {
                    // Non-input device, continue scanning
                }
                Err(e) => {
                    // Log error but continue scanning other ports
                    crate::serial::log_warn("USB", 201, "port enumeration failed");
                    let _ = e;
                }
            }
        }
    }

    // If no USB keyboard found, PS/2 keyboard remains the fallback
    // If no USB mouse found, PS/2 mouse remains the fallback
    
    Ok(result)
}

/// Probe a port to see if a device is connected.
/// Returns the link speed if a device is detected.
unsafe fn probe_port(controller: &XhciController, port: u8) -> Result<u8, XhciError> {
    let addr = controller.portsc(port);
    let ps = mmio::read32(addr);

    // Check for device presence
    if ps & PORTSC_CCS == 0 {
        return Err(XhciError::PortResetNoCCS);
    }

    // Wait briefly for link training to complete
    let speed = ((ps >> PORTSC_SPEED_SHIFT) & 0xF) as u8;
    if speed == 0 {
        return Err(XhciError::PortResetNoLink);
    }

    Ok(speed)
}

/// Enumerate a single device on a port.
/// Performs address assignment and configuration.
unsafe fn enumerate_port(
    controller: &mut XhciController,
    port: u8,
    speed: u8,
) -> Result<Option<UsbInputDevice>, XhciError> {
    // Reset port to attach device
    controller.port_reset(port)?;


    // Enable slot
    let slot_id = controller.enable_slot()?;
    controller.slot_id = slot_id;

    // Set address 0
    controller.set_address(slot_id, port, speed, 0)?;

    // Get device descriptor
    let desc_buf = dma::dma_base() + dma::OFF_DESCRIPTOR as u64;
    controller.get_device_descriptor(desc_buf)?;

    // Get configuration descriptor
    controller.get_configuration_descriptor(desc_buf + 64)?;

    // Parse configuration to find HID interface
    let hid_iface = controller.find_hid_interface(desc_buf as *const u8)?;

    if let Some(hid) = hid_iface {
        // Configure the device
        controller.configure_endpoints(slot_id, &hid)?;

        // Set HID idle on interface
        controller.set_hid_idle(hid.interface_num)?;

        // Return the USB input device
        Ok(Some(UsbInputDevice {
            interface_num: hid.interface_num,
            protocol: hid.protocol,
            ep_in: hid.ep_in,
            ep_out: hid.ep_out,
            max_packet_size: hid.max_packet_in,
        }))
    } else {
        // No HID interface found, but device exists
        // Disable slot and continue
        Ok(None)
    }
}

/// Detect HID interface from current configuration.
unsafe fn detect_hid_interface(
    controller: &mut XhciController,
) -> Option<HIDInterface> {
    let desc_buf = dma::dma_base() + dma::OFF_DESCRIPTOR as u64;
    
    // Get configuration descriptor
    if controller.get_configuration_descriptor(desc_buf + 64).is_err() {
        return None;
    }
    
    controller.find_hid_interface(desc_buf as *const u8)
}

/// Check if HID interface has keyboard report descriptor.
fn has_keyboard_interface(hid: &HIDInterface) -> bool {
    // Basic check: boot protocol keyboard uses endpoint 0x81 (IN)
    hid.ep_in != 0 && hid.protocol == hid::USB_PROTOCOL_KEYBOARD
}

/// Check if HID interface has mouse report descriptor.
fn has_mouse_interface(hid: &HIDInterface) -> bool {
    // Basic check: boot protocol mouse uses endpoint 0x81 (IN)
    hid.ep_in != 0 && hid.protocol == hid::USB_PROTOCOL_MOUSE
}

// EXTEND XHCI CONTROLLER FOR ENUMERATION

use crate::usb::controller::XhciController;

impl XhciController {
    /// Enable a device slot.
    pub unsafe fn enable_slot(&mut self) -> Result<u8, XhciError> {
        use crate::usb::rings::{vw32, CmdTrb, TRB_TYPE};

        // Allocate slot via command
        let cr = self.dma_base + dma::OFF_CMD_RING as u64;
        let trb = CmdTrb {
            data: 0,
            status: 0,
            control: TRB_TYPE::ENABLE_SLOTS.0 << 10,
        };
        
        // Enqueue command
        self.cmd_ring.enqueue(trb);
        self.ring_cmd_doorbell();

        // Wait for completion
        let (slot_id, _) = self.wait_cmd(1000)?;
        
        Ok(slot_id)
    }

    /// Set USB address for a device.
    pub unsafe fn set_address(
        &mut self,
        slot_id: u8,
        port: u8,
        speed: u8,
        address: u8,
    ) -> Result<(), XhciError> {
        use crate::usb::rings::{vw32, CmdTrb, TRB_TYPE};

        let input_ctx = self.dma_base + dma::OFF_CONTEXT as u64;
        
        // Build input context
        let slot_ctx = input_ctx;
        let ep_ctx = input_ctx + self.ctx_size as u64;
        
        // Slot context: route string, speed, port, hub, address
        let route = if speed >= 4 { 0u64 } else { (1u64 << 22) };
        let slot_ctrl = (route & 0x3F) | ((speed as u64) << 20) | ((port as u64) << 16) | (1u64 << 25);
        vw32(slot_ctx, slot_ctrl as u32);
        vw32(slot_ctx + 4, (slot_ctrl >> 32) as u32);
        vw32(slot_ctx + 8, (address as u64) << 16);
        
        // Route is in bits 0-5 of dword 0
        // Speed is bits 20-23
        // Port is bits 16-19 (for SS: bits 16-25)
        // Address is bits 16-23 of dword 2
        
        // Clear EP0 state
        vw32(ep_ctx, 0);
        vw32(ep_ctx + 4, 0);
        vw32(ep_ctx + 8, 0);
        vw32(ep_ctx + 12, 0);
        
        // Issue address device command
        let trb = CmdTrb {
            data: input_ctx,
            status: 0,
            control: ((slot_id as u32) << 24) | (TRB_TYPE::ADDRESS_DEVICE.0 as u32 << 10) | 0x12,
        };
        
        self.cmd_ring.enqueue(trb);
        self.ring_cmd_doorbell();
        
        let (_, _) = self.wait_cmd(5000)?;
        
        Ok(())
    }

    /// Get device descriptor from address 0.
    pub unsafe fn get_device_descriptor(
        &mut self,
        buf: u64,
    ) -> Result<(), XhciError> {
        use crate::usb::control::*;
        
        // Read 8 bytes of device descriptor (just bcdUSB, idVendor, idProduct)
        self.control_in(
            0x80, // IN, standard
            0x06, // GET_DESCRIPTOR
            0x0100, // device descriptor
            0,
            8,
            buf as *mut u8,
        )?;
        
        Ok(())
    }


    /// Get configuration descriptor and all associated interface/endpoint descriptors.
    pub unsafe fn get_configuration_descriptor(
        &mut self,
        buf: u64,
    ) -> Result<(), XhciError> {
        use crate::usb::control::*;
        
        // First get just the config descriptor to get wTotalLength
        self.control_in(
            0x80,
            0x06,
            0x0200, // configuration descriptor
            0,
            9,
            buf as *mut u8,
        )?;
        
        // Read total length from config descriptor (offset 10 for wTotalLength)
        let total_len = {
            let mut b = [0u8; 2];
            core::ptr::read_bytes(b.as_mut_ptr(), 0);
            core::ptr::copy_nonoverlapping((buf + 10) as *const u8, b.as_mut_ptr(), 2);
            u16::from_le_bytes(b) as usize
        };
        
        // Now get the full configuration
        self.control_in(
            0x80,
            0x06,
            0x0200,
            0,
            total_len.min(1024) as u16,
            buf as *mut u8,
        )?;
        
        Ok(())
    }

    /// Configure endpoints for HID device.
    pub unsafe fn configure_endpoints(
        &mut self,
        slot_id: u8,
        hid: &HIDInterface,
    ) -> Result<(), XhciError> {
        use crate::usb::rings::{vw32, CmdTrb, TRB_TYPE};


        let input_ctx = self.dma_base + dma::OFF_CONTEXT as u64;
        
        // Build input context for Configure Endpoint command
        // This sets up EP0 (control) and the HID interrupt endpoints
        
        // Zero the context
        for i in 0..(self.ctx_size as usize * 32) {
            core::ptr::write_byte((input_ctx + i as u64) as *mut u8, 0);
        }
        
        // Slot context: add flags (4)
        let slot_ctrl = vw32(input_ctx);
        vw32(input_ctx, slot_ctrl | 0x03); // Add Interrupter, Speed
        
        // EP0 context (control endpoint)
        let ep0_ctx = input_ctx + 0x20;
        vw32(ep0_ctx, 0x00000800); // Type: Control, max packet 8
        vw32(ep0_ctx + 4, 0); // Reserved
        vw32(ep0_ctx + 8, 0x40000000); // DCI 0
        vw32(ep0_ctx + 12, 0);
        
        // Interrupt IN endpoint
        let ep_in_ctx = input_ctx + (hid.ep_in as u64) * self.ctx_size as u64;
        let max_pkt = hid.max_packet_in.min(1024);
        let ep_type = 0x04; // Interrupt IN
        vw32(ep_in_ctx, (ep_type << 3) | (max_pkt as u64 << 16));
        vw32(ep_in_ctx + 4, 0);
        vw32(ep_in_ctx + 8, (self.dma_base + dma::OFF_XFER_EP_IN as u64) as u32);
        vw32(ep_in_ctx + 12, ((self.dma_base + dma::OFF_XFER_EP_IN as u64) >> 32) as u32);
        
        // Issue configure endpoints command
        let trb = CmdTrb {
            data: input_ctx,
            status: 0,
            control: ((slot_id as u32) << 24) | (TRB_TYPE::CONFIGURE_ENDPOINT.0 as u32 << 10),
        };
        
        self.cmd_ring.enqueue(trb);
        self.ring_cmd_doorbell();
        
        let (_, _) = self.wait_cmd(5000)?;
        
        Ok(())
    }
}
