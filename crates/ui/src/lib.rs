//! tidings-ui: the Wayland layer-shell UI of the tidings notification centre.
//!
//! The UI runs on its own thread with its own Wayland connection and calloop
//! event loop. The daemon drives it through the [`UiHandle`] returned by
//! [`spawn`] and receives [`UiEvent`]s back; the UI never mutates domain
//! state.
//!
//! Layers, each usable on its own:
//! - [`layout`]: pure `(ViewModel, size, scale, hover) -> Scene` (positioned
//!   paint nodes + hit regions), text measured through [`text::TextMeasure`].
//! - [`scene`]: the element tree, hit-testing and damage diffing.
//! - [`render`]: scene → tiny-skia pixmap (cosmic-text glyphs, icons, SVG).
//! - [`anim`]: spring/easing math, panel openness and toast motion.
//! - `wayland` (private): layer surfaces, fractional scaling, input routing.

pub mod anim;
mod error;
pub mod geom;
pub mod layout;
pub mod render;
pub mod scene;
pub mod style;
pub mod text;
mod wayland;

use std::sync::mpsc;

use smithay_client_toolkit::reexports::calloop::channel;
use tidings_core::config::AnimationConfig;
use tidings_core::ui::{UiEvent, UiHandle, UiUpdate};

pub use error::UiError;

/// Daemon-side handle: queues updates into the UI loop and wakes it.
struct ChannelHandle(channel::Sender<UiUpdate>);

impl UiHandle for ChannelHandle {
    fn send(&self, update: UiUpdate) {
        if self.0.send(update).is_err() {
            log::debug!("tidings-ui: UI thread has exited; update dropped");
        }
    }
}

/// Starts the UI thread (own Wayland connection + event loop).
///
/// Returns once the connection and globals are set up, so a missing
/// compositor or protocol is reported here rather than on the thread.
/// Motion follows the user's `animation` settings. The thread exits on
/// [`UiUpdate::Shutdown`] or when the handle is dropped.
pub fn spawn(
    events: mpsc::Sender<UiEvent>,
    animation: AnimationConfig,
) -> Result<Box<dyn UiHandle>, UiError> {
    let timing = anim::Timing::new(&animation);
    let (init_tx, init_rx) = mpsc::sync_channel::<Result<channel::Sender<UiUpdate>, UiError>>(1);
    std::thread::Builder::new()
        .name("tidings-ui".into())
        .spawn(move || match wayland::setup(events, timing) {
            Ok((event_loop, app, tx)) => {
                if init_tx.send(Ok(tx)).is_ok() {
                    wayland::run(event_loop, app);
                }
            }
            Err(e) => {
                // The receiver only disappears if `spawn` itself is gone.
                let _ = init_tx.send(Err(e));
            }
        })?;
    let tx = init_rx.recv().map_err(|_| UiError::Startup)??;
    Ok(Box::new(ChannelHandle(tx)))
}
