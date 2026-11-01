//! Multitouch protocol B decoder: raw events in, per-frame contact
//! snapshots out.
//!
//! Contacts are identified by tracking id, so a slot reused for a new finger
//! shows up as a new contact. Positions persist per slot across tracking ids
//! because the kernel suppresses unchanged values.

use std::time::Duration;

use crate::event::{
    ABS_MT_POSITION_X, ABS_MT_POSITION_Y, ABS_MT_SLOT, ABS_MT_TRACKING_ID, EV_ABS, EV_SYN,
    InputEvent, SYN_DROPPED, SYN_REPORT,
};

/// Slots beyond this are ignored (Apple trackpads report up to 16).
const MAX_SLOTS: usize = 64;

/// A finger on the pad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Contact {
    /// Kernel tracking id, unique for the life of one touch.
    pub tracking_id: i32,
    /// Position in device units.
    pub x: i32,
    /// Position in device units.
    pub y: i32,
}

/// All contacts at one `SYN_REPORT`, ordered by slot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Frame {
    /// Timestamp of the `SYN_REPORT`.
    pub time: Duration,
    /// Fingers down, in slot order.
    pub contacts: Vec<Contact>,
}

/// Output of [`MtDecoder::feed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded {
    /// A complete frame.
    Frame(Frame),
    /// Events were dropped (`SYN_DROPPED`); touch state is unknown until the
    /// fingers lift, so any gesture must be aborted.
    Dropped,
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    tracking_id: i32,
    x: Option<i32>,
    y: Option<i32>,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            tracking_id: -1,
            x: None,
            y: None,
        }
    }
}

/// Protocol B slot tracker.
#[derive(Debug, Clone, Default)]
pub struct MtDecoder {
    slot: usize,
    slots: Vec<Slot>,
    dropping: bool,
}

impl MtDecoder {
    /// A decoder with no known contacts.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one event; returns a frame on `SYN_REPORT`.
    pub fn feed(&mut self, ev: InputEvent) -> Option<Decoded> {
        match (ev.kind, ev.code) {
            (EV_SYN, SYN_DROPPED) => {
                self.dropping = true;
                self.slots.clear();
                None
            }
            (EV_SYN, SYN_REPORT) if self.dropping => {
                self.dropping = false;
                Some(Decoded::Dropped)
            }
            (EV_SYN, SYN_REPORT) => Some(Decoded::Frame(self.snapshot(ev.time))),
            _ if self.dropping || ev.kind != EV_ABS => None,
            (_, ABS_MT_SLOT) => {
                self.slot = usize::try_from(ev.value).unwrap_or(MAX_SLOTS);
                None
            }
            (_, code @ (ABS_MT_TRACKING_ID | ABS_MT_POSITION_X | ABS_MT_POSITION_Y)) => {
                if let Some(slot) = self.current_slot() {
                    match code {
                        ABS_MT_TRACKING_ID => slot.tracking_id = ev.value,
                        ABS_MT_POSITION_X => slot.x = Some(ev.value),
                        _ => slot.y = Some(ev.value),
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn current_slot(&mut self) -> Option<&mut Slot> {
        if self.slot >= MAX_SLOTS {
            return None;
        }
        if self.slots.len() <= self.slot {
            self.slots.resize(self.slot + 1, Slot::default());
        }
        self.slots.get_mut(self.slot)
    }

    fn snapshot(&self, time: Duration) -> Frame {
        let contacts = self
            .slots
            .iter()
            .filter(|s| s.tracking_id != -1)
            .filter_map(|s| {
                Some(Contact {
                    tracking_id: s.tracking_id,
                    x: s.x?,
                    y: s.y?,
                })
            })
            .collect();
        Frame { time, contacts }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Feeder(MtDecoder);

    impl Feeder {
        fn abs(&mut self, code: u16, value: i32) {
            assert_eq!(
                self.0
                    .feed(InputEvent::new(Duration::ZERO, EV_ABS, code, value)),
                None
            );
        }

        fn syn(&mut self) -> Vec<(i32, i32, i32)> {
            match self
                .0
                .feed(InputEvent::new(Duration::ZERO, EV_SYN, SYN_REPORT, 0))
            {
                Some(Decoded::Frame(f)) => f
                    .contacts
                    .iter()
                    .map(|c| (c.tracking_id, c.x, c.y))
                    .collect(),
                other => panic!("expected a frame, got {other:?}"),
            }
        }
    }

    #[test]
    fn frames_tracking_ids_and_slot_reuse() {
        let mut d = Feeder(MtDecoder::new());
        d.abs(ABS_MT_SLOT, 0);
        d.abs(ABS_MT_TRACKING_ID, 10);
        d.abs(ABS_MT_POSITION_X, 100);
        d.abs(ABS_MT_POSITION_Y, 200);
        assert_eq!(d.syn(), [(10, 100, 200)]);
        // Slot 1 joins; slot 0 unchanged and not re-sent.
        d.abs(ABS_MT_SLOT, 1);
        d.abs(ABS_MT_TRACKING_ID, 11);
        d.abs(ABS_MT_POSITION_X, 300);
        d.abs(ABS_MT_POSITION_Y, 400);
        assert_eq!(d.syn(), [(10, 100, 200), (11, 300, 400)]);
        // Slot 1 lifts.
        d.abs(ABS_MT_TRACKING_ID, -1);
        assert_eq!(d.syn(), [(10, 100, 200)]);
        // Slot 0 reused for a new finger within one frame: new id, X persists.
        d.abs(ABS_MT_SLOT, 0);
        d.abs(ABS_MT_TRACKING_ID, -1);
        d.abs(ABS_MT_TRACKING_ID, 12);
        d.abs(ABS_MT_POSITION_Y, 900);
        assert_eq!(d.syn(), [(12, 100, 900)]);
        d.abs(ABS_MT_TRACKING_ID, -1);
        assert_eq!(d.syn(), []);
    }

    #[test]
    fn untracked_slots_and_missing_positions_are_ignored() {
        let mut d = Feeder(MtDecoder::new());
        // A contact present before we started reading: no tracking id seen.
        d.abs(ABS_MT_POSITION_X, 5);
        d.abs(ABS_MT_POSITION_Y, 5);
        assert_eq!(d.syn(), []);
        // Tracked but no Y yet.
        d.abs(ABS_MT_SLOT, 2);
        d.abs(ABS_MT_TRACKING_ID, 7);
        d.abs(ABS_MT_POSITION_X, 1);
        assert_eq!(d.syn(), []);
    }

    #[test]
    fn out_of_range_slots_are_ignored() {
        let mut d = Feeder(MtDecoder::new());
        d.abs(ABS_MT_SLOT, -3);
        d.abs(ABS_MT_TRACKING_ID, 1);
        d.abs(ABS_MT_SLOT, 1000);
        d.abs(ABS_MT_TRACKING_ID, 2);
        assert_eq!(d.syn(), []);
    }

    #[test]
    fn syn_dropped_yields_dropped_then_resyncs() {
        let mut d = Feeder(MtDecoder::new());
        d.abs(ABS_MT_TRACKING_ID, 1);
        d.abs(ABS_MT_POSITION_X, 1);
        d.abs(ABS_MT_POSITION_Y, 1);
        assert_eq!(d.syn().len(), 1);
        let ev = |kind, code, value| InputEvent::new(Duration::ZERO, kind, code, value);
        assert_eq!(d.0.feed(ev(EV_SYN, SYN_DROPPED, 0)), None);
        assert_eq!(d.0.feed(ev(EV_ABS, ABS_MT_POSITION_X, 9)), None);
        assert_eq!(d.0.feed(ev(EV_SYN, SYN_REPORT, 0)), Some(Decoded::Dropped));
        assert_eq!(d.syn(), [], "state was discarded");
    }

    #[test]
    fn frame_carries_report_time_and_ignores_other_types() {
        let mut d = MtDecoder::new();
        assert_eq!(
            d.feed(InputEvent::new(Duration::ZERO, 0x01, 0x14a, 1)),
            None
        );
        let t = Duration::from_millis(1234);
        assert_eq!(
            d.feed(InputEvent::new(t, EV_SYN, SYN_REPORT, 0)),
            Some(Decoded::Frame(Frame {
                time: t,
                contacts: vec![]
            }))
        );
    }
}
