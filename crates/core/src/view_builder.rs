//! Builds the render-ready [`ViewModel`] from domain state.

use crate::clock::Clock;
use crate::model::Notification;
use crate::store::Store;
use crate::time_label::time_label;
use crate::toasts::Toasts;
use crate::view::{CardView, GroupView, ToastView, ViewModel};

/// Snapshot of `store` + `toasts` for the UI, with time labels as of `clock.now()`.
///
/// Toasts whose notification is no longer stored are skipped; at most
/// `max_toasts` are included.
#[must_use]
pub fn build_view(
    store: &Store,
    toasts: &Toasts,
    dnd: bool,
    max_toasts: usize,
    clock: &dyn Clock,
) -> ViewModel {
    let groups = store
        .groups()
        .iter()
        .map(|g| {
            let cards: Vec<CardView> = g.entries().map(|e| card(&e.notification, clock)).collect();
            // A group is never empty, so the newest card names and decorates it.
            let (app_name, icon) = cards
                .first()
                .map(|c| (c.app_name.clone(), c.icon.clone()))
                .unwrap_or_default();
            GroupView {
                key: g.key.to_owned(),
                app_name,
                icon,
                cards,
                expanded: g.expanded,
            }
        })
        .collect();
    let toasts = toasts
        .iter()
        .filter_map(|(id, paused)| {
            store.get(id).map(|e| ToastView {
                card: card(&e.notification, clock),
                paused,
            })
        })
        .take(max_toasts)
        .collect();
    ViewModel {
        groups,
        toasts,
        dnd,
    }
}

/// One card for `n`.
#[must_use]
pub fn card(n: &Notification, clock: &dyn Clock) -> CardView {
    CardView {
        id: n.id,
        app_name: n.app_name.clone(),
        icon: n.app_icon.clone(),
        image: n.image.clone(),
        summary: n.summary.clone(),
        body: n.body.clone(),
        time_label: time_label(n.received_at, clock),
        actions: n.actions.clone(),
        clickable: n.has_default_action,
        urgency: n.urgency,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;
    use crate::model::{Action, Image, NotificationId, Urgency};
    use crate::store::tests::notif;
    use std::time::{Duration, SystemTime};

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn empty_state() {
        let clock = FakeClock::at_unix(0);
        let vm = build_view(&Store::default(), &Toasts::new(4), true, 4, &clock);
        assert_eq!(
            vm,
            ViewModel {
                groups: vec![],
                toasts: vec![],
                dnd: true
            }
        );
    }

    #[test]
    fn groups_cards_and_toasts() {
        let clock = FakeClock::at_unix(10_000);
        let mut store = Store::default();
        let mut toasts = Toasts::new(4);

        let mut a = notif(1, "Firefox", Some("firefox"));
        a.app_icon = Some(Image::IconName("firefox".into()));
        a.received_at = at(10_000 - 300);
        a.actions = vec![Action {
            key: "open".into(),
            label: "Open".into(),
        }];
        a.has_default_action = true;
        a.urgency = Urgency::Critical;
        store.insert(a, false);
        let mut b = notif(2, "Slack", None);
        b.received_at = at(10_000);
        store.insert(b, false);
        let mut c = notif(3, "Firefox Beta", Some("firefox"));
        c.received_at = at(10_000 - 7200);
        store.insert(c, false);
        store.toggle_group("firefox");

        toasts.show(NotificationId(1), None, Duration::ZERO);
        toasts.show(NotificationId(2), None, Duration::ZERO);
        toasts.set_hovered(NotificationId(2), true, Duration::ZERO);
        toasts.show(NotificationId(99), None, Duration::ZERO); // not stored: skipped

        let vm = build_view(&store, &toasts, false, 4, &clock);
        assert!(!vm.dnd);
        let keys: Vec<_> = vm.groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["firefox", "Slack"]);
        let ff = &vm.groups[0];
        assert!(ff.expanded);
        assert_eq!(ff.app_name, "Firefox Beta", "named after the newest card");
        assert_eq!(ff.icon, None);
        let labels: Vec<_> = ff.cards.iter().map(|c| c.time_label.as_str()).collect();
        assert_eq!(labels, ["2h", "5m"]);
        let first = &ff.cards[1];
        assert!(first.clickable);
        assert_eq!(first.urgency, Urgency::Critical);
        assert_eq!(first.icon, Some(Image::IconName("firefox".into())));
        assert_eq!(first.actions.len(), 1);
        assert!(!vm.groups[1].expanded);

        let toast_ids: Vec<_> = vm.toasts.iter().map(|t| (t.card.id.0, t.paused)).collect();
        assert_eq!(toast_ids, [(2, true), (1, false)]);
        assert_eq!(vm.toasts[0].card.time_label, "now");
    }

    #[test]
    fn toasts_are_capped() {
        let clock = FakeClock::at_unix(0);
        let mut store = Store::default();
        let mut toasts = Toasts::new(10);
        for i in 1..=6 {
            store.insert(notif(i, "a", None), false);
            toasts.show(NotificationId(i), None, Duration::ZERO);
        }
        let vm = build_view(&store, &toasts, false, 4, &clock);
        let ids: Vec<_> = vm.toasts.iter().map(|t| t.card.id.0).collect();
        assert_eq!(ids, [6, 5, 4, 3]);
    }
}
