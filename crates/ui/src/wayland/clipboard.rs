//! Pasting into the reply field: reads the clipboard (the `wl_data_device`
//! selection) as text, without blocking the UI loop.
//!
//! Only paste is supported; tidings never offers anything to copy or drag,
//! so the drag-and-drop and data-source callbacks do nothing.

use std::io::{ErrorKind, Read};

use smithay_client_toolkit::data_device_manager::WritePipe;
use smithay_client_toolkit::data_device_manager::data_device::DataDeviceHandler;
use smithay_client_toolkit::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use smithay_client_toolkit::data_device_manager::data_source::DataSourceHandler;
use smithay_client_toolkit::reexports::calloop::PostAction;
use smithay_client_toolkit::reexports::client::protocol::wl_data_device::WlDataDevice;
use smithay_client_toolkit::reexports::client::protocol::wl_data_device_manager::DndAction;
use smithay_client_toolkit::reexports::client::protocol::wl_data_source::WlDataSource;
use smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface;
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};

use super::App;
use crate::compose::{Edit, MAX_CHARS};

/// Text types, best first: what GTK, Qt, Chrome and terminals offer.
const TEXT_TYPES: [&str; 5] = [
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
];

/// Most bytes read from the clipboard: well over [`MAX_CHARS`] of any
/// script, so a huge paste is cut by the editor, not mid-character here.
const MAX_BYTES: usize = MAX_CHARS * 4 + 4;

/// The best text type among what the clipboard offers.
#[must_use]
pub fn text_type(offered: &[String]) -> Option<&'static str> {
    TEXT_TYPES
        .into_iter()
        .find(|t| offered.iter().any(|o| o.eq_ignore_ascii_case(t)))
}

impl App {
    /// Starts reading the clipboard; the text is inserted at the caret once
    /// it has all arrived.
    pub(super) fn paste(&mut self) {
        let Some(offer) = self
            .data_device
            .as_ref()
            .and_then(|d| d.data().selection_offer())
        else {
            return;
        };
        let Some(mime) = offer.with_mime_types(text_type) else {
            log::debug!("tidings-ui: the clipboard holds no text");
            return;
        };
        let pipe = match offer.receive(mime.to_owned()) {
            Ok(pipe) => pipe,
            Err(e) => {
                log::warn!("tidings-ui: cannot read the clipboard: {e}");
                return;
            }
        };
        let mut bytes = Vec::new();
        let inserted = self.loop_handle.insert_source(pipe, move |(), file, app| {
            // One read per wake-up: the pipe is readable, so it cannot block
            // the UI however slowly the other app writes.
            let mut chunk = [0u8; 8192];
            // `&File` reads too, which is all `NoIoDrop` lends out.
            let mut file: &std::fs::File = file;
            match file.read(&mut chunk) {
                Ok(n) if n > 0 && bytes.len() + n < MAX_BYTES => {
                    bytes.extend_from_slice(&chunk[..n]);
                    return PostAction::Continue;
                }
                // End of the data, or enough of it.
                Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), ErrorKind::Interrupted | ErrorKind::WouldBlock) => {
                    return PostAction::Continue;
                }
                Err(e) => {
                    log::warn!("tidings-ui: reading the clipboard failed: {e}");
                    return PostAction::Remove;
                }
            }
            bytes.truncate(MAX_BYTES);
            let text = String::from_utf8_lossy(&bytes);
            app.edit_reply(&Edit::Insert(
                text.trim_end_matches(['\n', '\r']).to_owned(),
            ));
            PostAction::Remove
        });
        if let Err(e) = inserted {
            log::warn!("tidings-ui: cannot watch the clipboard pipe: {e}");
        }
    }
}

impl DataDeviceHandler for App {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _: f64,
        _: f64,
        _: &WlSurface,
    ) {
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}

impl DataOfferHandler for App {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }

    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}

impl DataSourceHandler for App {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: String,
        _: WritePipe,
    ) {
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_utf8_text() {
        let offered = |v: &[&str]| v.iter().map(|&s| s.to_owned()).collect::<Vec<_>>();
        assert_eq!(
            text_type(&offered(&[
                "image/png",
                "STRING",
                "text/plain;charset=UTF-8"
            ])),
            Some("text/plain;charset=utf-8")
        );
        assert_eq!(
            text_type(&offered(&["TEXT", "UTF8_STRING"])),
            Some("UTF8_STRING")
        );
        assert_eq!(text_type(&offered(&["image/png"])), None);
    }
}
