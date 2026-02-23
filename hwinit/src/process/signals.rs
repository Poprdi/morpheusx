//! POSIX-lite signals for process management.
//!
//! Signals are represented as a bitmask (`SignalSet`) holding up to 64 signals.
//! The `kill(pid, signal)` syscall sets a bit in the target process's
//! `pending_signals`; the scheduler delivers pending signals before resuming
//! execution.
//!
//! ## Implemented signals
//!
//! | Number | Name      | Description                          | Catchable |
//! |--------|-----------|--------------------------------------|-----------|
//! |  2     | SIGINT    | Keyboard interrupt (Ctrl-C)          | Yes       |
//! |  9     | SIGKILL   | Unconditional termination            | **No**    |
//! | 11     | SIGSEGV   | Segmentation fault (page fault)      | Yes       |
//! | 15     | SIGTERM   | Graceful termination request         | Yes       |
//! | 17     | SIGCHLD   | Child process exited                  | Yes       |
//! | 19     | SIGSTOP   | Pause process                        | **No**    |
//! | 18     | SIGCONT   | Resume stopped process               | Yes       |

// ═══════════════════════════════════════════════════════════════════════════
// SIGNAL NUMBERS (match Linux/POSIX for familiarity)
// ═══════════════════════════════════════════════════════════════════════════

/// Named signal constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Signal {
    /// Keyboard interrupt (Ctrl-C).
    SIGINT = 2,
    /// Kill (cannot be caught or ignored).
    SIGKILL = 9,
    /// Segmentation fault.
    SIGSEGV = 11,
    /// Software termination request.
    SIGTERM = 15,
    /// Child process status change.
    SIGCHLD = 17,
    /// Continue stopped process (can be caught).
    SIGCONT = 18,
    /// Stop (cannot be caught or ignored).
    SIGSTOP = 19,
}

impl Signal {
    /// Bit index of this signal in a `SignalSet`.
    #[inline]
    pub const fn bit(self) -> u64 {
        1u64 << (self as u8 as u32)
    }

    /// True if the signal cannot be caught or ignored.
    pub const fn is_uncatchable(self) -> bool {
        matches!(self, Signal::SIGKILL | Signal::SIGSTOP)
    }

    /// Default action when the process has no handler registered.
    pub const fn default_action(self) -> SignalAction {
        match self {
            Signal::SIGINT => SignalAction::Terminate,
            Signal::SIGKILL => SignalAction::Terminate,
            Signal::SIGSEGV => SignalAction::Terminate,
            Signal::SIGTERM => SignalAction::Terminate,
            Signal::SIGCHLD => SignalAction::Ignore,
            Signal::SIGCONT => SignalAction::Continue,
            Signal::SIGSTOP => SignalAction::Stop,
        }
    }

    /// Construct from a raw number.  Returns None if unknown.
    pub const fn from_u8(n: u8) -> Option<Self> {
        match n {
            2 => Some(Signal::SIGINT),
            9 => Some(Signal::SIGKILL),
            11 => Some(Signal::SIGSEGV),
            15 => Some(Signal::SIGTERM),
            17 => Some(Signal::SIGCHLD),
            18 => Some(Signal::SIGCONT),
            19 => Some(Signal::SIGSTOP),
            _ => None,
        }
    }
}

/// What to do when a signal fires without a registered handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalAction {
    Terminate,
    Ignore,
    Stop,
    Continue,
}

// ═══════════════════════════════════════════════════════════════════════════
// SIGNAL SET (bitmask)
// ═══════════════════════════════════════════════════════════════════════════

/// A bitmask of pending or blocked signals (up to 64).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SignalSet(u64);

impl SignalSet {
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Set a signal bit.
    #[inline]
    pub fn raise(&mut self, sig: Signal) {
        self.0 |= sig.bit();
    }

    /// Clear a signal bit.
    #[inline]
    pub fn clear(&mut self, sig: Signal) {
        self.0 &= !sig.bit();
    }

    /// True if this signal is pending.
    #[inline]
    pub const fn is_pending(self, sig: Signal) -> bool {
        (self.0 & sig.bit()) != 0
    }

    /// True if no signals are pending.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Return the highest-priority pending signal (lowest number wins)
    /// and clear its bit.  Returns `None` if none pending.
    pub fn take_next(&mut self) -> Option<Signal> {
        if self.0 == 0 {
            return None;
        }
        // Find lowest set bit
        let bit_idx = self.0.trailing_zeros() as u8;
        if let Some(sig) = Signal::from_u8(bit_idx) {
            self.0 &= !(1u64 << bit_idx);
            Some(sig)
        } else {
            // Unknown signal number — clear the bit and discard
            self.0 &= !(1u64 << bit_idx);
            None
        }
    }

    /// Raw bitmask value.
    pub const fn bits(self) -> u64 {
        self.0
    }
}
