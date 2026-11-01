//! Trackpad discovery (sysfs) and a read-only evdev device.
//!
//! The device is never grabbed: tidings only listens alongside libinput.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::event::{ABS_MT_POSITION_X, AxisRange, InputEvent};
use crate::reader::{DeviceOpener, EventSource};

/// `INPUT_PROP_POINTER`: the device needs an on-screen pointer (a touchpad,
/// as opposed to a touchscreen).
const INPUT_PROP_POINTER: usize = 0x00;

/// Default sysfs directory listing input devices.
pub const SYS_CLASS_INPUT: &str = "/sys/class/input";
/// Default directory holding the event device nodes.
pub const DEV_INPUT: &str = "/dev/input";

/// Size of the kernel's `struct input_event` on 64-bit Linux:
/// `struct timeval` (2 × 8) + type (2) + code (2) + value (4).
const EVENT_SIZE: usize = 24;
const _: () = assert!(
    size_of::<libc::input_event>() == EVENT_SIZE,
    "64-bit Linux layout"
);

/// Whether a sysfs bitmask (space-separated hex words, most significant
/// first, each `usize` bits wide) has `bit` set.
#[must_use]
pub fn has_bit(bitmask: &str, bit: usize) -> bool {
    let word_bits = usize::BITS as usize;
    bitmask
        .split_whitespace()
        .rev()
        .nth(bit / word_bits)
        .and_then(|w| usize::from_str_radix(w, 16).ok())
        .is_some_and(|w| w & (1 << (bit % word_bits)) != 0)
}

/// Finds the trackpad: the first `event*` whose name contains "Trackpad",
/// with `INPUT_PROP_POINTER` and an `ABS_MT_POSITION_X` axis. Returns the
/// event node's name (e.g. `event8`).
#[must_use]
pub fn find_trackpad(sys_class_input: &Path) -> Option<String> {
    let mut names: Vec<String> = fs::read_dir(sys_class_input)
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("event"))
        .collect();
    names.sort_by_key(|n| n["event".len()..].parse::<u32>().unwrap_or(u32::MAX));
    names.into_iter().find(|name| {
        let dev = sys_class_input.join(name).join("device");
        let read = |rel: &str| fs::read_to_string(dev.join(rel)).unwrap_or_default();
        read("name").contains("Trackpad")
            && has_bit(&read("properties"), INPUT_PROP_POINTER)
            && has_bit(&read("capabilities/abs"), usize::from(ABS_MT_POSITION_X))
    })
}

/// `_IOC(dir, 'E', nr, size)` from `<asm-generic/ioctl.h>`.
const fn evdev_ioctl(dir: u32, nr: u32, size: usize) -> u32 {
    // Sizes passed here are small struct sizes, well within the 14-bit field.
    #[allow(clippy::cast_possible_truncation)]
    let size = size as u32;
    (dir << 30) | (size << 16) | ((b'E' as u32) << 8) | nr
}
const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;

/// An open evdev node, read-only and non-blocking.
#[derive(Debug)]
pub struct EvdevDevice {
    file: File,
}

impl EvdevDevice {
    /// Opens `path` read-only. Does not grab the device.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)?;
        Ok(Self { file })
    }

    /// The range of absolute axis `code` (`EVIOCGABS`).
    pub fn abs_range(&self, code: u16) -> io::Result<AxisRange> {
        let mut info = libc::input_absinfo {
            value: 0,
            minimum: 0,
            maximum: 0,
            fuzz: 0,
            flat: 0,
            resolution: 0,
        };
        let request = evdev_ioctl(
            IOC_READ,
            0x40 + u32::from(code),
            size_of::<libc::input_absinfo>(),
        );
        // SAFETY: EVIOCGABS writes exactly one `input_absinfo` into the pointer we pass,
        // which points to a live, properly aligned value of that type.
        let rc = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                libc::Ioctl::from(request),
                &raw mut info,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(AxisRange {
            min: info.minimum,
            max: info.maximum,
        })
    }

    /// Asks for `CLOCK_MONOTONIC` event timestamps (`EVIOCSCLOCKID`). This
    /// only affects this file handle.
    pub fn use_monotonic_clock(&self) -> io::Result<()> {
        let clock: libc::c_int = libc::CLOCK_MONOTONIC;
        let request = evdev_ioctl(IOC_WRITE, 0xa0, size_of::<libc::c_int>());
        // SAFETY: EVIOCSCLOCKID reads one `int` from the pointer, which points to a live c_int.
        let rc = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                libc::Ioctl::from(request),
                &raw const clock,
            )
        };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn wait_readable(&self, timeout: Duration) -> io::Result<bool> {
        let mut pfd = libc::pollfd {
            fd: self.file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = libc::c_int::try_from(timeout.as_millis()).unwrap_or(libc::c_int::MAX);
        // SAFETY: `pfd` is a single valid pollfd and we pass a count of 1.
        let rc = unsafe { libc::poll(&raw mut pfd, 1, ms) };
        match rc {
            0 => Ok(false),
            n if n > 0 => {
                if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0
                    && pfd.revents & libc::POLLIN == 0
                {
                    return Err(io::Error::from_raw_os_error(libc::ENODEV));
                }
                Ok(true)
            }
            _ => Err(io::Error::last_os_error()),
        }
    }
}

/// Decodes one native-endian 64-bit `struct input_event`.
fn parse_event(b: &[u8; EVENT_SIZE]) -> InputEvent {
    let i64_at = |o: usize| {
        i64::from_ne_bytes([
            b[o],
            b[o + 1],
            b[o + 2],
            b[o + 3],
            b[o + 4],
            b[o + 5],
            b[o + 6],
            b[o + 7],
        ])
    };
    let u16_at = |o: usize| u16::from_ne_bytes([b[o], b[o + 1]]);
    let secs = u64::try_from(i64_at(0)).unwrap_or(0);
    let micros = u64::try_from(i64_at(8)).unwrap_or(0);
    InputEvent {
        time: Duration::from_secs(secs) + Duration::from_micros(micros),
        kind: u16_at(16),
        code: u16_at(18),
        value: i32::from_ne_bytes([b[20], b[21], b[22], b[23]]),
    }
}

impl EventSource for EvdevDevice {
    fn read_events(&mut self, timeout: Duration, out: &mut Vec<InputEvent>) -> io::Result<()> {
        if !self.wait_readable(timeout)? {
            return Ok(());
        }
        let mut buf = [0u8; EVENT_SIZE * 64];
        let n = match self.file.read(&mut buf) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        };
        out.extend(buf[..n].as_chunks::<EVENT_SIZE>().0.iter().map(parse_event));
        Ok(())
    }
}

/// Opens the trackpad found by [`find_trackpad`].
#[derive(Debug, Clone)]
pub struct TrackpadOpener {
    sys_class_input: PathBuf,
    dev_input: PathBuf,
}

impl Default for TrackpadOpener {
    fn default() -> Self {
        Self {
            sys_class_input: SYS_CLASS_INPUT.into(),
            dev_input: DEV_INPUT.into(),
        }
    }
}

impl TrackpadOpener {
    /// An opener looking in non-default directories (for tests).
    #[must_use]
    pub fn new(sys_class_input: PathBuf, dev_input: PathBuf) -> Self {
        Self {
            sys_class_input,
            dev_input,
        }
    }

    /// The trackpad's device node, if one is present.
    #[must_use]
    pub fn find(&self) -> Option<PathBuf> {
        find_trackpad(&self.sys_class_input).map(|name| self.dev_input.join(name))
    }
}

impl DeviceOpener for TrackpadOpener {
    type Source = EvdevDevice;

    fn open(&mut self) -> io::Result<(EvdevDevice, AxisRange)> {
        let path = self
            .find()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no trackpad found"))?;
        let device = EvdevDevice::open(&path)?;
        let range = device.abs_range(ABS_MT_POSITION_X)?;
        if let Err(e) = device.use_monotonic_clock() {
            tracing::debug!("EVIOCSCLOCKID failed, using realtime timestamps: {e}");
        }
        tracing::info!(path = %path.display(), x_min = range.min, x_max = range.max, "trackpad opened");
        Ok((device, range))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmasks() {
        assert!(has_bit("5", 0));
        assert!(!has_bit("5", 1));
        assert!(has_bit("5", 2));
        assert!(!has_bit("0", 0));
        assert!(!has_bit("", 0));
        // The real trackpad's abs capabilities: ABS_X, ABS_Y and the MT axes.
        let abs = "67f800001000003";
        assert!(has_bit(abs, 0));
        assert!(has_bit(abs, usize::from(ABS_MT_POSITION_X)));
        assert!(has_bit(abs, 0x39));
        assert!(!has_bit(abs, 2));
        // Multi-word: bit 64 lives in the second-to-last word.
        assert!(has_bit("1 0", 64));
        assert!(!has_bit("1 0", 0));
        assert!(!has_bit("zz", 0));
    }

    fn fake_device(root: &Path, node: &str, name: &str, props: &str, abs: &str) {
        let dev = root.join(node).join("device");
        fs::create_dir_all(dev.join("capabilities")).unwrap();
        fs::write(dev.join("name"), format!("{name}\n")).unwrap();
        fs::write(dev.join("properties"), format!("{props}\n")).unwrap();
        fs::write(dev.join("capabilities/abs"), format!("{abs}\n")).unwrap();
    }

    #[test]
    fn discovery_picks_the_pointer_trackpad() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Same name, but the keyboard half: no pointer property, no axes.
        fake_device(
            root,
            "event7",
            "Apple Inc. Apple Internal Keyboard / Trackpad",
            "0",
            "0",
        );
        fake_device(
            root,
            "event9",
            "Apple Inc. Touch Bar Display Touchpad",
            "3",
            "a60800000000003",
        );
        fake_device(root, "event10", "Some Trackpad without MT", "1", "3");
        assert_eq!(find_trackpad(root), None);
        fake_device(
            root,
            "event8",
            "Apple Inc. Apple Internal Keyboard / Trackpad",
            "5",
            "67f800001000003",
        );
        fake_device(root, "event12", "Second Trackpad", "1", "67f800001000003");
        fs::create_dir_all(root.join("mouse0")).unwrap();
        assert_eq!(find_trackpad(root).as_deref(), Some("event8"));
        let opener = TrackpadOpener::new(root.to_path_buf(), "/dev/input".into());
        assert_eq!(opener.find(), Some(PathBuf::from("/dev/input/event8")));
    }

    #[test]
    fn discovery_handles_missing_root() {
        assert_eq!(find_trackpad(Path::new("/nonexistent/tidings")), None);
    }

    #[test]
    fn ioctl_numbers_match_the_kernel() {
        // EVIOCGABS(ABS_MT_POSITION_X) = _IOR('E', 0x75, struct input_absinfo)
        assert_eq!(evdev_ioctl(IOC_READ, 0x40 + 0x35, 24), 0x8018_4575);
        // EVIOCSCLOCKID = _IOW('E', 0xa0, int)
        assert_eq!(evdev_ioctl(IOC_WRITE, 0xa0, 4), 0x4004_45a0);
    }

    #[test]
    fn parses_input_events() {
        let mut b = [0u8; EVENT_SIZE];
        b[0..8].copy_from_slice(&5i64.to_ne_bytes());
        b[8..16].copy_from_slice(&250_000i64.to_ne_bytes());
        b[16..18].copy_from_slice(&3u16.to_ne_bytes());
        b[18..20].copy_from_slice(&0x35u16.to_ne_bytes());
        b[20..24].copy_from_slice(&(-6046i32).to_ne_bytes());
        assert_eq!(
            parse_event(&b),
            InputEvent::new(Duration::from_millis(5250), 3, 0x35, -6046)
        );
    }

    #[test]
    fn open_reports_missing_node() {
        let err = EvdevDevice::open(Path::new("/nonexistent/event0")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    /// Hardware check: `cargo test -p tidings-gesture -- --ignored`.
    #[test]
    #[ignore = "needs the real trackpad"]
    fn real_trackpad_discovery_and_range() {
        let mut opener = TrackpadOpener::default();
        let path = opener.find().expect("trackpad present");
        let (device, range) = opener.open().unwrap();
        device.use_monotonic_clock().unwrap();
        eprintln!("{}: X {}..={}", path.display(), range.min, range.max);
        assert!(range.max > range.min);
    }

    #[test]
    fn opener_without_trackpad_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let mut opener = TrackpadOpener::new(dir.path().to_path_buf(), dir.path().to_path_buf());
        assert_eq!(opener.open().unwrap_err().kind(), io::ErrorKind::NotFound);
    }
}
