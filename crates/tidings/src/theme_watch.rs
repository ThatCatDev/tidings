//! Loading `colors.json` and polling it for changes.
//!
//! Polling the file's mtime/size once a second (rather than inotify) is
//! deliberate: it copes with the file and its directory not existing yet,
//! and with editors and scripts that replace it by rename, at negligible cost.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tidings_core::theme::Theme;

use crate::app::{AppEvent, AppSender};

/// Default poll interval.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Loads the theme at `path`; a missing or invalid file gives the dark defaults.
#[must_use]
pub fn load_theme(path: &Path) -> Theme {
    match std::fs::read_to_string(path) {
        Ok(json) => Theme::from_json(&json).unwrap_or_else(|e| {
            tracing::warn!("{}: {e}; using default colours", path.display());
            Theme::default()
        }),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("{}: {e}; using default colours", path.display());
            }
            Theme::default()
        }
    }
}

/// Identity of the file's current contents, as far as polling can tell.
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Watches the colours file for changes.
#[derive(Debug)]
pub struct ThemeWatcher {
    path: PathBuf,
    last: Option<(SystemTime, u64)>,
}

impl ThemeWatcher {
    /// Loads the current theme and starts tracking changes from this version.
    #[must_use]
    pub fn load(path: PathBuf) -> (Theme, Self) {
        // Stamp before reading: a write in between is then seen as a change.
        let last = stamp(&path);
        (load_theme(&path), Self { path, last })
    }

    /// Polls every `interval`, sending a reloaded theme on each change, until
    /// the actor goes away.
    pub async fn run(self, interval: Duration, events: AppSender) {
        let Self { path, mut last } = self;
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let current = stamp(&path);
            if current == last {
                continue;
            }
            last = current;
            tracing::info!("colours changed: {}", path.display());
            if events
                .send(AppEvent::Theme(Box::new(load_theme(&path))))
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
    use tidings_core::theme::{Mode, Rgb};
    use tokio::sync::mpsc;

    #[test]
    fn missing_and_invalid_files_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("colors.json");
        assert_eq!(load_theme(&path), Theme::default());
        std::fs::write(&path, "{ nope").unwrap();
        assert_eq!(load_theme(&path), Theme::default());
        std::fs::write(&path, r##"{"mode":"light","primary":"#112233"}"##).unwrap();
        let t = load_theme(&path);
        assert_eq!((t.mode, t.primary), (Mode::Light, Rgb(0x11, 0x22, 0x33)));
        // Unreadable (here a directory): defaults too.
        assert_eq!(load_theme(dir.path()), Theme::default());
    }

    #[tokio::test]
    async fn watching_stops_when_the_actor_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("colors.json");
        let (tx, rx) = mpsc::unbounded_channel();
        let (_, watcher) = ThemeWatcher::load(path.clone());
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
        let (theme, watcher) = ThemeWatcher::load(path.clone());
        assert_eq!(theme, Theme::default());
        let task = tokio::spawn(watcher.run(Duration::from_millis(10), tx));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Replace via rename, as the hook script does.
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
}
