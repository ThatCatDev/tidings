//! Toast policy: expiry per urgency/client timeout, the visible cap, and
//! Do Not Disturb rules.

use std::time::Duration;

use crate::model::{Notification, Urgency};

/// Tunable toast behaviour. `Default` matches DESIGN.md.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastPolicy {
    /// Expiry for low-urgency toasts without a client timeout.
    pub low_timeout: Duration,
    /// Expiry for normal-urgency toasts without a client timeout.
    pub normal_timeout: Duration,
    /// Maximum number of toasts on screen at once.
    pub max_visible: usize,
}

impl Default for ToastPolicy {
    fn default() -> Self {
        Self {
            low_timeout: Duration::from_secs(4),
            normal_timeout: Duration::from_secs(6),
            max_visible: 4,
        }
    }
}

impl ToastPolicy {
    /// How long a toast stays up; `None` means until dismissed.
    ///
    /// A positive client timeout wins; `Some(0)` means never. Otherwise the
    /// urgency decides: low and normal use their defaults, critical never expires.
    #[must_use]
    pub fn timeout(&self, n: &Notification) -> Option<Duration> {
        match n.expire_timeout_ms {
            Some(0) => None,
            Some(ms) => Some(Duration::from_millis(u64::from(ms))),
            None => match n.urgency {
                Urgency::Low => Some(self.low_timeout),
                Urgency::Normal => Some(self.normal_timeout),
                Urgency::Critical => None,
            },
        }
    }

    /// Whether a notification should toast: Do Not Disturb suppresses all but critical.
    #[must_use]
    pub fn should_toast(&self, n: &Notification, dnd: bool) -> bool {
        !dnd || n.urgency == Urgency::Critical
    }

    /// Whether a notification is kept in the panel history once its toast is gone.
    #[must_use]
    pub fn keeps_history(&self, n: &Notification) -> bool {
        !n.transient
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NotificationId;
    use crate::store::tests::notif;

    fn with(urgency: Urgency, timeout: Option<u32>) -> Notification {
        let mut n = notif(1, "a", None);
        n.urgency = urgency;
        n.expire_timeout_ms = timeout;
        n
    }

    #[test]
    fn timeouts_by_urgency() {
        let p = ToastPolicy::default();
        assert_eq!(
            p.timeout(&with(Urgency::Low, None)),
            Some(Duration::from_secs(4))
        );
        assert_eq!(
            p.timeout(&with(Urgency::Normal, None)),
            Some(Duration::from_secs(6))
        );
        assert_eq!(p.timeout(&with(Urgency::Critical, None)), None);
    }

    #[test]
    fn client_timeout_wins_and_zero_means_never() {
        let p = ToastPolicy::default();
        assert_eq!(
            p.timeout(&with(Urgency::Low, Some(1500))),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(
            p.timeout(&with(Urgency::Critical, Some(2000))),
            Some(Duration::from_secs(2))
        );
        assert_eq!(p.timeout(&with(Urgency::Normal, Some(0))), None);
    }

    #[test]
    fn dnd_suppresses_all_but_critical() {
        let p = ToastPolicy::default();
        for u in [Urgency::Low, Urgency::Normal, Urgency::Critical] {
            assert!(p.should_toast(&with(u, None), false));
        }
        assert!(!p.should_toast(&with(Urgency::Low, None), true));
        assert!(!p.should_toast(&with(Urgency::Normal, None), true));
        assert!(p.should_toast(&with(Urgency::Critical, None), true));
    }

    #[test]
    fn transient_is_not_kept() {
        let p = ToastPolicy::default();
        let mut n = notif(1, "a", None);
        assert!(p.keeps_history(&n));
        n.transient = true;
        n.id = NotificationId(2);
        assert!(!p.keeps_history(&n));
    }
}
