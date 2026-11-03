//! Pasting into the reply field: reads the clipboard (the seat's
//! `wl_data_device` selection) as text, without blocking the UI loop.
//!
//! tidings only ever pastes, so this speaks the protocol directly rather
//! than through sctk's data-device helpers, which would also want handlers
//! for drag-and-drop and for offering data that tidings never uses.

use std::io::{ErrorKind, Read};
use std::os::fd::AsFd;
use std::sync::{Mutex, PoisonError};

use smithay_client_toolkit::reexports::calloop::generic::Generic;
use smithay_client_toolkit::reexports::calloop::{Interest, Mode, PostAction};
use smithay_client_toolkit::reexports::client::protocol::wl_data_device::{self, WlDataDevice};
use smithay_client_toolkit::reexports::client::protocol::wl_data_device_manager::WlDataDeviceManager;
use smithay_client_toolkit::reexports::client::protocol::wl_data_offer::{self, WlDataOffer};
use smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat;
use smithay_client_toolkit::reexports::client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop, event_created_child,
};

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

/// The types an offer comes in, as the compositor announces them.
type OfferTypes = Mutex<Vec<String>>;

/// A seat's clipboard.
#[derive(Debug)]
pub struct Clipboard {
    _device: WlDataDevice,
    /// What is on the clipboard (the compositor only says while tidings
    /// has the keyboard, which is exactly when it can paste).
    selection: Option<WlDataOffer>,
}

impl Clipboard {
    /// The clipboard of `seat`.
    pub fn new(manager: &WlDataDeviceManager, seat: &WlSeat, qh: &QueueHandle<App>) -> Self {
        Self {
            _device: manager.get_data_device(seat, qh, ()),
            selection: None,
        }
    }
}

impl App {
    /// Starts reading the clipboard; the text is inserted at the caret once
    /// it has all arrived.
    pub(super) fn paste(&mut self) {
        let Some(offer) = self.clipboard.as_ref().and_then(|c| c.selection.as_ref()) else {
            return;
        };
        let mime = offer
            .data::<OfferTypes>()
            .and_then(|t| text_type(&t.lock().unwrap_or_else(PoisonError::into_inner)));
        let Some(mime) = mime else {
            log::debug!("tidings-ui: the clipboard holds no text");
            return;
        };
        let (reader, writer) = match std::io::pipe() {
            Ok(pipe) => pipe,
            Err(e) => {
                log::warn!("tidings-ui: cannot read the clipboard: {e}");
                return;
            }
        };
        offer.receive(mime.to_owned(), writer.as_fd());
        // Only the app sending the text may keep the write end open, or the
        // read would never see the end.
        drop(writer);
        let source = Generic::new(reader, Interest::READ, Mode::Level);
        let mut bytes = Vec::new();
        let inserted = self
            .loop_handle
            .insert_source(source, move |_, reader, app| {
                // One read per wake-up: the pipe is readable, so it cannot block
                // the UI however slowly the other app writes.
                let mut chunk = [0u8; 8192];
                match (&**reader).read(&mut chunk) {
                    Ok(n) if n > 0 && bytes.len() + n < MAX_BYTES => {
                        bytes.extend_from_slice(&chunk[..n]);
                        return Ok(PostAction::Continue);
                    }
                    // The end of the text, or enough of it.
                    Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == ErrorKind::Interrupted => {
                        return Ok(PostAction::Continue);
                    }
                    Err(e) => {
                        log::warn!("tidings-ui: reading the clipboard failed: {e}");
                        return Ok(PostAction::Remove);
                    }
                }
                bytes.truncate(MAX_BYTES);
                let text = String::from_utf8_lossy(&bytes);
                app.edit_reply(&Edit::Insert(
                    text.trim_end_matches(['\n', '\r']).to_owned(),
                ));
                Ok(PostAction::Remove)
            });
        if let Err(e) = inserted {
            log::warn!("tidings-ui: cannot watch the clipboard pipe: {e}");
        }
    }

    /// The clipboard changed (or tidings got the keyboard and was told what
    /// is on it).
    fn set_selection(&mut self, offer: Option<WlDataOffer>) {
        let Some(clipboard) = self.clipboard.as_mut() else {
            return;
        };
        if let Some(old) = std::mem::replace(&mut clipboard.selection, offer) {
            old.destroy();
        }
    }
}

impl Dispatch<WlDataDevice, ()> for App {
    fn event(
        app: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::Selection { id } => app.set_selection(id),
            // Something dragged over a notification: nothing takes drops.
            wl_data_device::Event::Enter {
                id: Some(offer), ..
            } => offer.destroy(),
            _ => {}
        }
    }

    event_created_child!(App, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, OfferTypes::default()),
    ]);
}

impl Dispatch<WlDataOffer, OfferTypes> for App {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        types: &OfferTypes,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            types
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(mime_type);
        }
    }
}

delegate_noop!(App: WlDataDeviceManager);

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
