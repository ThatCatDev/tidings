//! Gesture → panel mapping and the panel's open/closed state machine.
//!
//! Pure: gesture events and open/close requests go in, [`PanelCommand`]s for
//! the UI come out.

use crate::gesture::{GestureEvent, GestureKind};
use crate::ui::PanelCommand;

/// Progress (fraction of pad width) at which the panel is fully out.
pub const FULL_TRAVEL: f32 = 0.30;
/// A release faster than this (pad widths/s) toward completion always completes.
pub const FLICK_VELOCITY: f32 = 0.6;

/// Panel openness for a gesture's progress: `clamp(progress / 0.30, 0, 1)`
/// for Open, and one minus that for Close.
#[must_use]
pub fn openness(kind: GestureKind, progress: f32) -> f32 {
    let fraction = completion(progress);
    match kind {
        GestureKind::Open => fraction,
        GestureKind::Close => 1.0 - fraction,
    }
}

/// Whether a released gesture completes: a flick either way decides,
/// otherwise it completes past halfway.
#[must_use]
pub fn completes(progress: f32, velocity: f32) -> bool {
    if velocity > FLICK_VELOCITY {
        true
    } else if velocity < -FLICK_VELOCITY {
        false
    } else {
        completion(progress) > 0.5
    }
}

fn completion(progress: f32) -> f32 {
    (progress / FULL_TRAVEL).clamp(0.0, 1.0)
}

fn rest(open: bool) -> f32 {
    if open { 1.0 } else { 0.0 }
}

/// Tracks whether the panel is (or is heading to be) open, and any gesture
/// currently dragging it.
#[derive(Debug, Clone, Default)]
pub struct Panel {
    open: bool,
    dragging: Option<GestureKind>,
}

impl Panel {
    /// The panel's target state: true once an open has been decided, even if
    /// the UI is still animating.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Whether a gesture is currently dragging the panel.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.dragging.is_some()
    }

    /// Animate to open (`true`) or closed. Interrupts any drag.
    pub fn set_open(&mut self, open: bool) -> PanelCommand {
        self.dragging = None;
        self.open = open;
        PanelCommand::AnimateTo(rest(open))
    }

    /// Flip the panel.
    pub fn toggle(&mut self) -> PanelCommand {
        self.set_open(!self.open)
    }

    /// The UI finished animating and is resting at `open`.
    pub fn settled(&mut self, open: bool) {
        if self.dragging.is_none() {
            self.open = open;
        }
    }

    /// Maps a gesture event to a panel command.
    ///
    /// A `Begin` that doesn't fit the current state (Open while open, Close
    /// while closed) is ignored along with the rest of its gesture.
    pub fn on_gesture(&mut self, event: GestureEvent) -> Option<PanelCommand> {
        match event {
            GestureEvent::Begin(kind) => {
                let fits = match kind {
                    GestureKind::Open => !self.open,
                    GestureKind::Close => self.open,
                };
                self.dragging = fits.then_some(kind);
                None
            }
            GestureEvent::Update { progress } => self
                .dragging
                .map(|kind| PanelCommand::Track(openness(kind, progress))),
            GestureEvent::End { progress, velocity } => {
                let kind = self.dragging.take()?;
                if completes(progress, velocity) {
                    self.open = kind == GestureKind::Open;
                }
                Some(PanelCommand::AnimateTo(rest(self.open)))
            }
            GestureEvent::Cancel => {
                self.dragging.take()?;
                Some(PanelCommand::AnimateTo(rest(self.open)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn openness_clamps_and_inverts_for_close() {
        assert!(close(openness(GestureKind::Open, 0.0), 0.0));
        assert!(close(openness(GestureKind::Open, 0.15), 0.5));
        assert!(close(openness(GestureKind::Open, 0.45), 1.0));
        assert!(close(openness(GestureKind::Open, -0.1), 0.0));
        assert!(close(openness(GestureKind::Close, 0.0), 1.0));
        assert!(close(openness(GestureKind::Close, 0.06), 0.8));
        assert!(close(openness(GestureKind::Close, 0.5), 0.0));
    }

    #[test]
    fn snap_decision() {
        assert!(completes(0.01, 0.61), "flick completes from anywhere");
        assert!(!completes(0.29, -0.61), "reverse flick aborts past halfway");
        assert!(completes(0.16, 0.0));
        assert!(!completes(0.15, 0.0), "exactly halfway does not complete");
        assert!(!completes(0.1, 0.6), "0.6 itself is not a flick");
        assert!(completes(0.2, -0.6));
    }

    #[test]
    fn open_gesture_tracks_then_completes() {
        let mut p = Panel::default();
        assert_eq!(p.on_gesture(GestureEvent::Begin(GestureKind::Open)), None);
        assert!(p.is_dragging());
        assert_eq!(
            p.on_gesture(GestureEvent::Update { progress: 0.15 }),
            Some(PanelCommand::Track(0.5))
        );
        assert!(!p.is_open(), "not open until released");
        assert_eq!(
            p.on_gesture(GestureEvent::End {
                progress: 0.2,
                velocity: 0.0
            }),
            Some(PanelCommand::AnimateTo(1.0))
        );
        assert!(p.is_open());
        assert!(!p.is_dragging());
    }

    #[test]
    fn open_gesture_springs_back() {
        let mut p = Panel::default();
        p.on_gesture(GestureEvent::Begin(GestureKind::Open));
        assert_eq!(
            p.on_gesture(GestureEvent::End {
                progress: 0.1,
                velocity: 0.1
            }),
            Some(PanelCommand::AnimateTo(0.0))
        );
        assert!(!p.is_open());
    }

    #[test]
    fn close_gesture_closes() {
        let mut p = Panel::default();
        p.set_open(true);
        p.on_gesture(GestureEvent::Begin(GestureKind::Close));
        assert_eq!(
            p.on_gesture(GestureEvent::Update { progress: 0.03 }),
            Some(PanelCommand::Track(0.9))
        );
        assert_eq!(
            p.on_gesture(GestureEvent::End {
                progress: 0.05,
                velocity: 1.5
            }),
            Some(PanelCommand::AnimateTo(0.0))
        );
        assert!(!p.is_open());
    }

    #[test]
    fn mismatched_gestures_are_ignored() {
        let mut p = Panel::default();
        assert_eq!(p.on_gesture(GestureEvent::Begin(GestureKind::Close)), None);
        assert_eq!(p.on_gesture(GestureEvent::Update { progress: 0.2 }), None);
        assert_eq!(
            p.on_gesture(GestureEvent::End {
                progress: 0.2,
                velocity: 2.0
            }),
            None
        );
        assert!(!p.is_open());
        p.set_open(true);
        p.on_gesture(GestureEvent::Begin(GestureKind::Open));
        assert_eq!(p.on_gesture(GestureEvent::Update { progress: 0.2 }), None);
    }

    #[test]
    fn cancel_restores_previous_state() {
        let mut p = Panel::default();
        p.on_gesture(GestureEvent::Begin(GestureKind::Open));
        p.on_gesture(GestureEvent::Update { progress: 0.25 });
        assert_eq!(
            p.on_gesture(GestureEvent::Cancel),
            Some(PanelCommand::AnimateTo(0.0))
        );
        assert!(!p.is_open());
        assert_eq!(
            p.on_gesture(GestureEvent::Cancel),
            None,
            "nothing to cancel"
        );
    }

    #[test]
    fn toggle_and_settle() {
        let mut p = Panel::default();
        assert_eq!(p.toggle(), PanelCommand::AnimateTo(1.0));
        assert!(p.is_open());
        p.settled(false);
        assert!(!p.is_open(), "UI says it rests closed");
        p.on_gesture(GestureEvent::Begin(GestureKind::Open));
        p.settled(true);
        assert!(!p.is_open(), "settle during a drag is stale");
        assert_eq!(p.set_open(false), PanelCommand::AnimateTo(0.0));
        assert!(!p.is_dragging(), "explicit request interrupts the drag");
    }
}
