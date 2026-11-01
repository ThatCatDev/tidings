//! Decoder + recogniser: raw evdev events in, gesture events out.

use tidings_core::gesture::GestureEvent;

use crate::decoder::{Decoded, MtDecoder};
use crate::event::{AxisRange, InputEvent};
use crate::recognizer::{Recognizer, RecognizerConfig, RecognizerError};

/// Decodes one device's event stream and recognises gestures on it.
#[derive(Debug)]
pub struct GestureEngine {
    decoder: MtDecoder,
    recognizer: Recognizer,
}

impl GestureEngine {
    /// An engine for a pad whose X axis spans `x`.
    pub fn new(x: AxisRange, config: RecognizerConfig) -> Result<Self, RecognizerError> {
        Ok(Self {
            decoder: MtDecoder::new(),
            recognizer: Recognizer::new(x, config)?,
        })
    }

    /// See [`Recognizer::set_panel_open`].
    pub fn set_panel_open(&mut self, open: bool) {
        self.recognizer.set_panel_open(open);
    }

    /// See [`Recognizer::abort`].
    pub fn abort(&mut self) -> Option<GestureEvent> {
        self.recognizer.abort()
    }

    /// Feeds one raw event.
    pub fn feed(&mut self, ev: InputEvent) -> Vec<GestureEvent> {
        match self.decoder.feed(ev) {
            None => Vec::new(),
            Some(Decoded::Dropped) => self.recognizer.abort().into_iter().collect(),
            Some(Decoded::Frame(frame)) => self.recognizer.frame(&frame),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::event::{
        ABS_MT_POSITION_X, ABS_MT_POSITION_Y, ABS_MT_SLOT, ABS_MT_TRACKING_ID, EV_ABS, EV_SYN,
        SYN_DROPPED, SYN_REPORT,
    };
    use crate::recognizer::tests::{EDGE_X, MID_X, W, X_MAX, X_MIN, x_at};
    use std::time::Duration;
    use tidings_core::gesture::GestureKind;

    /// Builds raw protocol-B event streams.
    #[derive(Default)]
    pub(crate) struct EventScript {
        pub(crate) events: Vec<InputEvent>,
        t: Duration,
        next_id: i32,
    }

    impl EventScript {
        fn ev(&mut self, kind: u16, code: u16, value: i32) -> &mut Self {
            self.events.push(InputEvent::new(self.t, kind, code, value));
            self
        }

        #[allow(clippy::cast_possible_truncation)]
        pub(crate) fn down(&mut self, slot: i32, x: f64, y: f64) -> &mut Self {
            self.next_id += 1;
            let id = self.next_id;
            self.ev(EV_ABS, ABS_MT_SLOT, slot)
                .ev(EV_ABS, ABS_MT_TRACKING_ID, id)
                .ev(EV_ABS, ABS_MT_POSITION_X, x.round() as i32)
                .ev(EV_ABS, ABS_MT_POSITION_Y, y.round() as i32)
        }

        #[allow(clippy::cast_possible_truncation)]
        pub(crate) fn move_x(&mut self, slot: i32, x: f64) -> &mut Self {
            self.ev(EV_ABS, ABS_MT_SLOT, slot)
                .ev(EV_ABS, ABS_MT_POSITION_X, x.round() as i32)
        }

        pub(crate) fn up(&mut self, slot: i32) -> &mut Self {
            self.ev(EV_ABS, ABS_MT_SLOT, slot)
                .ev(EV_ABS, ABS_MT_TRACKING_ID, -1)
        }

        pub(crate) fn dropped(&mut self) -> &mut Self {
            self.ev(EV_SYN, SYN_DROPPED, 0)
        }

        /// Ends the frame and advances time by 8 ms.
        pub(crate) fn syn(&mut self) -> &mut Self {
            self.ev(EV_SYN, SYN_REPORT, 0);
            self.t += Duration::from_millis(8);
            self
        }

        pub(crate) fn wait(&mut self, d: Duration) -> &mut Self {
            self.t += d;
            self
        }

        /// Two fingers in slots 0 and 1 swiping by `dx` over `steps` frames.
        pub(crate) fn swipe(
            &mut self,
            x0: f64,
            dx: f64,
            steps: u32,
            gap: Duration,
            lift: bool,
        ) -> &mut Self {
            self.down(0, x0, -1000.0).syn();
            self.wait(gap);
            self.down(1, x0, 1000.0).syn();
            for i in 1..=steps {
                let x = x0 + dx * f64::from(i) / f64::from(steps);
                self.move_x(0, x).move_x(1, x).syn();
            }
            if lift {
                self.up(0).up(1).syn();
            }
            self
        }
    }

    pub(crate) fn engine() -> GestureEngine {
        GestureEngine::new(
            AxisRange {
                min: X_MIN,
                max: X_MAX,
            },
            RecognizerConfig::default(),
        )
        .unwrap()
    }

    fn run(events: &[InputEvent]) -> Vec<GestureEvent> {
        let mut e = engine();
        events.iter().flat_map(|&ev| e.feed(ev)).collect()
    }

    #[test]
    fn event_level_open() {
        let mut s = EventScript::default();
        s.swipe(EDGE_X, -0.3 * W, 20, Duration::ZERO, true);
        let out = run(&s.events);
        assert_eq!(out[0], GestureEvent::Begin(GestureKind::Open));
        let GestureEvent::End { progress, .. } = out.last().copied().unwrap() else {
            panic!("expected End: {out:?}");
        };
        assert!((f64::from(progress) - 0.3).abs() < 1e-3);
    }

    #[test]
    fn event_level_fingers_too_far_apart() {
        let mut s = EventScript::default();
        s.swipe(EDGE_X, -0.3 * W, 20, Duration::from_millis(300), true);
        assert_eq!(run(&s.events), []);
    }

    #[test]
    fn slot_reuse_mid_gesture_ends_instead_of_jumping() {
        let mut s = EventScript::default();
        s.swipe(EDGE_X, -0.2 * W, 10, Duration::ZERO, false);
        // Slot 1's finger lifts and a new one lands in the same slot within one frame.
        s.up(1).down(1, MID_X, 0.0).syn();
        s.move_x(1, MID_X - 0.2 * W).move_x(0, x_at(0.3)).syn();
        s.up(0).up(1).syn();
        let out = run(&s.events);
        assert_eq!(
            out.iter()
                .filter(|e| matches!(e, GestureEvent::Begin(_)))
                .count(),
            1
        );
        let GestureEvent::End { progress, .. } = out.last().copied().unwrap() else {
            panic!("expected End: {out:?}");
        };
        assert!((f64::from(progress) - 0.2).abs() < 1e-3);
    }

    #[test]
    fn syn_dropped_cancels() {
        let mut s = EventScript::default();
        s.swipe(EDGE_X, -0.2 * W, 10, Duration::ZERO, false);
        s.dropped().move_x(0, 0.0).syn();
        s.up(0).up(1).syn();
        let out = run(&s.events);
        assert_eq!(out[0], GestureEvent::Begin(GestureKind::Open));
        assert_eq!(out.last(), Some(&GestureEvent::Cancel));
    }

    #[test]
    fn panel_state_gates_close() {
        let mut s = EventScript::default();
        s.swipe(MID_X, 0.3 * W, 20, Duration::ZERO, true);
        assert_eq!(run(&s.events), []);
        let mut e = engine();
        e.set_panel_open(true);
        let out: Vec<_> = s.events.iter().flat_map(|&ev| e.feed(ev)).collect();
        assert_eq!(out[0], GestureEvent::Begin(GestureKind::Close));
        assert_eq!(e.abort(), None);
    }
}
