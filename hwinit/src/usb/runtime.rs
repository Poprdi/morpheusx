//! Runtime USB HID polling — keeps the xHC alive past Phase 9 and lets the
//! bootloader's input loop fetch keyboard reports while the kernel is up.
//!
//! Phase 9 used to drop the `XhciController` at the end of platform init,
//! leaving no way to actually use the enumerated keyboard. This module:
//!   * stores the controller + the discovered keyboard's slot/endpoint info
//!     in static locks so they survive past Phase 9
//!   * arms one interrupt-IN transfer on the keyboard's endpoint
//!   * exposes a non-blocking `poll_keyboard()` that the input loop calls
//!     each iteration — peeks the event ring, parses any report that arrived,
//!     pushes keystrokes into the unified input queue, and re-arms
//!
//! Polling model: *one outstanding transfer at all times*. The xHC waits for
//! the device to actually send a report (the controller does NAK retries
//! internally based on the endpoint's bInterval), at which point it writes
//! the report into `OFF_REPORT` and posts a TRANSFER_EVENT. Our poll just
//! peeks for that event with `wait_xfer(slot, dci, 0)` (timeout=0 → returns
//! immediately if nothing's there).

use crate::sync::SpinLock;
use crate::usb::controller::XhciController;
use crate::usb::dma;
use crate::usb::enumerate::UsbInputDevice;
use crate::usb::hid::keyboard::{parse_keyboard_report, KeyboardReport};
use crate::usb::hid::HIDInterface;
use crate::usb::regs::{TRB_IOC, TRB_ISP, TRB_NORMAL};

static USB_CONTROLLER: SpinLock<Option<XhciController>> = SpinLock::new(None);
static USB_KEYBOARD: SpinLock<Option<UsbInputDevice>> = SpinLock::new(None);

/// Move the xHC and the discovered keyboard into static storage so they live
/// past Phase 9. Arms the first interrupt-IN transfer so the controller is
/// ready to receive a report the next time the keyboard sends one.
///
/// Call once at the end of platform Phase 9 with the controller that just
/// finished enumeration. Passing `keyboard = None` is fine — no polling will
/// happen but the controller is still preserved for future use.
pub unsafe fn install_runtime(controller: XhciController, keyboard: Option<UsbInputDevice>) {
    {
        let mut c = USB_CONTROLLER.lock();
        *c = Some(controller);
    }
    {
        let mut k = USB_KEYBOARD.lock();
        *k = keyboard;
    }
    // Mirror to framebuffer; confirms runtime install path on serial-less boards.
    if let Some(kb) = keyboard {
        crate::serial::puts("[USB-DBG] runtime install: slot=");
        crate::serial::puts_dec_u8(kb.slot_id);
        crate::serial::puts(" ep_in=");
        crate::serial::puts_hex_u8(kb.ep_in);
        crate::serial::puts(" mpkt=");
        crate::serial::puts_hex_u8(kb.max_packet_size as u8);
        crate::serial::puts("\n");
    } else {
        crate::serial::puts("[USB-DBG] runtime install: no keyboard\n");
    }
    arm_keyboard_transfer();
    crate::serial::puts("[USB-DBG] runtime: int-IN transfer armed\n");
}

/// Enqueue one interrupt-IN TRB on the keyboard's transfer ring and ring its
/// doorbell. The xHC will populate `OFF_REPORT` from the device the next time
/// the device sends a report packet, then post a TRANSFER_EVENT.
unsafe fn arm_keyboard_transfer() {
    let mut ctrl_guard = USB_CONTROLLER.lock();
    let kb_guard = USB_KEYBOARD.lock();
    let (controller, kb) = match (ctrl_guard.as_mut(), kb_guard.as_ref()) {
        (Some(c), Some(k)) => (c, k),
        _ => return,
    };

    let buf = controller.dma_base + dma::OFF_REPORT as u64;
    let mpkt = kb.max_packet_size as u32;

    controller.slot_id = kb.slot_id;
    controller
        .bin
        .enqueue(buf, mpkt, TRB_NORMAL | TRB_IOC | TRB_ISP);

    // DCI = endpoint_number * 2 + (IN ? 1 : 0). For our keyboard interrupt-IN
    // endpoint with bEndpointAddress = 0x81, ep number = 1, IN = 1, DCI = 3.
    let dci = ((kb.ep_in & 0x7F) as u32) * 2 + 1;
    controller.ring_xfer_doorbell(dci);
}

/// Non-blocking poll: if the keyboard has produced a report since the last
/// call, parse it (pushing keystrokes into the unified input queue via
/// `input::push_keyboard_event_internal`) and re-arm.
///
/// Returns true if a report was consumed this call, false otherwise. The
/// caller can use the return value to keep `had_work = true` so the idle HLT
/// doesn't fire while keystrokes are arriving.
pub unsafe fn poll_keyboard() -> bool {
    let mut ctrl_guard = USB_CONTROLLER.lock();
    let kb_guard = USB_KEYBOARD.lock();
    let (controller, kb) = match (ctrl_guard.as_mut(), kb_guard.as_ref()) {
        (Some(c), Some(k)) => (c, k),
        _ => return false,
    };

    let dci = ((kb.ep_in & 0x7F) as u32) * 2 + 1;

    // wait_xfer with timeout_ms=0 is the non-blocking variant — peeks the
    // event ring, drains any unrelated TRBs it finds (per the
    // [[usb-event-ring-drain-invariant]]), and returns CommandTimeout if
    // nothing matches.
    match controller.wait_xfer(kb.slot_id, dci, 0) {
        Ok(_residue) => {
            // Got a report. Snapshot the bytes from the DMA buffer before
            // re-arming so we don't race the next packet writing over it.
            let buf = controller.dma_base + dma::OFF_REPORT as u64;
            let report_ptr = buf as *const KeyboardReport;

            // parse_keyboard_report currently ignores ctl/iface (predates this
            // polling layer); passed anyway for forward compat.
            let iface = HIDInterface {
                interface_num: kb.interface_num,
                protocol: kb.protocol,
                ep_in: kb.ep_in,
                ep_out: kb.ep_out,
                max_packet_in: kb.max_packet_size,
            };
            let _ = parse_keyboard_report(controller, &iface, report_ptr);

            // Re-arm immediately so we never miss a report.
            controller.slot_id = kb.slot_id;
            controller.bin.enqueue(
                buf,
                kb.max_packet_size as u32,
                TRB_NORMAL | TRB_IOC | TRB_ISP,
            );
            controller.ring_xfer_doorbell(dci);

            true
        }
        Err(_) => false,
    }
}

/// Has a USB keyboard been installed for runtime polling?
pub fn keyboard_present() -> bool {
    USB_KEYBOARD.lock().is_some()
}
