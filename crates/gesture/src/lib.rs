//! Trackpad gesture input for tidings.
//!
//! * [`MtDecoder`] turns raw multitouch protocol B events into frames.
//! * [`Recognizer`] turns frames into [`GestureEvent`](tidings_core::gesture::GestureEvent)s.
//! * [`GestureEngine`] chains the two.
//! * [`GestureReader`] runs them on a thread against the real device
//!   ([`TrackpadOpener`]), reconnecting with backoff.
//!
//! Everything but [`device`] is pure and tested on synthetic input.

pub mod decoder;
pub mod device;
pub mod engine;
pub mod event;
pub mod reader;
pub mod recognizer;

pub use decoder::{Contact, Decoded, Frame, MtDecoder};
pub use device::{EvdevDevice, TrackpadOpener};
pub use engine::GestureEngine;
pub use event::{AxisRange, InputEvent};
pub use reader::{DeviceOpener, EventSource, GestureReader, ReaderConfig};
pub use recognizer::{Recognizer, RecognizerConfig, RecognizerError};
