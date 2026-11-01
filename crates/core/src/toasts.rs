//! The on-screen toast stack: which notifications are toasting, newest
//! first, and when each expires. Deadlines are monotonic times from
//! [`Clock::monotonic`](crate::clock::Clock::monotonic).
//!
//! A toast's countdown is paused while it is hovered *or* expanded; it
//! resumes, with the time that remained, once it is neither.

use std::time::Duration;

use crate::model::NotificationId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expiry {
    Never,
    /// Expires at this monotonic time.
    At(Duration),
    /// Hovered or expanded: this much time remained when the pause began.
    Paused(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Toast {
    id: NotificationId,
    expiry: Expiry,
    hovered: bool,
    expanded: bool,
}

impl Toast {
    fn paused(&self) -> bool {
        self.hovered || self.expanded
    }

    /// Applies `change` to the hover/expand flags, pausing or resuming the
    /// countdown if that flips. Returns whether any flag changed.
    fn update(&mut self, now: Duration, change: impl FnOnce(&mut Self)) -> bool {
        let before = (self.hovered, self.expanded);
        let was_paused = self.paused();
        change(self);
        self.expiry = match (self.expiry, was_paused, self.paused()) {
            (Expiry::At(at), false, true) => Expiry::Paused(at.saturating_sub(now)),
            (Expiry::Paused(left), true, false) => Expiry::At(now + left),
            (other, ..) => other,
        };
        before != (self.hovered, self.expanded)
    }
}

/// A showing toast's presentation state, as reported by [`Toasts::iter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToastState {
    pub id: NotificationId,
    /// The countdown is paused (hovered or expanded).
    pub paused: bool,
    /// Shown as the full card rather than the compact pill.
    pub expanded: bool,
}

/// The toast stack, capped at `max_visible`.
#[derive(Debug, Clone)]
pub struct Toasts {
    /// Newest first.
    items: Vec<Toast>,
    max_visible: usize,
}

impl Toasts {
    /// An empty stack showing at most `max_visible` toasts.
    #[must_use]
    pub fn new(max_visible: usize) -> Self {
        Self {
            items: Vec::new(),
            max_visible,
        }
    }

    /// Shows (or re-shows) `id` on top with a fresh timer. Returns ids pushed
    /// off the bottom of the stack by the cap.
    ///
    /// A re-shown toast (a replaced notification) keeps its hover and expand
    /// state: the content updates in place under the reader, and its fresh
    /// timer stays paused until it is neither hovered nor expanded.
    pub fn show(
        &mut self,
        id: NotificationId,
        timeout: Option<Duration>,
        now: Duration,
    ) -> Vec<NotificationId> {
        let (hovered, expanded) = self
            .items
            .iter()
            .find(|t| t.id == id)
            .map_or((false, false), |t| (t.hovered, t.expanded));
        self.items.retain(|t| t.id != id);
        let expiry = match timeout {
            None => Expiry::Never,
            Some(d) if hovered || expanded => Expiry::Paused(d),
            Some(d) => Expiry::At(now + d),
        };
        self.items.insert(
            0,
            Toast {
                id,
                expiry,
                hovered,
                expanded,
            },
        );
        if self.items.len() > self.max_visible {
            self.items.drain(self.max_visible..).map(|t| t.id).collect()
        } else {
            Vec::new()
        }
    }

    /// Takes `id` off screen, forgetting its hover and expand state. Returns
    /// whether it was showing.
    pub fn remove(&mut self, id: NotificationId) -> bool {
        let before = self.items.len();
        self.items.retain(|t| t.id != id);
        self.items.len() != before
    }

    /// Records pointer hover; hovering pauses expiry, leaving resumes it with
    /// the time that remained (unless the toast is expanded). Returns whether
    /// anything changed.
    pub fn set_hovered(&mut self, id: NotificationId, hovered: bool, now: Duration) -> bool {
        self.find_mut(id)
            .is_some_and(|t| t.update(now, |t| t.hovered = hovered))
    }

    /// Expands or collapses `id`. An expanded toast does not expire;
    /// collapsing resumes the countdown with the time that remained (unless
    /// the toast is still hovered). Returns whether anything changed.
    pub fn set_expanded(&mut self, id: NotificationId, expanded: bool, now: Duration) -> bool {
        self.find_mut(id)
            .is_some_and(|t| t.update(now, |t| t.expanded = expanded))
    }

    fn find_mut(&mut self, id: NotificationId) -> Option<&mut Toast> {
        self.items.iter_mut().find(|t| t.id == id)
    }

    /// Removes and returns every toast whose deadline is at or before `now`.
    pub fn expire(&mut self, now: Duration) -> Vec<NotificationId> {
        let mut expired = Vec::new();
        self.items.retain(|t| {
            let due = matches!(t.expiry, Expiry::At(at) if at <= now);
            if due {
                expired.push(t.id);
            }
            !due
        });
        expired
    }

    /// The earliest running deadline, if any toast is counting down.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Duration> {
        self.items
            .iter()
            .filter_map(|t| match t.expiry {
                Expiry::At(at) => Some(at),
                _ => None,
            })
            .min()
    }

    /// Showing toasts, newest first, with their presentation state.
    pub fn iter(&self) -> impl Iterator<Item = ToastState> + '_ {
        self.items.iter().map(|t| ToastState {
            id: t.id,
            paused: t.paused(),
            expanded: t.expanded,
        })
    }

    /// Whether `id` is toasting.
    #[must_use]
    pub fn contains(&self, id: NotificationId) -> bool {
        self.items.iter().any(|t| t.id == id)
    }

    /// Number of toasts showing.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether no toast is showing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Takes every toast off screen.
    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: [NotificationId; 0] = [];

    const fn id(n: u32) -> NotificationId {
        NotificationId(n)
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn order(t: &Toasts) -> Vec<u32> {
        t.iter().map(|s| s.id.0).collect()
    }

    #[test]
    fn newest_on_top_and_capped() {
        let mut t = Toasts::new(2);
        assert_eq!(t.show(id(1), Some(secs(5)), secs(0)), NONE);
        assert_eq!(t.show(id(2), Some(secs(5)), secs(0)), NONE);
        assert_eq!(t.show(id(3), Some(secs(5)), secs(0)), vec![id(1)]);
        assert_eq!(order(&t), vec![3, 2]);
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn reshow_moves_to_top_and_resets_timer() {
        let mut t = Toasts::new(4);
        t.show(id(1), Some(secs(5)), secs(0));
        t.show(id(2), Some(secs(5)), secs(0));
        t.show(id(1), Some(secs(5)), secs(4));
        assert_eq!(order(&t), vec![1, 2]);
        assert_eq!(t.expire(secs(5)), vec![id(2)]);
        assert_eq!(t.next_deadline(), Some(secs(9)));
        assert_eq!(t.expire(secs(8)), NONE);
        assert_eq!(t.expire(secs(9)), vec![id(1)]);
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn never_expiring_toasts_have_no_deadline() {
        let mut t = Toasts::new(4);
        t.show(id(1), None, secs(0));
        assert_eq!(t.next_deadline(), None);
        assert_eq!(t.expire(secs(1_000_000)), NONE);
        assert!(t.contains(id(1)));
        assert!(t.remove(id(1)));
        assert!(!t.remove(id(1)));
    }

    #[test]
    fn hover_pauses_and_resumes_with_remaining_time() {
        let mut t = Toasts::new(4);
        t.show(id(1), Some(secs(6)), secs(0));
        assert!(t.set_hovered(id(1), true, secs(2)));
        assert!(!t.set_hovered(id(1), true, secs(2)), "no change");
        assert_eq!(
            t.iter().next().map(|s| (s.id, s.paused)),
            Some((id(1), true))
        );
        assert_eq!(t.next_deadline(), None);
        assert_eq!(t.expire(secs(100)), NONE);
        assert!(t.set_hovered(id(1), false, secs(100)));
        assert_eq!(t.next_deadline(), Some(secs(104)));
        assert_eq!(t.expire(secs(104)), vec![id(1)]);
        assert!(!t.set_hovered(id(9), true, secs(0)), "unknown id");
    }

    #[test]
    fn reshow_while_hovered_stays_paused() {
        let mut t = Toasts::new(4);
        t.show(id(1), Some(secs(6)), secs(0));
        t.set_hovered(id(1), true, secs(1));
        t.show(id(1), Some(secs(6)), secs(3));
        assert_eq!(t.next_deadline(), None);
        t.set_hovered(id(1), false, secs(10));
        assert_eq!(t.next_deadline(), Some(secs(16)));
    }

    #[test]
    fn clear_removes_everything() {
        let mut t = Toasts::new(4);
        t.show(id(1), None, secs(0));
        t.clear();
        assert_eq!(t.len(), 0);
    }

    fn state(t: &Toasts, n: u32) -> Option<ToastState> {
        t.iter().find(|s| s.id == id(n))
    }

    #[test]
    fn expand_pauses_and_collapse_resumes_with_remaining_time() {
        let mut t = Toasts::new(4);
        t.show(id(1), Some(secs(6)), secs(0));
        assert!(t.set_expanded(id(1), true, secs(4)));
        assert!(!t.set_expanded(id(1), true, secs(4)), "no change");
        let s = state(&t, 1);
        assert_eq!(s.map(|s| (s.paused, s.expanded)), Some((true, true)));
        assert_eq!(t.next_deadline(), None);
        assert_eq!(t.expire(secs(1_000)), NONE, "expanded never expires");
        assert!(t.set_expanded(id(1), false, secs(1_000)));
        assert_eq!(state(&t, 1).map(|s| s.paused), Some(false));
        assert_eq!(t.next_deadline(), Some(secs(1_002)));
        assert_eq!(t.expire(secs(1_002)), vec![id(1)]);
        assert!(!t.set_expanded(id(9), true, secs(0)), "unknown id");
    }

    #[test]
    fn hover_and_expand_both_hold_the_pause() {
        let mut t = Toasts::new(4);
        t.show(id(1), Some(secs(6)), secs(0));
        t.set_hovered(id(1), true, secs(1));
        t.set_expanded(id(1), true, secs(2));
        // Leaving while expanded keeps the pause.
        t.set_hovered(id(1), false, secs(3));
        assert_eq!(t.next_deadline(), None);
        // Collapsing while hovered keeps it too.
        t.set_hovered(id(1), true, secs(4));
        t.set_expanded(id(1), false, secs(5));
        assert_eq!(t.next_deadline(), None);
        assert_eq!(state(&t, 1).map(|s| s.paused), Some(true));
        // Only when neither holds does the original 5 s remainder resume.
        t.set_hovered(id(1), false, secs(10));
        assert_eq!(t.next_deadline(), Some(secs(15)));
    }

    #[test]
    fn removal_forgets_expanded_state() {
        let mut t = Toasts::new(4);
        t.show(id(1), Some(secs(6)), secs(0));
        t.set_expanded(id(1), true, secs(1));
        t.remove(id(1));
        t.show(id(1), Some(secs(6)), secs(2));
        assert_eq!(state(&t, 1).map(|s| s.expanded), Some(false));
        assert_eq!(t.next_deadline(), Some(secs(8)));
    }

    #[test]
    fn reshow_keeps_expanded_and_paused_with_a_fresh_timer() {
        let mut t = Toasts::new(4);
        t.show(id(1), Some(secs(6)), secs(0));
        t.set_expanded(id(1), true, secs(5));
        t.show(id(1), Some(secs(6)), secs(20));
        assert_eq!(state(&t, 1).map(|s| s.expanded), Some(true));
        assert_eq!(t.next_deadline(), None);
        t.set_expanded(id(1), false, secs(30));
        assert_eq!(t.next_deadline(), Some(secs(36)));
    }

    #[test]
    fn never_expiring_toasts_stay_never_through_expand() {
        let mut t = Toasts::new(4);
        t.show(id(1), None, secs(0));
        t.set_expanded(id(1), true, secs(1));
        t.set_expanded(id(1), false, secs(2));
        assert_eq!(t.next_deadline(), None);
    }
}
