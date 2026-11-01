//! The gesture reader thread: opens the trackpad, feeds its events through
//! a [`GestureEngine`] and hands gestures to a sink. Reconnects with
//! exponential backoff when the device is missing or disappears.
//!
//! I/O sits behind [`DeviceOpener`]/[`EventSource`] so the loop is tested
//! with scripted devices.

use std::io;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tidings_core::gesture::GestureEvent;

use crate::engine::GestureEngine;
use crate::event::{AxisRange, InputEvent};
use crate::recognizer::RecognizerConfig;

/// A stream of raw input events.
pub trait EventSource: Send {
    /// Waits up to `timeout` for events and appends any to `out`. Returning
    /// with nothing appended is fine. An error means the device is gone.
    fn read_events(&mut self, timeout: Duration, out: &mut Vec<InputEvent>) -> io::Result<()>;
}

/// Finds and opens the input device.
pub trait DeviceOpener: Send {
    /// The opened device.
    type Source: EventSource;

    /// Opens the device, returning it and its X axis range.
    fn open(&mut self) -> io::Result<(Self::Source, AxisRange)>;
}

/// Reader timing and recognition settings.
#[derive(Debug, Clone)]
pub struct ReaderConfig {
    /// Recognition constants.
    pub recognizer: RecognizerConfig,
    /// How often the thread wakes to check for shutdown and panel state.
    pub poll_interval: Duration,
    /// First reconnect delay; doubles per failure up to `backoff_max`.
    pub backoff_min: Duration,
    /// Longest reconnect delay.
    pub backoff_max: Duration,
}

impl Default for ReaderConfig {
    fn default() -> Self {
        Self {
            recognizer: RecognizerConfig::default(),
            poll_interval: Duration::from_millis(250),
            backoff_min: Duration::from_millis(500),
            backoff_max: Duration::from_secs(30),
        }
    }
}

/// Handle to the reader thread. Dropping it stops and joins the thread.
#[derive(Debug)]
pub struct GestureReader {
    panel_open: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl GestureReader {
    /// Starts the reader thread. `sink` receives every gesture event;
    /// returning `ControlFlow::Break` stops the thread.
    pub fn spawn<O, F>(opener: O, config: ReaderConfig, sink: F) -> io::Result<Self>
    where
        O: DeviceOpener + 'static,
        F: FnMut(GestureEvent) -> ControlFlow<()> + Send + 'static,
    {
        let panel_open = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Shared {
            panel_open: Arc::clone(&panel_open),
            stop: Arc::clone(&stop),
        };
        let thread = thread::Builder::new()
            .name("tidings-gesture".into())
            .spawn(move || run(opener, &config, &shared, sink))?;
        Ok(Self {
            panel_open,
            stop,
            thread: Some(thread),
        })
    }

    /// Tells the recogniser whether the panel is open (see
    /// [`Recognizer::set_panel_open`](crate::Recognizer::set_panel_open)).
    pub fn set_panel_open(&self, open: bool) {
        self.panel_open.store(open, Ordering::Relaxed);
    }

    /// Whether the thread has exited (sink broke or it panicked).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for GestureReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            // The thread checks `stop` at least every poll interval / backoff slice.
            if thread.join().is_err() {
                tracing::error!("gesture reader thread panicked");
            }
        }
    }
}

struct Shared {
    panel_open: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Shared {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Sleeps for `d`, waking early on stop. Returns false if stopped.
    fn sleep(&self, d: Duration, slice: Duration) -> bool {
        let deadline = Instant::now() + d;
        loop {
            if self.stopped() {
                return false;
            }
            let now = Instant::now();
            if now >= deadline {
                return true;
            }
            thread::sleep(slice.min(deadline - now));
        }
    }
}

fn run<O, F>(mut opener: O, config: &ReaderConfig, shared: &Shared, mut sink: F)
where
    O: DeviceOpener,
    F: FnMut(GestureEvent) -> ControlFlow<()>,
{
    let mut backoff = config.backoff_min;
    let mut reported = false;
    while !shared.stopped() {
        match opener.open() {
            Ok((source, range)) => {
                backoff = config.backoff_min;
                reported = false;
                match GestureEngine::new(range, config.recognizer.clone()) {
                    Ok(engine) => {
                        if read_until_lost(source, engine, config, shared, &mut sink).is_break() {
                            return;
                        }
                    }
                    Err(e) => tracing::warn!("unusable trackpad: {e}"),
                }
            }
            Err(e) => {
                if reported {
                    tracing::debug!("trackpad unavailable: {e}; retrying in {backoff:?}");
                } else {
                    tracing::warn!("trackpad unavailable: {e}; retrying with backoff");
                    reported = true;
                }
            }
        }
        if !shared.sleep(backoff, config.poll_interval) {
            return;
        }
        backoff = (backoff * 2).min(config.backoff_max);
    }
}

/// Pumps events until the device errors (Continue) or we must stop (Break).
fn read_until_lost<S, F>(
    mut source: S,
    mut engine: GestureEngine,
    config: &ReaderConfig,
    shared: &Shared,
    sink: &mut F,
) -> ControlFlow<()>
where
    S: EventSource,
    F: FnMut(GestureEvent) -> ControlFlow<()>,
{
    let mut events = Vec::new();
    loop {
        if shared.stopped() {
            return ControlFlow::Break(());
        }
        engine.set_panel_open(shared.panel_open.load(Ordering::Relaxed));
        events.clear();
        match source.read_events(config.poll_interval, &mut events) {
            Ok(()) => {
                for &ev in &events {
                    for gesture in engine.feed(ev) {
                        sink(gesture)?;
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => {
                tracing::warn!("trackpad lost: {e}");
                if let Some(cancel) = engine.abort() {
                    sink(cancel)?;
                }
                return ControlFlow::Continue(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::tests::EventScript;
    use crate::recognizer::tests::{EDGE_X, MID_X, W, X_MAX, X_MIN};
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicU32;
    use std::sync::mpsc;
    use tidings_core::gesture::GestureKind;

    /// Plays back batches of events, then fails (device unplugged).
    struct Scripted {
        batches: VecDeque<Vec<InputEvent>>,
    }

    impl EventSource for Scripted {
        fn read_events(&mut self, _: Duration, out: &mut Vec<InputEvent>) -> io::Result<()> {
            match self.batches.pop_front() {
                Some(b) => {
                    out.extend(b);
                    Ok(())
                }
                None => Err(io::Error::from_raw_os_error(libc::ENODEV)),
            }
        }
    }

    /// Devices handed out in order: each is a list of read batches, `None`
    /// fails with `NotFound`. Once exhausted, every open fails.
    type Devices = Vec<Option<Vec<Vec<InputEvent>>>>;

    struct Opener {
        devices: VecDeque<Option<Vec<Vec<InputEvent>>>>,
        opens: Arc<AtomicU32>,
        /// When set, each open first waits for a message.
        gate: Option<mpsc::Receiver<()>>,
    }

    impl DeviceOpener for Opener {
        type Source = Scripted;

        fn open(&mut self) -> io::Result<(Scripted, AxisRange)> {
            if let Some(gate) = &self.gate {
                let _ = gate.recv();
            }
            self.opens.fetch_add(1, Ordering::SeqCst);
            match self.devices.pop_front().flatten() {
                Some(batches) => Ok((
                    Scripted {
                        batches: batches.into(),
                    },
                    AxisRange {
                        min: X_MIN,
                        max: X_MAX,
                    },
                )),
                None => Err(io::Error::from(io::ErrorKind::NotFound)),
            }
        }
    }

    fn fast_config() -> ReaderConfig {
        ReaderConfig {
            recognizer: RecognizerConfig::default(),
            poll_interval: Duration::from_millis(1),
            backoff_min: Duration::from_millis(1),
            backoff_max: Duration::from_millis(4),
        }
    }

    struct Harness {
        reader: GestureReader,
        rx: mpsc::Receiver<GestureEvent>,
        opens: Arc<AtomicU32>,
    }

    fn spawn(devices: Devices, gate: Option<mpsc::Receiver<()>>) -> Harness {
        let (tx, rx) = mpsc::channel();
        let opens = Arc::new(AtomicU32::new(0));
        let opener = Opener {
            devices: devices.into(),
            opens: Arc::clone(&opens),
            gate,
        };
        let reader = GestureReader::spawn(opener, fast_config(), move |g| {
            if tx.send(g).is_ok() {
                ControlFlow::Continue(())
            } else {
                ControlFlow::Break(())
            }
        })
        .unwrap();
        Harness { reader, rx, opens }
    }

    fn recv(rx: &mpsc::Receiver<GestureEvent>) -> GestureEvent {
        rx.recv_timeout(Duration::from_secs(5)).unwrap()
    }

    /// Receives events until the first non-Update one.
    fn recv_past_updates(rx: &mpsc::Receiver<GestureEvent>) -> GestureEvent {
        loop {
            let ev = recv(rx);
            if !matches!(ev, GestureEvent::Update { .. }) {
                return ev;
            }
        }
    }

    #[test]
    fn reconnects_after_missing_and_lost_devices() {
        let mut swipe = EventScript::default();
        swipe.swipe(EDGE_X, -0.3 * W, 10, Duration::ZERO, true);
        let mut partial = EventScript::default();
        partial.swipe(EDGE_X, -0.3 * W, 10, Duration::ZERO, false);
        let h = spawn(
            vec![
                None,
                None,
                Some(vec![partial.events]),
                Some(vec![swipe.events]),
            ],
            None,
        );
        assert_eq!(recv(&h.rx), GestureEvent::Begin(GestureKind::Open));
        assert_eq!(
            recv_past_updates(&h.rx),
            GestureEvent::Cancel,
            "device loss cancels"
        );
        assert_eq!(
            recv(&h.rx),
            GestureEvent::Begin(GestureKind::Open),
            "reconnected"
        );
        assert!(matches!(recv_past_updates(&h.rx), GestureEvent::End { .. }));
        assert!(h.opens.load(Ordering::SeqCst) >= 4);
    }

    #[test]
    fn panel_state_reaches_the_recognizer() {
        let mut close = EventScript::default();
        close.swipe(MID_X, 0.3 * W, 10, Duration::ZERO, true);
        let (go, gate) = mpsc::channel();
        let h = spawn(vec![Some(vec![close.events])], Some(gate));
        h.reader.set_panel_open(true);
        go.send(()).unwrap();
        assert_eq!(recv(&h.rx), GestureEvent::Begin(GestureKind::Close));
        drop(go); // unblocks later opens so the reader can stop
    }

    #[test]
    fn sink_break_stops_the_thread() {
        let mut swipe = EventScript::default();
        swipe.swipe(EDGE_X, -0.3 * W, 10, Duration::ZERO, true);
        let opener = Opener {
            devices: VecDeque::from([Some(vec![swipe.events])]),
            opens: Arc::new(AtomicU32::new(0)),
            gate: None,
        };
        let reader =
            GestureReader::spawn(opener, fast_config(), |_| ControlFlow::Break(())).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !reader.is_finished() {
            assert!(Instant::now() < deadline, "reader did not stop");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn drop_stops_a_reader_waiting_for_a_device() {
        let h = spawn(vec![], None);
        while h.opens.load(Ordering::SeqCst) < 2 {
            thread::sleep(Duration::from_millis(1));
        }
        drop(h); // must not hang
    }
}
