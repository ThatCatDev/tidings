//! Raw evdev events and the protocol constants this crate uses.

use std::time::Duration;

/// `EV_SYN` event type.
pub const EV_SYN: u16 = 0x00;
/// `EV_ABS` event type.
pub const EV_ABS: u16 = 0x03;
/// `SYN_REPORT`: end of a frame.
pub const SYN_REPORT: u16 = 0;
/// `SYN_DROPPED`: the kernel buffer overflowed; state until the next report is lost.
pub const SYN_DROPPED: u16 = 3;
/// `ABS_MT_SLOT`: selects the slot subsequent MT events apply to.
pub const ABS_MT_SLOT: u16 = 0x2f;
/// `ABS_MT_POSITION_X`.
pub const ABS_MT_POSITION_X: u16 = 0x35;
/// `ABS_MT_POSITION_Y`.
pub const ABS_MT_POSITION_Y: u16 = 0x36;
/// `ABS_MT_TRACKING_ID`: -1 lifts the slot's contact.
pub const ABS_MT_TRACKING_ID: u16 = 0x39;

/// One `struct input_event`, with its timestamp as time since the clock origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputEvent {
    /// Kernel timestamp (monotonic once `EVIOCSCLOCKID` succeeded).
    pub time: Duration,
    /// Event type (`EV_*`).
    pub kind: u16,
    /// Event code (`SYN_*`, `ABS_*`, ...).
    pub code: u16,
    /// Event value.
    pub value: i32,
}

impl InputEvent {
    /// Builds an event.
    #[must_use]
    pub const fn new(time: Duration, kind: u16, code: u16, value: i32) -> Self {
        Self {
            time,
            kind,
            code,
            value,
        }
    }
}

/// The inclusive range of an absolute axis, from `EVIOCGABS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisRange {
    /// Smallest reported value.
    pub min: i32,
    /// Largest reported value.
    pub max: i32,
}
