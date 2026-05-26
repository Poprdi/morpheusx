//! POSIX-lite signals. Numbers match Linux. `kill()` sets a bit in `pending_signals`;
//! scheduler delivers before resuming. SIGKILL (9) and SIGSTOP (19) are uncatchable.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Signal {
    SIGINT = 2,
    SIGKILL = 9,
    SIGSEGV = 11,
    SIGTERM = 15,
    SIGCHLD = 17,
    SIGCONT = 18,
    SIGSTOP = 19,
}

impl Signal {
    #[inline]
    pub const fn bit(self) -> u64 {
        1u64 << (self as u8 as u32)
    }

    pub const fn is_uncatchable(self) -> bool {
        matches!(self, Signal::SIGKILL | Signal::SIGSTOP)
    }

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

/// Action when no handler is registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalAction {
    Terminate,
    Ignore,
    Stop,
    Continue,
}

/// Bitmask of pending/blocked signals (up to 64).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SignalSet(u64);

impl SignalSet {
    pub const fn empty() -> Self {
        Self(0)
    }

    #[inline]
    pub fn raise(&mut self, sig: Signal) {
        self.0 |= sig.bit();
    }

    #[inline]
    pub fn clear(&mut self, sig: Signal) {
        self.0 &= !sig.bit();
    }

    #[inline]
    pub const fn is_pending(self, sig: Signal) -> bool {
        (self.0 & sig.bit()) != 0
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Take lowest-numbered pending signal (highest priority); clears the bit.
    pub fn take_next(&mut self) -> Option<Signal> {
        if self.0 == 0 {
            return None;
        }
        let bit_idx = self.0.trailing_zeros() as u8;
        if let Some(sig) = Signal::from_u8(bit_idx) {
            self.0 &= !(1u64 << bit_idx);
            Some(sig)
        } else {
            // Unknown signal number — discard.
            self.0 &= !(1u64 << bit_idx);
            None
        }
    }

    pub const fn bits(self) -> u64 {
        self.0
    }
}
