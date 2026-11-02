# tidings: design

A macOS-style Notification Centre for Hyprland, written in Rust as one binary.

- **Notification daemon:** owns `org.freedesktop.Notifications`, replacing swaync/mako/dunst.
- **Toasts:** popups at the top-right.
- **Panel:** a full-height glass panel on the right edge.
- **Gesture:** a **two-finger swipe from the right edge of the trackpad** pulls the panel out, and the panel **follows the fingers** like macOS. Releasing past halfway, or with a flick, opens it; otherwise it springs back. A two-finger swipe right while it is open closes it the same way.

Target machine: MacBookPro16,2 (T2), CachyOS, Hyprland 0.57, single display eDP-1 2560x1600 at fractional scale 1.6. Rust 1.99, edition 2024.

## Architecture

```
crates/core      tidings-core     pure domain: CONTRACT types + store, policies, view building
crates/gesture   tidings-gesture  evdev MT decoder + gesture recognizer (pure) + device reader
crates/ui        tidings-ui       Wayland layer-shell UI thread (rendering, input, animation)
crates/tidings   tidings          binary: D-Bus server, app actor, IPC socket, CLI, wiring
```

Dependency direction: `tidings -> {core, gesture, ui}`, `gesture -> core`, `ui -> core`. `core` depends on nothing platform-specific.

**CONTRACT modules** in `tidings-core` (`model`, `gesture`, `theme`, `view`, `ui`) are fixed interfaces between the parallel workstreams. Don't change them without coordination. Additive changes (new helper methods, `impl` blocks, new modules beside them) are fine.

### Runtime model: single-owner actor

- One **App actor** (in `tidings`) owns all state. Everything else sends it messages through one channel (`AppEvent`):
  - D-Bus method calls (`Notify`, `CloseNotification`, ...): oneshot reply channels
  - Gesture events: from the gesture reader thread
  - `UiEvent`s: from the UI thread
  - IPC requests: unix socket, with oneshot replies
  - Timer ticks: toast expiry and relative-time refresh
  - Theme file changes
- After handling each event, the actor rebuilds the `ViewModel` if state changed (dirty flag) and sends `UiUpdate::View`. Gesture events map to `UiUpdate::Panel(Track|AnimateTo)`.
- The UI is a passive renderer. It never mutates domain state; it reports interactions as `UiEvent`.
- Time is injected (a `Clock` trait), so expiry, time labels and gesture velocity are deterministic in tests.

### Quality bar

- Each module has one responsibility. Traits sit at every I/O seam (clock, input device, D-Bus emitter, UI handle) so logic is unit-testable without hardware or a compositor.
- Errors use `thiserror` in libraries and `anyhow` only in the binary's `main`/wiring. No `unwrap()`/`expect()` outside tests, except where an invariant is documented.
- Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` (workspace lints: clippy pedantic) and `cargo test` before reporting done.
- Doc comments on public items. No dead code, no commented-out code.
- Dependencies: pick mainstream, maintained crates; note why in Cargo.toml comments where it isn't obvious.

## Behaviour spec

### Notifications (D-Bus)
- Spec version 1.2. Capabilities: `actions`, `body`, `body-hyperlinks`, `body-markup`, `icon-static`, `persistence`.
- `Notify` with `replaces_id` replaces in place: same id, card stays put, the toast re-shows.
- Hints: `urgency`, `image-data`/`image_data`/`icon_data` (iiibiiay → straight RGBA), `image-path`, `desktop-entry`, `transient`, `resident` (don't remove on action).
- Body markup is reduced to plain text (strip tags, decode entities).
- Emit `ActionInvoked` and `NotificationClosed` signals with correct reason codes.
- `CloseNotification` → reason 3.

### Toasts
- Top-right below the bar: top margin 52 logical px, right margin 12, width 360. At most 4 visible, newest on top.
- Compact pill: 58 logical px tall (radius 29), round 40 px icon, a bold one-line summary over one line of body, then the time and a round 28 px expand (⌄) button (with a dot when the notification has actions). Hover tints the glass and shows a "×" over the icon.
- Expanded: the full card (app name + time, summary, body up to 6 lines, action buttons) with the icon level with the app-name row and a collapse (⌃) button in the top-right corner. The stack re-flows when a toast grows or shrinks. The UI sends `SetToastExpanded`.
- Expiry: low 4 s, normal 6 s, critical never (all configurable). A positive client timeout wins; 0 means never. Hover or expansion pauses expiry; it resumes with the remaining time once the toast is neither. A replaced notification keeps its expanded state. Expired toasts stay in the panel history (unless `transient`).
- Glass: far more transparent than the panel (alpha 0.46 at rest), with a 1 px light rim and a soft shadow whose alpha stays below 0.3, so a Hyprland `ignore_alpha` of 0.4 blurs the glass but not the shadow.
- Click on the body → `default` action (if any), then dismiss. Close button and action buttons.
- Do Not Disturb suppresses toasts, except critical ones.

### Panel
- Right-edge layer surface, layer Top, namespace `tidings-panel`, width 380 logical, margins: top 52, right/bottom 8. Translucent glass background (alpha from the theme, default 0.62); Hyprland supplies the blur via a layer rule.
- Header: "Notifications", a Do Not Disturb toggle, "Clear all".
- Groups by app, newest group first. A multi-card group shows collapsed as a stacked card (up to 2 offset "sheets" peeking below), expands on click, and has a per-group "×" clear.
- Expanding or collapsing a group morphs over ~320 ms (OutCubic, reversible midway): the cards behind slide out from under the top card and fade in, the header fades in, the sheets fade out, and the groups below glide. Collapsing plays it backwards.
- Card: icon (centred against the text), app name, relative time, bold summary, body (2 lines when collapsed), action buttons, hover "×".
- A missing icon becomes a letter tile: the app's first letter, dark on a pastel picked by a stable hash of the app name.
- Empty state: "No Notifications".
- Closing: outside click (a transparent full-output catcher surface `tidings-catcher` mapped while the panel is open), Esc, or the gesture.
- Slide: openness 0..1 translates the panel content horizontally. Content opacity follows openness. The surface maps when openness > 0 and unmaps on settle-closed. The input region covers only the visible part.
- `AnimateTo`: critically damped spring or OutCubic over ~220 ms scaled by distance. It is frame-callback driven (no busy loops) and interruptible by `Track`.

### Gesture recognition (`tidings-gesture`)
- Device: /sys/class/input/event* whose name contains "Trackpad" and whose properties have INPUT_PROP_POINTER. Read-only, never grabbed. Access comes from the udev uaccess rule `/etc/udev/rules.d/70-touchpad-uaccess.rules` (already installed).
- Decode MT protocol B (`ABS_MT_SLOT`, `ABS_MT_TRACKING_ID`, `ABS_MT_POSITION_X/Y`, `SYN_REPORT`) into frames, then recognise on frames.
- Edge zone: the rightmost 10% of pad width. Both fingers land within 250 ms. Start threshold: 1.5% of width. Dominance: |dx| ≥ 1.5|dy|. A third finger cancels. One gesture per touch sequence.
- `Close` gestures are recognised only while the app says the panel is open (`set_panel_open(bool)`).
- Constants live in a `RecognizerConfig` with defaults.
- The reader thread reconnects with backoff if the device disappears.

### Gesture → panel mapping (in core, pure)
- `openness = clamp(progress / 0.30, 0, 1)` for Open, and `1 - that` for Close.
- On End: complete if velocity > 0.6, abort if < -0.6, else complete if fraction > 0.5.

### IPC / CLI
- The `tidings` binary with no args (or `tidings daemon`) runs the daemon.
- `tidings ctl <cmd>` talks to `$XDG_RUNTIME_DIR/tidings.sock` (line-based request/response). Commands: `toggle | open | close | clear-all | dnd [on|off|toggle] | count | is-open | gesture <begin open|begin close|update P|end P V|cancel>`. The gesture command is for tests: it injects GestureEvents.
- Exit codes: 0 ok, 1 error, 2 daemon not running.

### Config file
- `~/.config/tidings/config.toml` (override: `TIDINGS_CONFIG`). Read at startup. Schema, parsing and validation live in `tidings_core::config`; the file system side is in `tidings::config_file`.
- **Validation runs in two stages.**
  1. serde reads the TOML into raw structs that mirror the file. `deny_unknown_fields` makes a typo an error, and serde enforces the TOML types.
  2. Each value is validated (durations parse, numbers in range) into the typed `Config`. Every value error is collected, not just the first. Values are `toml::Spanned`, so each error carries a line and column.
- An invalid file is logged and the defaults apply: a config mistake must never cost the user their notifications. `tidings config check` exits 1 on errors.
- `config.example.toml` is the template (`tidings config default`); a test pins it to `Config::default()`, so docs and defaults cannot drift.
- Settings reach their owners at startup:
  - toasts → `ToastPolicy`
  - gestures → `RecognizerConfig.edge` and `panel::Swipe`
  - animation → `tidings_ui::anim::Timing`, which scales every duration and the spring by `speed`, or makes everything instant
  - theme.colors → the colours watcher (`TIDINGS_COLORS` still wins)

### Theme
- `~/.config/tidings/colors.json` (override: `TIDINGS_COLORS`), watched for changes (inotify via `notify` crate or polling mtime ~1 s). Invalid or missing file → built-in dark defaults.
- `scripts/noctalia-colors.sh` (POSIX shell, no python/node) converts Noctalia's `~/.config/hypr/noctalia.lua` + `noctalia msg theme-mode-get` into colors.json.

## Tests

- Unit tests per module (core policies/store/view building; recognizer on synthetic frames; MT decoder on raw event sequences; D-Bus hint parsing; IPC parsing).
- Integration tests in `crates/tidings/tests/`. Start the daemon logic against a private bus (`dbus-daemon --session --print-address --fork`, or `dbus-run-session`) with a null UI handle, then call Notify/Close and assert signals, IPC responses and toast expiry with a fake clock.
- UI: layout and hit-testing are pure functions of (ViewModel, size, scale), unit-tested. Rendering to an offscreen pixmap gets golden-free sanity checks (non-empty, correct size). Run a live smoke test (`tests/smoke.sh`) on the real compositor under a private D-Bus session. It must not fight the session's real notification daemon.
- **No Python or Node anywhere.** Shell scripts are fine.
