//! Notification storage: the panel history plus everything currently toasting.
//!
//! Entries keep their insertion order (a sequence number); replacing a
//! notification keeps its position, so a replaced card "stays put". Groups are
//! keyed by `desktop_entry`, falling back to `app_name`.

use std::collections::HashSet;

use crate::model::{Notification, NotificationId};

/// Default cap on stored notifications; the oldest are evicted beyond it.
pub const DEFAULT_CAPACITY: usize = 500;

/// A stored notification plus server-side metadata not in the contract model.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The notification as the client last sent it.
    pub notification: Notification,
    /// `resident` hint: invoking an action does not remove the notification.
    pub resident: bool,
    seq: u64,
}

/// Result of [`Store::insert`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InsertOutcome {
    /// An entry with the same id existed and was replaced in place.
    pub replaced: bool,
    /// Oldest entries dropped to stay within capacity.
    pub evicted: Vec<NotificationId>,
}

/// One app's notifications, newest first.
#[derive(Debug, Clone, Copy)]
pub struct Group<'a> {
    /// Grouping key (see [`group_key`]).
    pub key: &'a str,
    /// Whether the user expanded this group.
    pub expanded: bool,
    entries: &'a [Entry],
    /// Indices into `entries`, newest first.
    order: &'a [usize],
}

impl<'a> Group<'a> {
    /// Entries in this group, newest first.
    pub fn entries(&self) -> impl Iterator<Item = &'a Entry> + 'a {
        let entries = self.entries;
        self.order.iter().map(move |&i| &entries[i])
    }

    /// Number of entries in the group.
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Always false: groups only exist while they have entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

/// Grouping key: the desktop entry when present and non-empty, else the app name.
#[must_use]
pub fn group_key(n: &Notification) -> &str {
    match n.desktop_entry.as_deref() {
        Some(entry) if !entry.is_empty() => entry,
        _ => &n.app_name,
    }
}

/// All stored notifications and the per-group expanded state.
#[derive(Debug, Clone)]
pub struct Store {
    /// Ascending by `seq` (oldest first).
    entries: Vec<Entry>,
    next_seq: u64,
    last_id: u32,
    expanded: HashSet<String>,
    capacity: usize,
}

impl Default for Store {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

impl Store {
    /// An empty store holding at most `capacity` notifications (minimum 1).
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            next_seq: 0,
            last_id: 0,
            expanded: HashSet::new(),
            capacity: capacity.max(1),
        }
    }

    /// Allocates a fresh id: never 0 and never one that is currently stored.
    pub fn allocate_id(&mut self) -> NotificationId {
        loop {
            self.last_id = self.last_id.wrapping_add(1);
            let id = NotificationId(self.last_id);
            if self.last_id != 0 && !self.contains(id) {
                return id;
            }
        }
    }

    /// Inserts `n`, replacing (in place) any entry with the same id.
    pub fn insert(&mut self, n: Notification, resident: bool) -> InsertOutcome {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.notification.id == n.id) {
            let old_key = group_key(&entry.notification).to_owned();
            entry.notification = n;
            entry.resident = resident;
            self.prune_expanded(&old_key);
            return InsertOutcome {
                replaced: true,
                evicted: Vec::new(),
            };
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.entries.push(Entry {
            notification: n,
            resident,
            seq,
        });
        let excess = self.entries.len().saturating_sub(self.capacity);
        let evicted: Vec<NotificationId> = self
            .entries
            .drain(..excess)
            .map(|e| e.notification.id)
            .collect();
        if !evicted.is_empty() {
            self.prune_all_expanded();
        }
        InsertOutcome {
            replaced: false,
            evicted,
        }
    }

    /// Looks up an entry.
    #[must_use]
    pub fn get(&self, id: NotificationId) -> Option<&Entry> {
        self.entries.iter().find(|e| e.notification.id == id)
    }

    /// Whether `id` is stored.
    #[must_use]
    pub fn contains(&self, id: NotificationId) -> bool {
        self.get(id).is_some()
    }

    /// Removes and returns an entry.
    pub fn remove(&mut self, id: NotificationId) -> Option<Entry> {
        let idx = self.entries.iter().position(|e| e.notification.id == id)?;
        let entry = self.entries.remove(idx);
        self.prune_expanded(group_key(&entry.notification));
        Some(entry)
    }

    /// Removes every entry in group `key`, returning their ids (newest first).
    pub fn clear_group(&mut self, key: &str) -> Vec<NotificationId> {
        let mut removed = Vec::new();
        self.entries.retain(|e| {
            let keep = group_key(&e.notification) != key;
            if !keep {
                removed.push(e.notification.id);
            }
            keep
        });
        removed.reverse();
        self.expanded.remove(key);
        removed
    }

    /// Removes everything, returning the removed ids (newest first).
    pub fn clear_all(&mut self) -> Vec<NotificationId> {
        self.expanded.clear();
        self.entries
            .drain(..)
            .rev()
            .map(|e| e.notification.id)
            .collect()
    }

    /// Flips a group's expanded state. Returns false if no such group exists.
    pub fn toggle_group(&mut self, key: &str) -> bool {
        if !self
            .entries
            .iter()
            .any(|e| group_key(&e.notification) == key)
        {
            return false;
        }
        if !self.expanded.remove(key) {
            self.expanded.insert(key.to_owned());
        }
        true
    }

    /// Whether group `key` is expanded.
    #[must_use]
    pub fn is_expanded(&self, key: &str) -> bool {
        self.expanded.contains(key)
    }

    /// Number of stored notifications.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Groups, newest group first (by its newest entry), each newest first.
    ///
    /// The returned [`Groups`] owns the index tables the [`Group`]s borrow.
    #[must_use]
    pub fn groups(&self) -> Groups<'_> {
        let mut keyed: Vec<(&str, Vec<usize>)> = Vec::new();
        // Walk newest to oldest so both group order and entry order come out newest first.
        for (i, e) in self.entries.iter().enumerate().rev() {
            let key = group_key(&e.notification);
            match keyed.iter_mut().find(|(k, _)| *k == key) {
                Some((_, idx)) => idx.push(i),
                None => keyed.push((key, vec![i])),
            }
        }
        Groups { store: self, keyed }
    }

    fn prune_expanded(&mut self, key: &str) {
        if !self
            .entries
            .iter()
            .any(|e| group_key(&e.notification) == key)
        {
            self.expanded.remove(key);
        }
    }

    fn prune_all_expanded(&mut self) {
        let live: HashSet<&str> = self
            .entries
            .iter()
            .map(|e| group_key(&e.notification))
            .collect();
        self.expanded.retain(|k| live.contains(k.as_str()));
    }
}

/// Grouped view over a [`Store`]; see [`Store::groups`].
#[derive(Debug)]
pub struct Groups<'a> {
    store: &'a Store,
    keyed: Vec<(&'a str, Vec<usize>)>,
}

impl Groups<'_> {
    /// Iterates groups, newest first.
    pub fn iter(&self) -> impl Iterator<Item = Group<'_>> {
        self.keyed.iter().map(|(key, order)| Group {
            key,
            expanded: self.store.is_expanded(key),
            entries: &self.store.entries,
            order,
        })
    }

    /// Number of groups.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keyed.len()
    }

    /// Whether there are no groups.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keyed.is_empty()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const NONE: [NotificationId; 0] = [];
    use crate::model::Urgency;
    use std::time::SystemTime;

    pub(crate) fn notif(id: u32, app: &str, entry: Option<&str>) -> Notification {
        Notification {
            id: NotificationId(id),
            app_name: app.to_owned(),
            app_icon: None,
            image: None,
            summary: format!("summary {id}"),
            body: String::new(),
            actions: Vec::new(),
            has_default_action: false,
            reply: None,
            origin: None,
            urgency: Urgency::Normal,
            expire_timeout_ms: None,
            desktop_entry: entry.map(str::to_owned),
            transient: false,
            received_at: SystemTime::UNIX_EPOCH,
        }
    }

    fn ids(store: &Store) -> Vec<(String, Vec<u32>)> {
        store
            .groups()
            .iter()
            .map(|g| {
                (
                    g.key.to_owned(),
                    g.entries().map(|e| e.notification.id.0).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn groups_by_desktop_entry_else_app_name_newest_first() {
        let mut s = Store::default();
        s.insert(notif(1, "Firefox", Some("firefox")), false);
        s.insert(notif(2, "Slack", None), false);
        s.insert(notif(3, "Firefox Nightly", Some("firefox")), false);
        s.insert(notif(4, "Slack", Some("")), false);
        assert_eq!(
            ids(&s),
            vec![
                ("Slack".to_owned(), vec![4, 2]),
                ("firefox".to_owned(), vec![3, 1])
            ]
        );
        assert_eq!(s.groups().len(), 2);
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn replace_keeps_position_and_id() {
        let mut s = Store::default();
        s.insert(notif(1, "a", None), false);
        s.insert(notif(2, "b", None), false);
        let mut n = notif(1, "a", None);
        n.summary = "updated".to_owned();
        let out = s.insert(n, true);
        assert!(out.replaced);
        assert_eq!(
            ids(&s),
            vec![("b".to_owned(), vec![2]), ("a".to_owned(), vec![1])]
        );
        let e = s.get(NotificationId(1)).unwrap();
        assert_eq!(e.notification.summary, "updated");
        assert!(e.resident);
    }

    #[test]
    fn replace_can_move_between_groups() {
        let mut s = Store::default();
        s.insert(notif(1, "a", None), false);
        s.insert(notif(2, "a", None), false);
        s.toggle_group("a");
        s.insert(notif(1, "b", None), false);
        assert_eq!(
            ids(&s),
            vec![("a".to_owned(), vec![2]), ("b".to_owned(), vec![1])]
        );
        assert!(s.is_expanded("a"));
        s.insert(notif(2, "b", None), false);
        assert!(
            !s.is_expanded("a"),
            "empty group's expanded state is dropped"
        );
    }

    #[test]
    fn remove_and_clear() {
        let mut s = Store::default();
        for (id, app) in [(1, "a"), (2, "b"), (3, "a"), (4, "c")] {
            s.insert(notif(id, app, None), false);
        }
        assert_eq!(
            s.remove(NotificationId(2)).unwrap().notification.id,
            NotificationId(2)
        );
        assert!(s.remove(NotificationId(2)).is_none());
        assert_eq!(
            s.clear_group("a"),
            vec![NotificationId(3), NotificationId(1)]
        );
        assert_eq!(s.clear_group("a"), NONE);
        assert_eq!(s.clear_all(), vec![NotificationId(4)]);
        assert_eq!(s.len(), 0);
        assert_eq!(s.groups().len(), 0);
    }

    #[test]
    fn expanded_state_toggles_and_is_pruned() {
        let mut s = Store::default();
        assert!(!s.toggle_group("a"), "unknown group");
        s.insert(notif(1, "a", None), false);
        s.insert(notif(2, "a", None), false);
        assert!(s.toggle_group("a"));
        assert!(s.groups().iter().next().unwrap().expanded);
        s.remove(NotificationId(1));
        assert!(s.is_expanded("a"), "group still has a card");
        s.remove(NotificationId(2));
        assert!(!s.is_expanded("a"));
        s.insert(notif(3, "a", None), false);
        assert!(!s.is_expanded("a"), "a new group starts collapsed");
        s.toggle_group("a");
        s.toggle_group("a");
        assert!(!s.is_expanded("a"));
    }

    #[test]
    fn ids_are_fresh_nonzero_and_skip_live_ones_on_wrap() {
        let mut s = Store::default();
        assert_eq!(s.allocate_id(), NotificationId(1));
        assert_eq!(s.allocate_id(), NotificationId(2));
        s.insert(notif(1, "a", None), false);
        s.last_id = u32::MAX - 1;
        assert_eq!(s.allocate_id(), NotificationId(u32::MAX));
        assert_eq!(
            s.allocate_id(),
            NotificationId(2),
            "skips 0 and the live id 1"
        );
    }

    #[test]
    fn capacity_evicts_oldest() {
        let mut s = Store::with_capacity(2);
        s.insert(notif(1, "a", None), false);
        s.toggle_group("a");
        s.insert(notif(2, "b", None), false);
        let out = s.insert(notif(3, "b", None), false);
        assert_eq!(out.evicted, vec![NotificationId(1)]);
        assert!(!s.is_expanded("a"));
        assert_eq!(ids(&s), vec![("b".to_owned(), vec![3, 2])]);
    }
}
