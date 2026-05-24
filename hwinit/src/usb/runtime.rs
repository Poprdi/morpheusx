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
    arm_keyboard_transfer();
}

/// Enqueue one interrupt-IN TRB on the keyboard's transfer ring and ring its
/// doorbell. The xHC will populate `OFF_REPORT` from the device the next time
/// the device sends a report packet, then post a TRANSFER_EVENT.
///
/// Before arming, checks the EP context's State field. On real hardware the
/// LS keyboard often arrives in Halted state by the time we get here — the
/// controller polled it 3 times right after CONFIGURE_ENDPOINT, hit USB
/// errors (timing-sensitive LS-on-HS via the PCH's internal TT, or a STALL
/// while SET_PROTOCOL was still applying), CErr counted down to 0, endpoint
/// halted. A halted endpoint silently ignores doorbell rings forever; we
/// MUST clear it with RESET_ENDPOINT + SET_TR_DEQUEUE_POINTER before our
/// transfer goes anywhere.
unsafe fn arm_keyboard_transfer() {
    let mut ctrl_guard = USB_CONTROLLER.lock();
    let kb_guard = USB_KEYBOARD.lock();
    let (controller, kb) = match (ctrl_guard.as_mut(), kb_guard.as_ref()) {
        (Some(c), Some(k)) => (c, k),
        _ => return,
    };

    let dci = ((kb.ep_in & 0x7F) as u32) * 2 + 1;

    // Check current EP state in the slot's output context.
    let cs = controller.ctx_size as u64;
    let out_ctx = controller.dma_base + dma::slot_out_ctx_offset(kb.slot_id) as u64;
    let ep_ctx = out_ctx + (dci as u64) * cs;
    let ep_state = crate::usb::rings::vr32(ep_ctx) & 0x7;

    if ep_state == 2 || ep_state == 4 {
        // 2 = Halted, 4 = Error — both require reset before further use.
        crate::serial::puts("[USB-DBG] EP halted/error; issuing RESET_ENDPOINT\n");
        if let Err(_e) = controller.reset_endpoint(kb.slot_id, dci) {
            crate::serial::puts("[USB-DBG] reset_endpoint failed\n");
            return;
        }
        // After reset the EP is in Stopped state; the TR Dequeue Pointer is
        // wherever the xHC halted, which is probably the offending TRB.
        // Point dequeue back at offset 0 of our ring with cycle=1, and
        // reset our Rust-side producer to match.
        controller.bin.reset();
        let ring_base = controller.dma_base + dma::OFF_XFER_BIN as u64;
        let deq = (ring_base & !0xF) | 1; // DCS=1
        if let Err(_e) = controller.set_tr_dequeue_pointer(kb.slot_id, dci, deq) {
            crate::serial::puts("[USB-DBG] set_tr_dequeue_pointer failed\n");
            return;
        }
        crate::serial::puts("[USB-DBG] EP recovered, re-arming\n");
    }

    let buf = controller.dma_base + dma::OFF_REPORT as u64;
    let mpkt = kb.max_packet_size as u32;

    controller.slot_id = kb.slot_id;
    controller
        .bin
        .enqueue(buf, mpkt, TRB_NORMAL | TRB_IOC | TRB_ISP);
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

    // Halt recovery, runs every poll. An endpoint that halted between
    // arming and now (USB errors, CErr decremented to 0) is silently
    // unresponsive to doorbell rings until RESET_ENDPOINT clears it.
    // Detect by reading EP State (bits [2:0] of the EP context DW0) out
    // of the slot's output context. 2 = Halted, 4 = Error.
    {
        let cs = controller.ctx_size as u64;
        let out_ctx =
            controller.dma_base + crate::usb::dma::slot_out_ctx_offset(kb.slot_id) as u64;
        let ep_ctx = out_ctx + (dci as u64) * cs;
        let ep_state = crate::usb::rings::vr32(ep_ctx) & 0x7;
        if ep_state == 2 || ep_state == 4 {
            if controller.reset_endpoint(kb.slot_id, dci).is_ok() {
                controller.bin.reset();
                let ring_base = controller.dma_base + dma::OFF_XFER_BIN as u64;
                let deq = (ring_base & !0xF) | 1;
                let _ = controller.set_tr_dequeue_pointer(kb.slot_id, dci, deq);
                // Re-arm — controller is now Stopped; ring doorbell to restart.
                let buf = controller.dma_base + dma::OFF_REPORT as u64;
                let mpkt = kb.max_packet_size as u32;
                controller.slot_id = kb.slot_id;
                controller
                    .bin
                    .enqueue(buf, mpkt, TRB_NORMAL | TRB_IOC | TRB_ISP);
                controller.ring_xfer_doorbell(dci);
            }
            // Don't try to peek the event ring this round — let the
            // controller's next poll cycle produce something.
            return false;
        }
    }

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
