# tidings

A macOS-style Notification Centre for Hyprland, in Rust.

- **Notification daemon.** It implements `org.freedesktop.Notifications` (spec 1.2: actions, body markup, images, persistence), so it replaces swaync, mako or dunst.
- **Toasts** in the top-right corner: compact one-line glass pills. Hovering pauses expiry; the expand button grows a toast into the full card (long body, action buttons), and an expanded toast stays until collapsed or dismissed.
- **Inline replies.** Notifications that offer a reply (Telegram and other apps using KDE's `inline-reply` extension, and Chrome web notifications once `chrome://flags/#enable-experimental-web-platform-features` is on) get a Reply button that opens a text field right in the toast or panel card. Enter sends; Esc discards. Web notifications also show their site after the app name.
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
-- toasts are lighter glass (alpha >= 0.46) with a soft shadow (alpha < 0.3):
-- the threshold sits between, so the glass is blurred and the shadow is not
hl.layer_rule({ name = "tidings-toast", match = { namespace = "^tidings-toast$" },
                blur = true, ignore_alpha = 0.4, xray = false })
```

### Configuration

Settings live in `~/.config/tidings/config.toml` (override the path with `TIDINGS_CONFIG`). Every setting is optional. See [`config.example.toml`](config.example.toml) for the full list with defaults:

- **Toasts:** timeouts for each urgency and the number shown at once.
- **Gestures:** on or off, edge zone, swipe distance and flick speed.
- **Animation:** on or off, and speed.
- **Theme:** which theme, overrides of any of its settings, and the colours file path.

```sh
tidings config default > ~/.config/tidings/config.toml   # start from the template
tidings config check                                      # validate it
tidings config path                                       # where tidings looks
```

The file is type-checked. Unknown keys, wrong types, malformed durations (`"4s"`, `"250ms"`) and out-of-range numbers are errors, reported as `file:line:column: key: message`. If the file is invalid, the daemon logs the errors and starts with the defaults. Changes apply when tidings restarts.

### Themes

A theme sets the look: corner radius, border width, colour and opacity, transparency, blur, shadow, font, text size and line height. Nine are built in:

| theme | look |
|---|---|
| `glass` (default) | Frosted, translucent and pill-shaped, like the macOS Notification Centre. |
| `frosted` | Thicker, milkier glass: more opaque, softer rims, rounder corners. |
| `clear` | Barely-there glass held together by crisp light rims. |
| `solid` | Opaque surfaces in your Noctalia colours, rounded but not pills. |
| `flat` | Minimal: opaque, small corners, hairline borders and no shadow. |
| `paper` | Light, warm off-white paper with a terracotta accent and soft shadows. |
| `nord` | Glass in the Nord palette: polar night surfaces, frost-blue accents. |
| `mocha` | Catppuccin Mocha: deep base, mauve accents, soft lavender rims. |
| `terminal` | Gruvbox and a monospace font: square corners, bright yellow borders, no shadow. |

Pick one in `config.toml`, and change any of its settings right there:

```toml
[theme]
name = "nord"

[theme.panel]
radius = 12
border_width = 2

[theme.font]
line_height = 1.4
```

To make your own, put a file in `~/.config/tidings/themes/` (`tidings theme path`). A file with a built-in's name replaces it. [`themes/glass.toml`](themes/glass.toml) documents every setting at its default; a theme only needs the keys it changes.

```sh
tidings theme list                        # * marks the one in use
tidings theme show glass > ~/.config/tidings/themes/mine.toml
tidings theme check mine                  # or a path to a .toml file
```

Theme files are checked like the config. A broken theme is logged and `glass` is used instead.

Colours are `#rrggbb` or a slot of the colour scheme (`surface`, `surface_variant`, `on_surface`, `on_surface_variant`, `primary`, `on_primary`, `error`, `outline`, or `tint`), and slots follow Noctalia. A theme's `[palette]` replaces colours from `colors.json` outright.

Blur is drawn by Hyprland. With `blur = true` the surfaces use the `tidings-panel` and `tidings-toast` layer namespaces, and with `blur = false` they use `tidings-panel-noblur` and `tidings-toast-noblur`. Hyprland skips pixels below a layer rule's `ignore_alpha`, so keep blurred opacities above it and the toast shadow below it.

### Colours

The colours come from `~/.config/tidings/colors.json`. Override the path with `TIDINGS_COLORS` or the config's `theme.colors`. tidings watches the file and applies changes live. With [Noctalia](https://noctalia.dev), have its `colors_changed` hook run `tidings-noctalia-colors`, and tidings follows your wallpaper palette.

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
                                # (the UI end-to-end tests need sway, wtype, wl-clipboard and grim;
                                #  they run a private headless sway and skip without it)
sh tests/colors.sh              # the Noctalia colours script
scripts/coverage.sh             # coverage report + the 80%-per-file gate (needs cargo-llvm-cov, jq)
cargo run -p tidings-ui --example demo --release        # UI demo on the live compositor
cargo run -p tidings-ui --example demo --release -- themes DIR   # render every theme to PNGs
cargo run -p tidings --example web_notify     # test page for Chrome web notifications (replies, buttons) on localhost:8317
```

## Releases

Every pull request gets a comment with its test coverage, compared with the branch it targets (the `coverage` workflow; it updates the same comment on each push). The check fails if any source file drops under 80% of lines covered; the aim is 85% or more everywhere.

Releases are fully automatic (semantic release via [cocogitto](https://docs.cocogitto.io)). Commit messages must follow [Conventional Commits](https://www.conventionalcommits.org), and CI rejects any that don't.

| commit | release |
|---|---|
| `fix: ...`, `perf: ...` | patch (0.1.0 → 0.1.1) |
| `feat: ...` | minor (0.1.0 → 0.2.0) |
| `feat!: ...` or a `BREAKING CHANGE:` footer | major |
| `docs:`, `refactor:`, `test:`, `ci:`, `build:`, `chore:`, `style:` | no release |

After CI passes on `main`, the release workflow does the rest:
1. Bumps the workspace version in `Cargo.toml` and `Cargo.lock`.
2. Updates `CHANGELOG.md`.
3. Commits `chore(version): vX.Y.Z` and tags it.
4. Publishes a GitHub release with the changelog and a `tidings-vX.Y.Z-x86_64-linux.tar.gz` binary.

Locally, `cog check` validates messages and `cog bump --auto --dry-run` shows the next version.

## License

MIT
