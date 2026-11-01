//! Two-finger edge-swipe recogniser: decoded frames in, [`GestureEvent`]s out.
//!
//! State machine per touch sequence (first finger down to last finger up):
//!
//! * **Idle** — waiting for exactly two fingers that landed within the pair
//!   window to move past the start threshold. The decision is made once, the
//!   first time either axis passes it: horizontal-dominant motion in the
//!   right direction begins a gesture, anything else spends the sequence.
//! * **Active** — streaming `Update`s; a third finger cancels, lifting either
//!   finger of the pair ends.
//! * **Spent** — nothing more fires until every finger lifts, so there is at
//!   most one gesture per sequence.

use std::collections::VecDeque;
use std::time::Duration;

use tidings_core::gesture::{GestureEvent, GestureKind};

use crate::decoder::{Contact, Frame};
use crate::event::AxisRange;

/// Recognition constants. Distances are fractions of pad width.
#[derive(Debug, Clone, PartialEq)]
pub struct RecognizerConfig {
    /// Width of the right-edge zone both fingers must land in to open.
    pub edge: f64,
    /// Both fingers must land within this long of each other.
    pub pair_window: Duration,
    /// Movement needed (on either axis) before deciding.
    pub threshold: f64,
    /// Horizontal motion must be at least this many times the vertical.
    pub dominance: f64,
    /// Release velocity is measured over this trailing window.
    pub velocity_window: Duration,
}

impl Default for RecognizerConfig {
    fn default() -> Self {
        Self {
            edge: 0.10,
            pair_window: Duration::from_millis(250),
            threshold: 0.015,
            dominance: 1.5,
            velocity_window: Duration::from_millis(80),
        }
    }
}

/// Invalid recogniser setup.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RecognizerError {
    /// The device reported `max <= min`.
    #[error("empty X axis range {min}..={max}")]
    EmptyRange {
        /// Reported minimum.
        min: i32,
        /// Reported maximum.
        max: i32,
    },
}

/// Updates are de-duplicated at this progress resolution.
const PROGRESS_QUANTUM: f64 = 1e-4;

#[derive(Debug, Clone, Copy)]
struct Start {
    id: i32,
    x: i32,
    y: i32,
    t: Duration,
}

#[derive(Debug)]
enum Phase {
    Idle,
    Active(Active),
    Spent,
}

#[derive(Debug)]
struct Active {
    kind: GestureKind,
    pair: [i32; 2],
    /// (time, progress), trimmed to a few velocity windows.
    history: VecDeque<(Duration, f64)>,
    last_quantum: i64,
}

/// The gesture state machine. Pure: no I/O, time comes from frames.
#[derive(Debug)]
pub struct Recognizer {
    config: RecognizerConfig,
    width: f64,
    edge_x: f64,
    panel_open: bool,
    phase: Phase,
    starts: Vec<Start>,
}

impl Recognizer {
    /// A recogniser for a pad whose X axis spans `x`.
    pub fn new(x: AxisRange, config: RecognizerConfig) -> Result<Self, RecognizerError> {
        if x.max <= x.min {
            return Err(RecognizerError::EmptyRange {
                min: x.min,
                max: x.max,
            });
        }
        let width = f64::from(x.max) - f64::from(x.min);
        Ok(Self {
            edge_x: f64::from(x.max) - config.edge * width,
            width,
            config,
            panel_open: false,
            phase: Phase::Idle,
            starts: Vec::new(),
        })
    }

    /// Tells the recogniser whether the panel is open. Open gestures are only
    /// recognised while closed, Close gestures only while open. Takes effect
    /// for the next gesture.
    pub fn set_panel_open(&mut self, open: bool) {
        self.panel_open = open;
    }

    /// Input glitch (`SYN_DROPPED`, device lost): cancels any active gesture
    /// and ignores the rest of the touch sequence.
    pub fn abort(&mut self) -> Option<GestureEvent> {
        let was_active = matches!(self.phase, Phase::Active(_));
        if !matches!(self.phase, Phase::Idle) || !self.starts.is_empty() {
            self.phase = Phase::Spent;
        }
        was_active.then_some(GestureEvent::Cancel)
    }

    /// Processes one frame.
    pub fn frame(&mut self, frame: &Frame) -> Vec<GestureEvent> {
        let contacts = &frame.contacts;
        self.starts
            .retain(|s| contacts.iter().any(|c| c.tracking_id == s.id));
        for c in contacts {
            if !self.starts.iter().any(|s| s.id == c.tracking_id) {
                self.starts.push(Start {
                    id: c.tracking_id,
                    x: c.x,
                    y: c.y,
                    t: frame.time,
                });
            }
        }

        let out = match &mut self.phase {
            Phase::Idle => self.idle(contacts, frame.time),
            Phase::Active(_) => self.active(contacts, frame.time),
            Phase::Spent => Vec::new(),
        };

        if contacts.is_empty() {
            self.phase = Phase::Idle;
            self.starts.clear();
        }
        out
    }

    fn start(&self, id: i32) -> Option<&Start> {
        self.starts.iter().find(|s| s.id == id)
    }

    /// Mean (dx, dy) of the given contacts since they landed, in device units.
    fn displacement(&self, contacts: &[Contact], ids: [i32; 2]) -> Option<(f64, f64)> {
        let mut dx = 0.0;
        let mut dy = 0.0;
        for id in ids {
            let c = contacts.iter().find(|c| c.tracking_id == id)?;
            let s = self.start(id)?;
            dx += f64::from(c.x) - f64::from(s.x);
            dy += f64::from(c.y) - f64::from(s.y);
        }
        Some((dx / 2.0, dy / 2.0))
    }

    fn progress(&self, kind: GestureKind, dx: f64) -> f64 {
        match kind {
            GestureKind::Open => -dx / self.width,
            GestureKind::Close => dx / self.width,
        }
    }

    fn idle(&mut self, contacts: &[Contact], t: Duration) -> Vec<GestureEvent> {
        match contacts.len() {
            0 | 1 => return Vec::new(),
            2 => {}
            _ => {
                self.phase = Phase::Spent;
                return Vec::new();
            }
        }
        // `starts` is in landing order and both contacts are in it.
        let mut pair: Vec<&Start> = self.starts.iter().collect();
        pair.sort_by_key(|s| s.t);
        let (first, second) = (*pair[0], *pair[1]);
        if second.t.saturating_sub(first.t) > self.config.pair_window {
            self.phase = Phase::Spent;
            return Vec::new();
        }
        let ids = [first.id, second.id];
        let Some((dx, dy)) = self.displacement(contacts, ids) else {
            return Vec::new();
        };
        let threshold = self.config.threshold * self.width;
        if dx.abs() < threshold && dy.abs() < threshold {
            return Vec::new();
        }
        if dx.abs() < threshold || dx.abs() < self.config.dominance * dy.abs() {
            self.phase = Phase::Spent;
            return Vec::new();
        }
        let from_edge = [first, second]
            .iter()
            .all(|s| f64::from(s.x) >= self.edge_x);
        let kind = if dx < 0.0 && from_edge && !self.panel_open {
            GestureKind::Open
        } else if dx > 0.0 && self.panel_open {
            GestureKind::Close
        } else {
            self.phase = Phase::Spent;
            return Vec::new();
        };
        let p = self.progress(kind, dx);
        self.phase = Phase::Active(Active {
            kind,
            pair: ids,
            history: VecDeque::from([(t, p)]),
            last_quantum: quantize(p),
        });
        vec![
            GestureEvent::Begin(kind),
            GestureEvent::Update {
                progress: narrow(p),
            },
        ]
    }

    fn active(&mut self, contacts: &[Contact], t: Duration) -> Vec<GestureEvent> {
        let Phase::Active(active) = &self.phase else {
            return Vec::new();
        };
        if contacts.len() >= 3 {
            self.phase = Phase::Spent;
            return vec![GestureEvent::Cancel];
        }
        let (kind, pair) = (active.kind, active.pair);
        let Some((dx, _)) = self.displacement(contacts, pair) else {
            // A finger of the pair lifted (or its slot now holds a new contact).
            let progress = active.history.back().map_or(0.0, |&(_, p)| p);
            let velocity = velocity(&active.history, self.config.velocity_window);
            self.phase = Phase::Spent;
            return vec![GestureEvent::End {
                progress: narrow(progress),
                velocity,
            }];
        };
        let p = self.progress(kind, dx);
        let window = self.config.velocity_window;
        let Phase::Active(active) = &mut self.phase else {
            return Vec::new();
        };
        active.history.push_back((t, p));
        let cutoff = t.saturating_sub(window * 4);
        while active.history.len() > 2 && active.history.front().is_some_and(|&(ts, _)| ts < cutoff)
        {
            active.history.pop_front();
        }
        let q = quantize(p);
        if q == active.last_quantum {
            return Vec::new();
        }
        active.last_quantum = q;
        vec![GestureEvent::Update {
            progress: narrow(p),
        }]
    }
}

/// dp/dt from the oldest sample inside the trailing window to the newest,
/// in pad widths per second.
fn velocity(history: &VecDeque<(Duration, f64)>, window: Duration) -> f32 {
    let n = history.len();
    if n < 2 {
        return 0.0;
    }
    let (t_last, p_last) = history[n - 1];
    let mut reference = history[n - 2];
    if let Some(&(ts, ps)) = history.iter().find(|&&(ts, _)| ts + window >= t_last)
        && ts < t_last
    {
        reference = (ts, ps);
    }
    let dt = t_last.saturating_sub(reference.0).as_secs_f64();
    if dt <= 0.0 {
        return 0.0;
    }
    // `+ 0.0` turns a -0.0 (no movement) into 0.0.
    narrow((p_last - reference.1) / dt + 0.0)
}

fn quantize(p: f64) -> i64 {
    // Progress is a small fraction of pad width; the saturating cast is fine.
    #[allow(clippy::cast_possible_truncation)]
    let q = (p / PROGRESS_QUANTUM).round() as i64;
    q
}

fn narrow(v: f64) -> f32 {
    // Pad-width fractions and velocities are well within f32 range.
    #[allow(clippy::cast_possible_truncation)]
    let v = v as f32;
    v
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // The real device's X range.
    pub(crate) const X_MIN: i32 = -6046;
    pub(crate) const X_MAX: i32 = 6536;
    pub(crate) const W: f64 = (X_MAX - X_MIN) as f64;
    /// Inside the 10% edge zone.
    pub(crate) const EDGE_X: f64 = X_MAX as f64 - 0.05 * W;
    pub(crate) const MID_X: f64 = X_MIN as f64 + 0.5 * W;

    pub(crate) fn x_at(f: f64) -> f64 {
        f64::from(X_MIN) + f * W
    }

    type Touches = Vec<(i32, f64, f64)>;

    #[allow(clippy::cast_possible_truncation)]
    fn frame(touches: &Touches, t: f64) -> Frame {
        Frame {
            time: Duration::from_secs_f64(t),
            contacts: touches
                .iter()
                .map(|&(id, x, y)| Contact {
                    tracking_id: id,
                    x: x.round() as i32,
                    y: y.round() as i32,
                })
                .collect(),
        }
    }

    fn recognizer() -> Recognizer {
        Recognizer::new(
            AxisRange {
                min: X_MIN,
                max: X_MAX,
            },
            RecognizerConfig::default(),
        )
        .unwrap()
    }

    fn open_recognizer() -> Recognizer {
        let mut r = recognizer();
        r.set_panel_open(true);
        r
    }

    fn run(rec: &mut Recognizer, frames: &[(Touches, f64)]) -> Vec<GestureEvent> {
        frames
            .iter()
            .flat_map(|(t, time)| rec.frame(&frame(t, *time)))
            .collect()
    }

    /// Two fingers (ids 1, 2) moving together, then a full lift.
    fn pair_frames(x0: f64, dx: f64, steps: u32, t0: f64, dy: f64) -> Vec<(Touches, f64)> {
        let dt = 0.008;
        let mut frames = vec![(vec![(1, x0, -1000.0), (2, x0, 1000.0)], t0)];
        for i in 1..=steps {
            let f = f64::from(i) / f64::from(steps);
            frames.push((
                vec![
                    (1, x0 + f * dx, -1000.0 + f * dy),
                    (2, x0 + f * dx, 1000.0 + f * dy),
                ],
                t0 + f64::from(i) * dt,
            ));
        }
        frames.push((vec![], t0 + f64::from(steps + 1) * dt));
        frames
    }

    fn swipe(x0: f64, dx: f64) -> Vec<(Touches, f64)> {
        pair_frames(x0, dx, 20, 0.0, 0.0)
    }

    /// `pair_frames` without the final lift.
    fn held(x0: f64, dx: f64, steps: u32) -> Vec<(Touches, f64)> {
        let mut f = pair_frames(x0, dx, steps, 0.0, 0.0);
        f.pop();
        f
    }

    fn updates(out: &[GestureEvent]) -> Vec<f32> {
        out.iter()
            .filter_map(|e| match e {
                GestureEvent::Update { progress } => Some(*progress),
                _ => None,
            })
            .collect()
    }

    fn end_of(out: &[GestureEvent]) -> (f32, f32) {
        let ends: Vec<_> = out
            .iter()
            .filter_map(|e| match e {
                GestureEvent::End { progress, velocity } => Some((*progress, *velocity)),
                _ => None,
            })
            .collect();
        assert_eq!(ends.len(), 1, "{out:?}");
        ends[0]
    }

    fn count(out: &[GestureEvent], pred: fn(&GestureEvent) -> bool) -> usize {
        out.iter().filter(|e| pred(e)).count()
    }

    fn is_begin(e: &GestureEvent) -> bool {
        matches!(e, GestureEvent::Begin(_))
    }

    fn is_end(e: &GestureEvent) -> bool {
        matches!(e, GestureEvent::End { .. })
    }

    fn is_cancel(e: &GestureEvent) -> bool {
        matches!(e, GestureEvent::Cancel)
    }

    fn approx(a: f32, b: f64, tol: f64) -> bool {
        (f64::from(a) - b).abs() <= tol
    }

    fn last_frame_touches(frames: &[(Touches, f64)]) -> (Touches, f64) {
        frames.last().cloned().unwrap()
    }

    #[test]
    fn empty_range_is_rejected() {
        let err = Recognizer::new(AxisRange { min: 5, max: 5 }, RecognizerConfig::default());
        assert_eq!(
            err.unwrap_err(),
            RecognizerError::EmptyRange { min: 5, max: 5 }
        );
    }

    #[test]
    fn edge_open_full_flow_monotonic_updates_and_end() {
        let out = run(&mut recognizer(), &swipe(EDGE_X, -0.3 * W));
        assert_eq!(out[0], GestureEvent::Begin(GestureKind::Open));
        assert!(is_end(out.last().unwrap()));
        assert!(
            out[1..out.len() - 1]
                .iter()
                .all(|e| matches!(e, GestureEvent::Update { .. }))
        );
        let ps = updates(&out);
        assert!(ps.len() > 5);
        assert!(
            ps.windows(2).all(|w| w[0] < w[1]),
            "strictly increasing: {ps:?}"
        );
        assert!(approx(*ps.last().unwrap(), 0.3, 1e-3));
        let (p, v) = end_of(&out);
        assert!(approx(p, 0.3, 1e-3));
        // 0.3 W over 20 frames of 8 ms = 1.875 W/s.
        assert!(approx(v, 0.3 / (20.0 * 0.008), 0.05), "{v}");
    }

    #[test]
    fn begins_only_after_threshold() {
        let mut rec = recognizer();
        let at = |dx: f64| vec![(1, EDGE_X + dx, 0.0), (2, EDGE_X + dx, 2000.0)];
        assert_eq!(rec.frame(&frame(&at(0.0), 0.0)), []);
        assert_eq!(rec.frame(&frame(&at(-0.01 * W), 0.01)), []);
        let out = rec.frame(&frame(&at(-0.02 * W), 0.02));
        assert_eq!(out[0], GestureEvent::Begin(GestureKind::Open));
        assert!(approx(updates(&out)[0], 0.02, 1e-4));
    }

    #[test]
    fn non_edge_swipe_is_ignored() {
        assert_eq!(run(&mut recognizer(), &swipe(MID_X, -0.3 * W)), []);
    }

    #[test]
    fn one_finger_outside_edge_is_ignored() {
        let frames = vec![
            (vec![(1, EDGE_X, 0.0), (2, x_at(0.85), 2000.0)], 0.0),
            (
                vec![(1, EDGE_X - 0.1 * W, 0.0), (2, x_at(0.75), 2000.0)],
                0.05,
            ),
            (vec![], 0.1),
        ];
        assert_eq!(run(&mut recognizer(), &frames), []);
    }

    #[test]
    fn no_open_while_open() {
        assert_eq!(run(&mut open_recognizer(), &swipe(EDGE_X, -0.3 * W)), []);
    }

    #[test]
    fn edge_width_is_configurable() {
        let x = x_at(0.85);
        assert_eq!(run(&mut recognizer(), &swipe(x, -0.3 * W)), []);
        let config = RecognizerConfig {
            edge: 0.2,
            ..RecognizerConfig::default()
        };
        let mut rec = Recognizer::new(
            AxisRange {
                min: X_MIN,
                max: X_MAX,
            },
            config,
        )
        .unwrap();
        assert_eq!(
            run(&mut rec, &swipe(x, -0.3 * W))[0],
            GestureEvent::Begin(GestureKind::Open)
        );
    }

    #[test]
    fn close_only_when_open() {
        assert_eq!(run(&mut recognizer(), &swipe(MID_X, 0.3 * W)), []);
        let out = run(&mut open_recognizer(), &swipe(MID_X, 0.3 * W));
        assert_eq!(out[0], GestureEvent::Begin(GestureKind::Close));
        let (p, v) = end_of(&out);
        assert!(approx(p, 0.3, 1e-3));
        assert!(v > 1.0);
    }

    #[test]
    fn close_from_anywhere_but_not_leftward() {
        let out = run(&mut open_recognizer(), &swipe(x_at(0.1), 0.2 * W));
        assert_eq!(out[0], GestureEvent::Begin(GestureKind::Close));
        assert_eq!(run(&mut open_recognizer(), &swipe(MID_X, -0.3 * W)), []);
    }

    #[test]
    fn panel_state_applies_to_next_gesture() {
        let mut rec = recognizer();
        assert_eq!(
            run(&mut rec, &swipe(EDGE_X, -0.3 * W))[0],
            GestureEvent::Begin(GestureKind::Open)
        );
        rec.set_panel_open(true);
        let out = run(&mut rec, &pair_frames(MID_X, 0.3 * W, 20, 1.0, 0.0));
        assert_eq!(out[0], GestureEvent::Begin(GestureKind::Close));
    }

    #[test]
    fn vertical_scroll_is_ignored() {
        assert_eq!(
            run(
                &mut recognizer(),
                &pair_frames(EDGE_X, -0.01 * W, 20, 0.0, 0.3 * W)
            ),
            []
        );
        assert_eq!(
            run(
                &mut open_recognizer(),
                &pair_frames(MID_X, 0.01 * W, 20, 0.0, -0.3 * W)
            ),
            []
        );
    }

    #[test]
    fn diagonal_without_dominance_is_ignored() {
        assert_eq!(
            run(
                &mut recognizer(),
                &pair_frames(EDGE_X, -0.2 * W, 20, 0.0, 0.2 * W)
            ),
            []
        );
    }

    #[test]
    fn scroll_that_turns_horizontal_stays_dead() {
        let mut frames = pair_frames(EDGE_X, 0.0, 10, 0.0, 0.1 * W);
        frames.pop();
        let (last, t) = last_frame_touches(&frames);
        for i in 1..20 {
            let d = f64::from(i) * 0.02 * W;
            frames.push((
                last.iter().map(|&(id, x, y)| (id, x - d, y)).collect(),
                t + f64::from(i) * 0.008,
            ));
        }
        frames.push((vec![], t + 1.0));
        assert_eq!(run(&mut recognizer(), &frames), []);
    }

    fn staggered(gap: f64) -> Vec<(Touches, f64)> {
        let mut frames = vec![
            (vec![(1, EDGE_X, -1000.0)], 0.0),
            (vec![(1, EDGE_X, -1000.0), (2, EDGE_X, 1000.0)], gap),
        ];
        for i in 1..20 {
            let d = -f64::from(i) * 0.02 * W;
            frames.push((
                vec![(1, EDGE_X + d, -1000.0), (2, EDGE_X + d, 1000.0)],
                gap + f64::from(i) * 0.008,
            ));
        }
        frames.push((vec![], 1.0));
        frames
    }

    #[test]
    fn fingers_landing_more_than_250ms_apart_are_ignored() {
        assert_eq!(run(&mut recognizer(), &staggered(0.3)), []);
        assert_eq!(run(&mut recognizer(), &staggered(0.26)), []);
    }

    #[test]
    fn fingers_landing_within_window_open() {
        assert_eq!(
            run(&mut recognizer(), &staggered(0.2))[0],
            GestureEvent::Begin(GestureKind::Open)
        );
    }

    #[test]
    fn single_finger_is_ignored() {
        let mut frames: Vec<(Touches, f64)> = (0..30)
            .map(|i| {
                (
                    vec![(1, EDGE_X - f64::from(i) * 0.02 * W, 0.0)],
                    f64::from(i) * 0.008,
                )
            })
            .collect();
        frames.push((vec![], 1.0));
        assert_eq!(run(&mut recognizer(), &frames), []);
    }

    #[test]
    fn three_fingers_before_begin_spend_the_sequence() {
        let mut frames = vec![(
            vec![(1, EDGE_X, 0.0), (2, EDGE_X, 1000.0), (3, EDGE_X, 2000.0)],
            0.0,
        )];
        for i in 1..20 {
            let d = -f64::from(i) * 0.02 * W;
            frames.push((
                vec![(1, EDGE_X + d, 0.0), (2, EDGE_X + d, 1000.0)],
                f64::from(i) * 0.008,
            ));
        }
        frames.push((vec![], 1.0));
        assert_eq!(run(&mut recognizer(), &frames), []);
    }

    #[test]
    fn third_finger_cancels() {
        let mut frames = held(EDGE_X, -0.2 * W, 10);
        let (mut three, t) = last_frame_touches(&frames);
        three.push((3, MID_X, 0.0));
        frames.push((three.clone(), t + 0.008));
        frames.push((
            three
                .iter()
                .map(|&(id, x, y)| (id, x - 0.1 * W, y))
                .collect(),
            t + 0.016,
        ));
        frames.push((vec![three[0]], t + 0.024));
        frames.push((vec![], t + 0.03));
        let out = run(&mut recognizer(), &frames);
        assert_eq!(out[0], GestureEvent::Begin(GestureKind::Open));
        assert_eq!(out.last(), Some(&GestureEvent::Cancel));
        assert_eq!(count(&out, is_cancel), 1);
        assert_eq!(count(&out, is_end), 0);
    }

    #[test]
    fn abort_cancels_active_gesture_once() {
        let mut rec = recognizer();
        let out = run(&mut rec, &held(EDGE_X, -0.2 * W, 10));
        assert_eq!(out[0], GestureEvent::Begin(GestureKind::Open));
        assert_eq!(rec.abort(), Some(GestureEvent::Cancel));
        assert_eq!(
            rec.frame(&frame(&vec![(1, 0.0, 0.0), (2, 0.0, 0.0)], 5.0)),
            []
        );
        assert_eq!(rec.abort(), None);
        // A full lift re-arms the recogniser.
        assert_eq!(rec.frame(&frame(&vec![], 5.1)), []);
        let out = run(&mut rec, &pair_frames(EDGE_X, -0.2 * W, 10, 6.0, 0.0));
        assert_eq!(out[0], GestureEvent::Begin(GestureKind::Open));
    }

    #[test]
    fn reversal_gives_negative_progress() {
        let mut frames = held(EDGE_X, -0.1 * W, 10);
        let t = frames.last().unwrap().1;
        for i in 1..=20 {
            let x = EDGE_X - 0.1 * W + f64::from(i) * 0.01 * W;
            frames.push((
                vec![(1, x, -1000.0), (2, x, 1000.0)],
                t + f64::from(i) * 0.008,
            ));
        }
        frames.push((vec![], t + 0.2));
        let out = run(&mut recognizer(), &frames);
        let ps = updates(&out);
        let max = ps.iter().copied().fold(f32::MIN, f32::max);
        assert!(approx(max, 0.1, 1e-3));
        assert!(*ps.last().unwrap() < 0.0);
        let (p, v) = end_of(&out);
        assert!(approx(p, -0.1, 1e-3));
        assert!(v < -0.6, "moving away from completion: {v}");
    }

    #[test]
    fn flick_velocity() {
        // 0.1 W in 4 frames of 8 ms: ~3.1 W/s.
        let out = run(
            &mut recognizer(),
            &pair_frames(EDGE_X, -0.1 * W, 4, 0.0, 0.0),
        );
        let (p, v) = end_of(&out);
        assert!(approx(p, 0.1, 1e-3));
        assert!(approx(v, 0.1 / 0.032, 0.2), "{v}");
    }

    #[test]
    fn hold_before_release_gives_zero_velocity() {
        let mut frames = held(EDGE_X, -0.2 * W, 10);
        let (hold, t) = last_frame_touches(&frames);
        for i in 1..30 {
            frames.push((hold.clone(), t + f64::from(i) * 0.008));
        }
        frames.push((vec![], t + 0.3));
        let (p, v) = end_of(&run(&mut recognizer(), &frames));
        assert!(approx(p, 0.2, 1e-3));
        assert_eq!(v.to_bits(), 0.0f32.to_bits(), "positive zero, not -0.0");
    }

    #[test]
    fn velocity_uses_recent_window_only() {
        // Fast start, then slow drift for 200 ms: velocity reflects the drift.
        let mut frames = held(EDGE_X, -0.2 * W, 4);
        let t = frames.last().unwrap().1;
        let x = EDGE_X - 0.2 * W;
        for i in 1..26 {
            let xi = x - f64::from(i) * 0.0004 * W;
            frames.push((
                vec![(1, xi, -1000.0), (2, xi, 1000.0)],
                t + f64::from(i) * 0.008,
            ));
        }
        frames.push((vec![], t + 0.3));
        let (_, v) = end_of(&run(&mut recognizer(), &frames));
        assert!(approx(v, 0.0004 / 0.008, 0.01), "{v}");
    }

    #[test]
    fn no_duplicate_updates() {
        let mut frames = held(EDGE_X, -0.1 * W, 5);
        let (last, t) = last_frame_touches(&frames);
        for i in 1..10 {
            frames.push((last.clone(), t + f64::from(i) * 0.008));
        }
        let ps = updates(&run(&mut recognizer(), &frames));
        assert!(
            ps.windows(2).all(|w| w[0].to_bits() != w[1].to_bits()),
            "{ps:?}"
        );
        assert_eq!(ps.len(), 5);
    }

    #[test]
    fn end_when_one_finger_lifts() {
        let mut frames = held(EDGE_X, -0.2 * W, 10);
        let (last, t) = last_frame_touches(&frames);
        let a = last[0];
        frames.push((vec![a], t + 0.008));
        frames.push((vec![(a.0, a.1 - 0.2 * W, a.2)], t + 0.016));
        let out = run(&mut recognizer(), &frames);
        assert!(is_end(out.last().unwrap()));
        assert!(approx(end_of(&out).0, 0.2, 1e-3));
    }

    #[test]
    fn one_gesture_per_sequence() {
        let mut frames = held(EDGE_X, -0.2 * W, 10);
        let (last, t) = last_frame_touches(&frames);
        let a = last[0];
        frames.push((vec![a], t + 0.008));
        frames.push((vec![a, (3, EDGE_X, 0.0)], t + 0.016));
        for i in 1..20 {
            let d = -f64::from(i) * 0.02 * W;
            frames.push((
                vec![(a.0, a.1 + d, a.2), (3, EDGE_X + d, 0.0)],
                t + 0.016 + f64::from(i) * 0.008,
            ));
        }
        let mut rec = recognizer();
        let out = run(&mut rec, &frames);
        assert_eq!(count(&out, is_begin), 1);
        assert_eq!(count(&out, is_end), 1);
        // After everything lifts, a new gesture works again.
        assert_eq!(rec.frame(&frame(&vec![], 10.0)), []);
        let out = run(&mut rec, &pair_frames(EDGE_X, -0.2 * W, 20, 11.0, 0.0));
        assert_eq!(out[0], GestureEvent::Begin(GestureKind::Open));
    }

    #[test]
    fn after_cancel_nothing_until_full_lift() {
        let mut frames = held(EDGE_X, -0.2 * W, 10);
        frames.push((vec![(1, 0.0, 0.0), (2, 0.0, 0.0), (3, 0.0, 0.0)], 1.0));
        frames.push((vec![(1, EDGE_X, 0.0), (2, EDGE_X, 0.0)], 1.01));
        for i in 1..10 {
            let d = -f64::from(i) * 0.03 * W;
            frames.push((
                vec![(1, EDGE_X + d, 0.0), (2, EDGE_X + d, 0.0)],
                1.01 + f64::from(i) * 0.008,
            ));
        }
        let out = run(&mut recognizer(), &frames);
        assert_eq!(count(&out, is_begin), 1);
        assert_eq!(out.last(), Some(&GestureEvent::Cancel));
    }
}
