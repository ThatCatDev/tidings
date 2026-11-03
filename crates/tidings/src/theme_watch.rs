//! Polling the colours source for changes (see [`crate::colors`]).
//!
//! Polling the file's mtime/size once a second (rather than inotify) is
//! deliberate: it copes with the file and its directory not existing yet,
//! and with editors and scripts that replace it by rename, at negligible cost.

use std::path::Path;
use std::time::{Duration, SystemTime};

use tidings_core::theme::Theme;

use crate::app::{AppEvent, AppSender};
use crate::colors::ColorSource;

/// Default poll interval.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Identity of the file's current contents, as far as polling can tell.
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Watches a colours source for changes.
#[derive(Debug)]
pub struct ThemeWatcher {
    source: ColorSource,
    last: Option<(SystemTime, u64)>,
}

impl ThemeWatcher {
    /// Loads the current colours and starts tracking changes from this
    /// version.
    #[must_use]
    pub fn load(source: ColorSource) -> (Theme, Self) {
        // Stamp before reading: a write in between is then seen as a change.
        let last = stamp(source.path());
        (source.load(), Self { source, last })
    }

    /// Polls every `interval`, sending reloaded colours on each change, until
    /// the actor goes away.
    pub async fn run(self, interval: Duration, events: AppSender) {
        let Self { source, mut last } = self;
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let current = stamp(source.path());
            if current == last {
                continue;
            }
            last = current;
            tracing::info!("colours changed: {}", source.path().display());
            if events
                .send(AppEvent::Theme(Box::new(source.load())))
                .is_err()
            {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidings_core::theme::Rgb;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn watching_stops_when_the_actor_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("colors.json");
        let (tx, rx) = mpsc::unbounded_channel();
        let (_, watcher) = ThemeWatcher::load(ColorSource::File(path.clone()));
        drop(rx);
        let task = tokio::spawn(watcher.run(Duration::from_millis(10), tx));
        std::fs::write(&path, "{}").unwrap();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
    }

    async fn next_theme(rx: &mut mpsc::UnboundedReceiver<AppEvent>) -> Theme {
        let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let AppEvent::Theme(theme) = ev else {
            panic!("expected a theme, got {ev:?}")
        };
        *theme
    }

    #[tokio::test]
    async fn changes_are_sent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("colors.json");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (theme, watcher) = ThemeWatcher::load(ColorSource::File(path.clone()));
        assert_eq!(theme, Theme::default());
        let task = tokio::spawn(watcher.run(Duration::from_millis(10), tx));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Replace via rename, as palette tools do.
        let tmp = dir.path().join("tmp.json");
        std::fs::write(&tmp, r##"{"primary":"#ff0000"}"##).unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        assert_eq!(next_theme(&mut rx).await.primary, Rgb(255, 0, 0));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            next_theme(&mut rx).await,
            Theme::default(),
            "deleted file → defaults"
        );
        task.abort();
    }

    #[tokio::test]
    async fn noctalia_palette_changes_are_followed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("noctalia.lua");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (_, watcher) = ThemeWatcher::load(ColorSource::Noctalia(path.clone()));
        let task = tokio::spawn(watcher.run(Duration::from_millis(10), tx));
        std::fs::write(
            &path,
            "local primary = \"rgb(00ff00)\"\nlocal surface = \"rgb(101010)\"\n",
        )
        .unwrap();
        assert_eq!(next_theme(&mut rx).await.primary, Rgb(0, 255, 0));
        task.abort();
    }
}
