//! Runtime USB HID polling — keeps the xHC alive past Phase 9 so the
//! bootloader's input loop can fetch keyboard and mouse reports.
//!
//! Keyboard and mouse live on the same controller and share one event ring, so
//! `poll_input()` drains that ring once per tick and dispatches each transfer
//! event to the matching device by slot id. Each device has its own transfer
//! ring + report buffer (keyboard: OFF_XFER_BIN/OFF_REPORT, mouse:
//! OFF_XFER_MOUSE/OFF_REPORT_MOUSE) so both stay armed concurrently — a single
//! shared ring would corrupt the other slot's outstanding transfer.

use crate::controller::XhciController;
use crate::dma;
use crate::enumerate::UsbInputDevice;
use crate::regs::{TRB_IOC, TRB_ISP, TRB_NORMAL};
use crate::usb::hid::keyboard::{parse_keyboard_report, KeyboardReport};
use crate::usb::hid::mouse::dispatch_mouse;
use crate::usb::hid::HIDInterface;
use morpheus_kernel::sync::SpinLock;

static USB_CONTROLLER: SpinLock<Option<XhciController>> = SpinLock::new(None);
static USB_KEYBOARD: SpinLock<Option<UsbInputDevice>> = SpinLock::new(None);
static USB_MOUSE: SpinLock<Option<UsbInputDevice>> = SpinLock::new(None);

const ARM_FLAGS: u32 = TRB_NORMAL | TRB_IOC | TRB_ISP;

/// Move the xHC and the discovered HID devices into static storage so they
/// survive past Phase 9, and arm one interrupt-IN transfer on each.
///
/// # Safety
/// `controller` must be a fully enumerated xHCI controller with valid MMIO/DMA
/// mappings; call once from Phase 9 with no concurrent USB access.
pub unsafe fn install_runtime(
    controller: XhciController,
    keyboard: Option<UsbInputDevice>,
    mouse: Option<UsbInputDevice>,
) {
    {
        *USB_CONTROLLER.lock() = Some(controller);
    }
    {
        *USB_KEYBOARD.lock() = keyboard;
    }
    {
        *USB_MOUSE.lock() = mouse;
    }

    let mut ctrl_guard = USB_CONTROLLER.lock();
    if let Some(controller) = ctrl_guard.as_mut() {
        if let Some(k) = USB_KEYBOARD.lock().as_ref() {
            arm_device(controller, k, false);
        }
        if let Some(m) = USB_MOUSE.lock().as_ref() {
            arm_device(controller, m, true);
        }
    }
}

#[inline]
fn dci_of(dev: &UsbInputDevice) -> u32 {
    ((dev.ep_in & 0x7F) as u32) * 2 + 1
}

#[inline]
fn rings_of(mouse: bool) -> (usize, usize) {
    if mouse {
        (dma::OFF_XFER_MOUSE, dma::OFF_REPORT_MOUSE)
    } else {
        (dma::OFF_XFER_BIN, dma::OFF_REPORT)
    }
}

fn iface_of(dev: &UsbInputDevice) -> HIDInterface {
    HIDInterface {
        interface_num: dev.interface_num,
        protocol: dev.protocol,
        ep_in: dev.ep_in,
        ep_out: dev.ep_out,
        max_packet_in: dev.max_packet_size,
        report_desc_len: 0,
    }
}

/// EP State 2 = Halted, 4 = Error (xHCI 4.8.3). Both ignore doorbell rings
/// until RESET_ENDPOINT clears them — real LS keyboards/mice often land here
/// right after CONFIGURE_ENDPOINT.
unsafe fn ep_halted(controller: &XhciController, dev: &UsbInputDevice) -> bool {
    let cs = controller.ctx_size as u64;
    let out_ctx = controller.dma_base + dma::slot_out_ctx_offset(dev.slot_id) as u64;
    let ep_ctx = out_ctx + (dci_of(dev) as u64) * cs;
    let st = crate::rings::vr32(ep_ctx) & 0x7;
    st == 2 || st == 4
}

unsafe fn ring_enqueue(controller: &mut XhciController, mouse: bool, buf: u64, mpkt: u32) {
    if mouse {
        controller.mouse_ring.enqueue(buf, mpkt, ARM_FLAGS);
    } else {
        controller.bin.enqueue(buf, mpkt, ARM_FLAGS);
    }
}

/// Reset a halted endpoint if needed, then enqueue one interrupt-IN transfer.
unsafe fn arm_device(controller: &mut XhciController, dev: &UsbInputDevice, mouse: bool) {
    let dci = dci_of(dev);
    let (ring_off, report_off) = rings_of(mouse);

    if ep_halted(controller, dev) {
        if controller.reset_endpoint(dev.slot_id, dci).is_err() {
            crate::logger::warn("USB", 971, "reset_endpoint failed");
            return;
        }
        if mouse {
            controller.mouse_ring.reset();
        } else {
            controller.bin.reset();
        }
        let deq = ((controller.dma_base + ring_off as u64) & !0xF) | 1;
        if controller
            .set_tr_dequeue_pointer(dev.slot_id, dci, deq)
            .is_err()
        {
            crate::logger::warn("USB", 972, "set_tr_dequeue_pointer failed");
            return;
        }
    }

    let buf = controller.dma_base + report_off as u64;
    controller.slot_id = dev.slot_id;
    ring_enqueue(controller, mouse, buf, dev.max_packet_size as u32);
    controller.ring_xfer_doorbell(dci);
}

/// Drain the shared event ring once and dispatch each transfer event to the
/// keyboard or mouse by slot id, re-arming the matching ring. Returns true if
/// any report was consumed.
///
/// # Safety
/// Run only after `install_runtime`; the stored controller's MMIO/DMA mappings
/// must remain valid and no other code may touch the controller.
pub unsafe fn poll_input() -> bool {
    let mut ctrl_guard = USB_CONTROLLER.lock();
    let kb_guard = USB_KEYBOARD.lock();
    let mouse_guard = USB_MOUSE.lock();
    let controller = match ctrl_guard.as_mut() {
        Some(c) => c,
        None => return false,
    };
    let kb = kb_guard.as_ref();
    let mouse = mouse_guard.as_ref();

    if let Some(k) = kb {
        if ep_halted(controller, k) {
            arm_device(controller, k, false);
        }
    }
    if let Some(m) = mouse {
        if ep_halted(controller, m) {
            arm_device(controller, m, true);
        }
    }

    let mut worked = false;
    while let Some((sid, _dci, _residue)) = controller.poll_xfer_event() {
        if let Some(k) = kb {
            if sid == k.slot_id {
                let buf = controller.dma_base + dma::OFF_REPORT as u64;
                let iface = iface_of(k);
                let _ = parse_keyboard_report(controller, &iface, buf as *const KeyboardReport);
                controller.slot_id = k.slot_id;
                ring_enqueue(controller, false, buf, k.max_packet_size as u32);
                controller.ring_xfer_doorbell(dci_of(k));
                worked = true;
                continue;
            }
        }
        if let Some(m) = mouse {
            if sid == m.slot_id {
                let buf = controller.dma_base + dma::OFF_REPORT_MOUSE as u64;
                let len = (m.max_packet_size as usize).min(64);
                let raw = core::slice::from_raw_parts(buf as *const u8, len);
                dispatch_mouse(&m.mouse_layout, raw);
                controller.slot_id = m.slot_id;
                ring_enqueue(controller, true, buf, m.max_packet_size as u32);
                controller.ring_xfer_doorbell(dci_of(m));
                worked = true;
                continue;
            }
        }
    }
    worked
}

pub fn keyboard_present() -> bool {
    USB_KEYBOARD.lock().is_some()
}

pub fn mouse_present() -> bool {
    USB_MOUSE.lock().is_some()
}
