//! The inline reply: which card is being replied to, the drafts, and the
//! keyboard focus the field needs.
//!
//! Layer surfaces normally take no keyboard (toasts) or take it only when
//! clicked (the panel). Opening a reply field asks for it exclusively, so
//! typing goes to the field straight away. Once it arrives the surface drops
//! back to on-demand, so clicking another window takes the keyboard away
//! again as usual; clicking the field takes it back.
//!
//! Drafts outlive the field: closing it (or the toast moving on) keeps what
//! was typed until it is sent, cancelled with Esc, or its notification goes.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use smithay_client_toolkit::seat::keyboard::KeyEvent;
use smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity;
use tidings_core::model::NotificationId;
use tidings_core::ui::UiEvent;

use super::{App, SurfaceId};
use crate::compose::{Edit, KeyCommand, LineEditor, ReplyDraft, command_for};
use crate::scene::ReplyClick;

/// The reply field that is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenReply {
    pub id: NotificationId,
    /// The surface showing it.
    pub surface: SurfaceId,
    /// Whether the surface has the keyboard.
    pub focused: bool,
}

/// Reply state: the open field and every unsent draft.
#[derive(Debug, Default)]
pub struct Replies {
    pub open: Option<OpenReply>,
    drafts: HashMap<NotificationId, LineEditor>,
}

impl Replies {
    /// The open field as layout sees it.
    #[must_use]
    pub fn draft(&self) -> Option<ReplyDraft> {
        let open = self.open?;
        let editor = self.drafts.get(&open.id)?;
        Some(ReplyDraft::new(open.id, editor, open.focused))
    }

    /// The open field's draft, if it is open on `surface`.
    #[must_use]
    pub fn draft_on(&self, surface: SurfaceId) -> Option<ReplyDraft> {
        self.open
            .filter(|o| o.surface == surface)
            .and_then(|_| self.draft())
    }
}

impl App {
    /// Handles a click on a reply target on `surface`.
    pub(super) fn reply_click(&mut self, surface: SurfaceId, click: ReplyClick) {
        match click {
            ReplyClick::Focus(id) => self.open_reply(surface, id),
            ReplyClick::Send(id) => {
                if self.replies.open.is_some_and(|o| o.id == id) {
                    self.send_reply();
                }
            }
        }
    }

    /// Opens (or re-focuses) the reply field for `id` on `surface`, and asks
    /// for the keyboard.
    fn open_reply(&mut self, surface: SurfaceId, id: NotificationId) {
        if let Some(open) = self.replies.open
            && (open.id, open.surface) != (id, surface)
        {
            self.close_reply();
        }
        self.replies.drafts.entry(id).or_default();
        // The surface may have the keyboard already (the panel does once
        // clicked), in which case no focus event is coming.
        let focused = self.keyboard_on == Some(surface);
        self.replies.open = Some(OpenReply {
            id,
            surface,
            focused,
        });
        if !focused && let Some(c) = self.canvas_mut(surface) {
            c.set_keyboard(Some(KeyboardInteractivity::Exclusive));
            // Ask now rather than at the end of this turn of the loop, so the
            // keyboard moves before the user (or a test) starts typing.
            if let Err(e) = self.conn.flush() {
                log::debug!("tidings-ui: flush failed: {e}");
            }
        }
        self.reply_changed();
    }

    /// Closes the field, keeping its draft, and gives the keyboard back.
    pub(super) fn close_reply(&mut self) {
        let Some(open) = self.replies.open.take() else {
            return;
        };
        if let Some(c) = self.canvas_mut(open.surface) {
            c.set_keyboard(None);
        }
        self.reply_changed();
    }

    /// `surface` is going away: closes the field if it is open there, and
    /// forgets that it had the keyboard (its leave event may never arrive
    /// once the surface is gone).
    pub(super) fn close_reply_on(&mut self, surface: SurfaceId) {
        if self.keyboard_on == Some(surface) {
            self.keyboard_on = None;
        }
        if self.replies.open.is_some_and(|o| o.surface == surface) {
            self.close_reply();
        }
    }

    /// Sends the open draft, if there is anything to send.
    fn send_reply(&mut self) {
        let Some(draft) = self.replies.draft().filter(ReplyDraft::can_send) else {
            return;
        };
        self.replies.drafts.remove(&draft.id);
        self.close_reply();
        self.emit(UiEvent::Reply {
            id: draft.id,
            text: draft.text.trim().to_owned(),
        });
    }

    /// A key press (or repeat). Returns whether the reply field took it.
    pub(super) fn reply_key(&mut self, event: &KeyEvent) -> bool {
        let Some(open) = self.replies.open.filter(|o| o.focused) else {
            return false;
        };
        let Some(command) = command_for(event.keysym, event.utf8.as_deref(), self.mods) else {
            // Swallow the rest (Tab, F-keys...) while typing.
            return true;
        };
        match command {
            KeyCommand::Submit => self.send_reply(),
            KeyCommand::Cancel => {
                self.replies.drafts.remove(&open.id);
                self.close_reply();
            }
            KeyCommand::Paste => self.paste(),
            KeyCommand::Edit(edit) => self.edit_reply(&edit),
        }
        true
    }

    /// Applies `edit` to the open draft.
    pub(super) fn edit_reply(&mut self, edit: &Edit) {
        let Some(open) = self.replies.open else {
            return;
        };
        if self
            .replies
            .drafts
            .get_mut(&open.id)
            .is_some_and(|e| e.apply(edit))
        {
            self.reply_changed();
        }
    }

    /// Keyboard focus entered (`true`) or left `surface`.
    pub(super) fn reply_focus(&mut self, surface: SurfaceId, focused: bool) {
        let Some(open) = self.replies.open.as_mut().filter(|o| o.surface == surface) else {
            return;
        };
        if open.focused == focused {
            return;
        }
        open.focused = focused;
        if focused && let Some(c) = self.canvas_mut(surface) {
            // Got it: let a click elsewhere take it away again.
            c.set_keyboard(Some(KeyboardInteractivity::OnDemand));
        }
        self.reply_changed();
    }

    /// After a new view: forgets drafts whose notification is gone, and
    /// closes the field if its card no longer shows a reply on its surface
    /// (the toast collapsed or moved on, say).
    pub(super) fn prune_replies(&mut self) {
        let view = &self.view;
        let in_panel = |id| {
            view.groups
                .iter()
                .flat_map(|g| &g.cards)
                .any(|c| c.id == id && c.reply.is_some())
        };
        let in_toasts = |id| {
            view.toasts
                .iter()
                .any(|t| t.card.id == id && t.expanded && t.card.reply.is_some())
        };
        let live: HashSet<NotificationId> = view
            .groups
            .iter()
            .flat_map(|g| &g.cards)
            .map(|c| c.id)
            .chain(view.toasts.iter().map(|t| t.card.id))
            .collect();
        let shown = self.replies.open.is_none_or(|o| match o.surface {
            SurfaceId::Panel => in_panel(o.id),
            SurfaceId::Toasts => in_toasts(o.id),
            SurfaceId::Catcher => false,
        });
        self.replies.drafts.retain(|id, _| live.contains(id));
        if !shown {
            self.close_reply();
        }
    }

    /// Pushes the open draft to the surfaces' layouts and repaints them.
    fn reply_changed(&mut self) {
        let panel = self.replies.draft_on(SurfaceId::Panel);
        if self.panel_view.ui.reply != panel {
            self.panel_view.ui.reply = panel;
            self.panel_view.needs_layout = true;
            self.schedule(SurfaceId::Panel);
        }
        let toasts = self.replies.draft_on(SurfaceId::Toasts);
        if self.toast_stack.reply != toasts {
            self.toast_stack.reply = toasts;
            self.sync_toasts(Instant::now());
        }
    }
}
