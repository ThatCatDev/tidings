//! The on-screen toast stack: which notifications are toasting, newest
//! first, and when each expires. Deadlines are monotonic times from
//! [`Clock::monotonic`](crate::clock::Clock::monotonic).

use std::time::Duration;

use crate::model::NotificationId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expiry {
    Never,
    /// Expires at this monotonic time.
    At(Duration),
    /// Hovered: this much time remained when the pointer arrived.
    Paused(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Toast {
    id: NotificationId,
    expiry: Expiry,
    hovered: bool,
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
    pub fn show(
        &mut self,
        id: NotificationId,
        timeout: Option<Duration>,
        now: Duration,
    ) -> Vec<NotificationId> {
        let hovered = self
            .items
            .iter()
            .find(|t| t.id == id)
            .is_some_and(|t| t.hovered);
        self.items.retain(|t| t.id != id);
        let expiry = match timeout {
            None => Expiry::Never,
            Some(d) if hovered => Expiry::Paused(d),
            Some(d) => Expiry::At(now + d),
        };
        self.items.insert(
            0,
            Toast {
                id,
                expiry,
                hovered,
            },
        );
        if self.items.len() > self.max_visible {
            self.items.drain(self.max_visible..).map(|t| t.id).collect()
        } else {
            Vec::new()
        }
    }

    /// Takes `id` off screen. Returns whether it was showing.
    pub fn remove(&mut self, id: NotificationId) -> bool {
        let before = self.items.len();
        self.items.retain(|t| t.id != id);
        self.items.len() != before
    }

    /// Records pointer hover; hovering pauses expiry, leaving resumes it with
    /// the time that remained. Returns whether anything changed.
    pub fn set_hovered(&mut self, id: NotificationId, hovered: bool, now: Duration) -> bool {
        let Some(t) = self.items.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        if t.hovered == hovered {
            return false;
        }
        t.hovered = hovered;
        t.expiry = match (t.expiry, hovered) {
            (Expiry::At(at), true) => Expiry::Paused(at.saturating_sub(now)),
            (Expiry::Paused(left), false) => Expiry::At(now + left),
            (other, _) => other,
        };
        true
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

    /// Showing toasts, newest first, with whether each is paused (hovered).
    pub fn iter(&self) -> impl Iterator<Item = (NotificationId, bool)> + '_ {
        self.items.iter().map(|t| (t.id, t.hovered))
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
        t.iter().map(|(i, _)| i.0).collect()
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
        assert_eq!(t.iter().next(), Some((id(1), true)));
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
}
