# tidings

A macOS-style Notification Centre for Hyprland, in Rust.

- **Notification daemon.** It implements `org.freedesktop.Notifications` (spec 1.2: actions, body markup, images, persistence), so it replaces swaync, mako or dunst.
- **Toasts** in the top-right corner. Hovering pauses expiry, and they support action buttons and critical urgency.
- **Notification Centre panel.** A full-height glass panel on the right edge. Notifications are grouped by app into macOS-style stacks, with Do Not Disturb, Clear all and per-group clear.
- **Trackpad gesture.** Put two fingers on the right edge of the trackpad and swipe left: the panel **follows your fingers**. Let go past halfway, or flick, and it opens; otherwise it springs back. Two fingers swiping right close it the same way.

One binary. No GTK, no Qt, no runtime scripting.

## Architecture

```
crates/core      domain model, store, policies, view building   (pure, no I/O)
crates/gesture   evdev multitouch decoder + gesture recogniser   (pure) + device reader
crates/ui        Wayland layer-shell UI: sctk + tiny-skia + cosmic-text
crates/tidings   the binary: D-Bus server (zbus), app actor, IPC socket, CLI
```

- **A single actor owns all state.** D-Bus calls, gesture events, UI interactions, IPC requests and timers are messages on one channel.
- **The UI is a passive renderer.** It receives a `ViewModel` snapshot and reports interactions back as events.
- **I/O sits behind traits.** The clock, input device, UI handle and signal sink are all traits, so everything is testable without hardware, a compositor or the session bus.

See [DESIGN.md](DESIGN.md) for the full spec.

## Requirements

- Hyprland, or another compositor with `wlr-layer-shell`, `fractional-scale-v1`, `viewporter` and `cursor-shape-v1`.
- Rust 1.89 or newer to build.
- **Trackpad access.** The gesture reads the touchpad's evdev node (read-only; it never grabs the device). Grant the logged-in user access to the internal touchpad *only*:

  ```sh
  echo 'SUBSYSTEM=="input", KERNEL=="event*", ENV{ID_INPUT_TOUCHPAD}=="1", ENV{ID_INPUT_TOUCHPAD_INTEGRATION}=="internal", TAG+="uaccess"' \
    | sudo tee /etc/udev/rules.d/70-touchpad-uaccess.rules
  sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=input --action=change
  ```

  Without this rule, tidings works normally but the gesture is disabled.

## Install

```sh
./install.sh   # ~/.local/bin/tidings, the colours hook, a D-Bus activation file
```

Stop any other notification daemon. tidings refuses to start if `org.freedesktop.Notifications` is already owned; it never replaces another daemon.

### Hyprland config (Lua)

```lua
-- autostart
hl.exec_cmd("tidings")

-- open/close the panel from the keyboard
hl.bind("SUPER + A", hl.dsp.exec_cmd("tidings ctl toggle"))

-- glass: blur behind the panel and the toasts
hl.layer_rule({ name = "tidings-panel", match = { namespace = "^tidings-panel$" },
                blur = true, ignore_alpha = 0.5, xray = false })
hl.layer_rule({ name = "tidings-toast", match = { namespace = "^tidings-toast$" },
                blur = true, ignore_alpha = 0.5, xray = false })
```

### Theme

The colours come from `~/.config/tidings/colors.json`. Override the path with `TIDINGS_COLORS`. tidings watches the file and applies changes live. With [Noctalia](https://noctalia.dev), have its `colors_changed` hook run `tidings-noctalia-colors`, and tidings follows your wallpaper palette.

## Control

```
tidings ctl toggle | open | close | clear-all | count | is-open
tidings ctl dnd [on|off|toggle]
tidings ctl gesture begin open|close / update P / end P V / cancel   # testing
```

Exit codes: 0 ok, 1 error, 2 daemon not running. Logging: `TIDINGS_LOG=debug tidings`.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings   # clippy::pedantic
cargo test --workspace          # unit + integration (private dbus-daemon, never the session bus)
sh tests/colors.sh              # the Noctalia colours script
cargo run -p tidings-ui --example demo --release        # UI demo on the live compositor
```

## License

MIT
