<p align="center">
  <img src="docs/logo.svg" width="128" height="128" alt="">
</p>

<h1 align="center">tidings</h1>

<p align="center">A macOS-style Notification Centre for Wayland desktops, in Rust.</p>

![The Notification Centre panel beside a collapsed toast, a hovered one and an expanded one, in the default glass theme](docs/screenshots/overview.png)

- **Notification daemon.** It implements `org.freedesktop.Notifications` (actions, body markup, images, persistence), so it replaces swaync, mako or dunst.
- **Toasts** in the top-right corner: compact one-line glass pills. Hovering pauses expiry; the expand button grows a toast into the full card (long body, action buttons), and an expanded toast stays until collapsed or dismissed.
- **Inline replies.** Notifications that offer a reply (Telegram, Chrome web notifications and other apps using KDE's `inline-reply` extension) get a Reply button that opens a text field right in the toast or the panel. Enter sends; Esc discards. Web notifications also show their site after the app name.
- **Notification Centre panel.** A full-height glass panel on the right edge, with notifications grouped by app into stacks, Do Not Disturb, Clear all and per-group clear.
- **Trackpad gesture.** Put two fingers on the right edge of the trackpad and swipe left: the panel **follows your fingers**. Let go past halfway, or flick, and it opens; otherwise it springs back. Two fingers swiping right close it.
- **Themes.** Nine built in, from frosted glass to a terminal look, and every shape, colour and font setting can be changed.
- **Fits in.** Real blur behind the glass wherever the compositor offers it, with no setup on Hyprland and niri, and colours that follow your wallpaper with Noctalia or any palette tool.

| Toast | Expanded | Replying |
|---|---|---|
| ![A collapsed toast](docs/screenshots/toast-collapsed.png) | ![An expanded toast with its action buttons](docs/screenshots/toast-expanded.png) | ![An expanded toast with the reply field open](docs/screenshots/toast-reply-typing.png) |

<sub>Rendered by tidings' own drawing code. On screen, the compositor also blurs what is behind the glass.</sub>

One binary. No GTK, no Qt, no runtime scripting.

## Contents

- [Supported desktops](#supported-desktops)
- [Install](#install): from a release, or from source
- [Set up](#set-up): replace your old notification daemon, start it with your desktop, the trackpad gesture
- [Integrations](#integrations): Hyprland, niri, SwayFX, Noctalia, Waybar
- [Using it](#using-it)
- [Configuration](#configuration), [themes](#themes) and [colours](#colours)
- [Updating and uninstalling](#updating-and-uninstalling)
- [Troubleshooting](#troubleshooting)
- [Contributing](#contributing)

## Supported desktops

tidings draws its panel and toasts as Wayland *layer surfaces* (`wlr-layer-shell`, the protocol bars and launchers use), so it runs on desktops whose compositor supports that:

| desktop | status | notes |
|---|---|---|
| Hyprland | **tested**: used every day | blur with no setup |
| sway | **tested**: the automated tests run on it | no blur ([SwayFX](#swayfx) can blur) |
| niri (26.04 and newer) | should work, not tested yet | blur with no setup |
| river, labwc, Wayfire | should work, not tested yet | no blur |
| KDE Plasma, COSMIC | should work, not tested yet | turn off the desktop's own notifications first |
| GNOME (Ubuntu's default desktop) | not supported | GNOME has no layer-shell and keeps its own notification service |
| X11 sessions | not supported | Wayland only |

Support for more desktops will come over time. If you try tidings somewhere not marked tested, please [say how it went](CONTRIBUTING.md#reporting-a-problem-or-how-it-went-on-your-desktop).

On Ubuntu (22.04 or newer), tidings works with Kubuntu's Plasma, sway or another desktop from the table; the release build and the steps below work as they are.

Besides the compositor you need `libxkbcommon`, a font and a D-Bus session, which every desktop has.

## Install

### From a release

Each [release](https://github.com/ThatCatDev/tidings/releases) has a prebuilt x86-64 Linux build, which runs on Ubuntu 22.04, Debian 12, Fedora, Arch and anything newer:

```sh
tar xzf tidings-v*-x86_64-linux.tar.gz
cd tidings-v*-x86_64-linux
./install.sh
```

### From source

You need Rust 1.89 or newer (use [rustup](https://rustup.rs); Ubuntu's and Debian's packaged Rust is usually older), a C toolchain and the `libxkbcommon` and Wayland development files:

```sh
# Debian / Ubuntu
sudo apt install build-essential libxkbcommon-dev libwayland-dev
# Fedora
sudo dnf install gcc libxkbcommon-devel wayland-devel
# Arch
sudo pacman -S --needed base-devel libxkbcommon wayland
```

Then:

```sh
git clone https://github.com/ThatCatDev/tidings.git
cd tidings
./install.sh
```

### What `install.sh` does

It installs for your user only, with no root needed, and is safe to run again:

| file | what it is |
|---|---|
| `~/.local/bin/tidings` | the daemon, and `tidings ctl` to control it |
| `~/.local/share/dbus-1/services/dev.thatcat.tidings.service` | lets D-Bus start tidings when the first notification arrives |

Set `PREFIX` to install somewhere other than `~/.local`. Make sure `~/.local/bin` is on your `PATH`.

## Set up

### 1. Stop your old notification daemon

Only one program can be the notification daemon. tidings refuses to start while another one is running; it never takes over. To see which one is running now:

```sh
busctl --user status org.freedesktop.Notifications | grep -E '^(Comm|CommandLine)='
```

Remove it from your autostart and stop it: `pkill mako`, `pkill dunst` or `pkill swaync`, for example. Desktops and shells with their own notifications (KDE Plasma, COSMIC, Noctalia, ...) need theirs turned off in their settings.

### 2. Start it with your desktop

Thanks to the D-Bus file, tidings starts by itself when the first notification arrives. Start it with your session as well, so the trackpad gesture and the panel work from the moment you log in, and bind a key to open the panel:

| desktop | start with the session | open the panel with Super+A |
|---|---|---|
| Hyprland (Lua) | `hl.exec_cmd("tidings")` | `hl.bind("SUPER + A", hl.dsp.exec_cmd("tidings ctl toggle"))` |
| Hyprland (`hyprland.conf`) | `exec-once = tidings` | `bind = SUPER, A, exec, tidings ctl toggle` |
| sway | `exec tidings` | `bindsym $mod+a exec tidings ctl toggle` |
| niri | `spawn-at-startup "tidings"` | `Mod+A { spawn "tidings" "ctl" "toggle"; }` in `binds` |
| river | `riverctl spawn tidings` | `riverctl map normal Super A spawn 'tidings ctl toggle'` |
| labwc | `tidings &` in `~/.config/labwc/autostart` | a `<keybind key="W-a">` running `tidings ctl toggle` |
| anything else | run `tidings` at login | run `tidings ctl toggle` from a shortcut |

To start it now, without logging out, run `setsid -f tidings`.

### 3. The trackpad gesture (optional)

The swipe reads the trackpad directly (read-only; it never takes the device from your desktop). Allow your user to read the built-in trackpad, and nothing else, with one udev rule:

```sh
echo 'SUBSYSTEM=="input", KERNEL=="event*", ENV{ID_INPUT_TOUCHPAD}=="1", ENV{ID_INPUT_TOUCHPAD_INTEGRATION}=="internal", TAG+="uaccess"' \
  | sudo tee /etc/udev/rules.d/70-touchpad-uaccess.rules
sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=input --action=change
```

Restart tidings afterwards. Without the rule everything else works, just not the swipe. Turn the gesture off entirely with `[gestures] enabled = false` in the config.

### 4. Try it

```sh
notify-send "Hello" "from tidings"
notify-send -A open=Open -A later=Later "With buttons" "Expand the toast to see them"   # waits for a click
tidings ctl toggle
```

## Integrations

None of these are needed; each makes tidings fit in better where the tool is there.

### Blur

tidings asks the compositor to blur behind its glass through the `ext-background-effect` protocol, shaped to the rounded panel and toasts. Compositors that support it, such as Hyprland and niri 26.04 or newer, blur with no configuration. Elsewhere the glass is simply translucent; the `frosted`, `solid` and `flat` [themes](#themes) look right without blur.

A theme with `blur = false` asks for none.

#### Layer rules

Compositors can also blur by rule, matching tidings' layer namespaces: `tidings-panel` and `tidings-toast` (or `tidings-panel-noblur` and `tidings-toast-noblur` when the theme turns blur off, so rules don't match). Use them on a compositor without the protocol, or to change how the blur looks.

On Hyprland (Lua; for `hyprland.conf` see its [layer rules](https://wiki.hypr.land/configuring/core/rules/layer-rules/)):

```lua
hl.layer_rule({ name = "tidings-panel", match = { namespace = "^tidings-panel$" },
                blur = true, ignore_alpha = 0.3, xray = false })
hl.layer_rule({ name = "tidings-toast", match = { namespace = "^tidings-toast$" },
                blur = true, ignore_alpha = 0.4, xray = false })
```

With a rule, `ignore_alpha` matters: Hyprland only blurs pixels more opaque than it, so it must sit below the glass (the `clear` theme's panel is 0.4 opaque) and, for toasts, above their soft shadow (at most 0.28), or the shadow blurs into a grey halo. Blur requested through the protocol has no such catch: it covers exactly the glass.

#### SwayFX

[SwayFX](https://github.com/WillPower3309/swayfx) blurs layer surfaces by rule:

```
layer_effects "tidings-panel" blur enable
layer_effects "tidings-toast" blur enable
```

### Noctalia

[Noctalia](https://noctalia.dev) is a desktop shell. With it, tidings follows its wallpaper colours by itself: when there is no `colors.json`, tidings reads the palette Noctalia generates and keeps up as it changes (see [colours](#colours)).

Turn off Noctalia's own notification service so tidings can start, in `~/.config/noctalia/config.toml`:

```toml
[notification]
enable_daemon = false
```

A bell in Noctalia's bar that opens the panel: add `"tidings"` to one of the bar's sections (`start`, `center` or `end` under `[bar.default]`), and

```toml
[widget.tidings]
type = "custom_button"
glyph = "bell"
tooltip = "Notification Centre"

[widget.tidings.actions]
left = "exec tidings ctl toggle"
right = "exec tidings ctl dnd toggle"
```

### Waybar

A button that opens the panel (right click: Do Not Disturb). Add `"custom/tidings"` to a module list, and:

```json
"custom/tidings": {
    "format": "🔔",
    "tooltip": false,
    "on-click": "tidings ctl toggle",
    "on-click-right": "tidings ctl dnd toggle"
}
```

Any other bar works the same way: run `tidings ctl toggle` on click.

## Using it

- **Toasts** appear top right. Hover to keep one on screen, click to open the app, ⌄ to expand it and see its buttons, × to dismiss.
- **The panel** holds everything you haven't dismissed: swipe in from the trackpad's right edge, press your key binding, or run `tidings ctl toggle`. Click a stack to expand it. Esc or a click outside closes it.
- **Replying:** click **Reply** on an expanded toast or in the panel, type, and press Enter. Ctrl+V pastes; Esc throws the draft away.
  - Chrome only offers replies for web notifications with `chrome://flags/#enable-experimental-web-platform-features` turned on, and only after a restart once tidings is running (it checks what the notification daemon supports when it starts).
- **Do Not Disturb** (the moon in the panel, or `tidings ctl dnd`) keeps notifications in the panel without showing toasts, except critical ones (a low battery, say).

From the command line:

```
tidings ctl toggle | open | close     the panel
tidings ctl clear-all                  dismiss everything
tidings ctl count                      how many notifications there are
tidings ctl is-open                    whether the panel is open
tidings ctl dnd [on|off|toggle]        Do Not Disturb (no argument: show it)
```

Exit codes: 0 ok, 1 error, 2 tidings is not running.

## Configuration

Settings live in `~/.config/tidings/config.toml` (override the path with `TIDINGS_CONFIG`). Every setting is optional. See [`config.example.toml`](config.example.toml) for the full list with defaults:

- **Toasts:** timeouts for each urgency and the number shown at once.
- **Gestures:** on or off, edge zone, swipe distance and flick speed.
- **Animation:** on or off, and speed.
- **Theme:** which theme, overrides of any of its settings, and the colours file path.

```sh
mkdir -p ~/.config/tidings
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
| `solid` | Opaque surfaces in your colour scheme's colours, rounded but not pills. |
| `flat` | Minimal: opaque, small corners, hairline borders and no shadow. |
| `paper` | Light, warm off-white paper with a terracotta accent and soft shadows. |
| `nord` | Glass in the Nord palette: polar night surfaces, frost-blue accents. |
| `mocha` | Catppuccin Mocha: deep base, mauve accents, soft lavender rims. |
| `terminal` | Gruvbox and a monospace font: square corners, bright yellow borders, no shadow. |

| | | |
|---|---|---|
| ![glass](docs/screenshots/themes/glass.png)<br>`glass` | ![frosted](docs/screenshots/themes/frosted.png)<br>`frosted` | ![clear](docs/screenshots/themes/clear.png)<br>`clear` |
| ![solid](docs/screenshots/themes/solid.png)<br>`solid` | ![flat](docs/screenshots/themes/flat.png)<br>`flat` | ![paper](docs/screenshots/themes/paper.png)<br>`paper` |
| ![nord](docs/screenshots/themes/nord.png)<br>`nord` | ![mocha](docs/screenshots/themes/mocha.png)<br>`mocha` | ![terminal](docs/screenshots/themes/terminal.png)<br>`terminal` |

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
mkdir -p ~/.config/tidings/themes
tidings theme show glass > ~/.config/tidings/themes/mine.toml
tidings theme check mine                  # or a path to a .toml file
```

Theme files are checked like the config. A broken theme is logged and `glass` is used instead.

Blur, and the layer namespaces for compositor rules, are under [Integrations](#blur).

### Colours

A theme's colours are `#rrggbb` or a slot of the colour scheme (`surface`, `surface_variant`, `on_surface`, `on_surface_variant`, `primary`, `on_primary`, `error`, `outline`, or `tint`). The slots come from the colour scheme below, so a theme can follow your wallpaper; a theme's `[palette]` replaces slots outright.

The colour scheme comes from the first of these that exists:

1. **`~/.config/tidings/colors.json`**, which any palette tool (matugen, wallust, pywal templates, a script of your own) can write.
2. **Noctalia's palette**, read directly when Noctalia is installed (see [Noctalia](#noctalia)).
3. tidings' own dark colours.

Set `theme.colors` in the config to a file to use that instead, or to `"noctalia"` to always use Noctalia's palette; `TIDINGS_COLORS` overrides both. tidings checks for changes every second and recolours live; the log's `colours:` line says which source it chose.

`colors.json` looks like this. Every key is optional; missing ones keep the default:

```json
{
  "mode": "dark",
  "surface": "#18181d",
  "surfaceVariant": "#22232a",
  "onSurface": "#e4e1e7",
  "onSurfaceVariant": "#a3abbd",
  "primary": "#a8c7fa",
  "onPrimary": "#0b0e13",
  "error": "#d3453b",
  "onError": "#ffffff",
  "outline": "#a0b2d2",
  "surfaceAlpha": 0.62
}
```

`mode` is `dark` or `light`, `surfaceAlpha` (0 to 1) is how opaque the glass is when the theme does not say. Write the file somewhere else and rename it into place, so tidings never reads half of it.

## Updating and uninstalling

To update, install again the same way (a newer release, or `git pull && ./install.sh`), then restart tidings:

```sh
pkill -x tidings; setsid -f tidings
```

To uninstall, run `./install.sh --uninstall`. Your settings in `~/.config/tidings` are left in place; delete that folder too if you want. Then set up your previous notification daemon again.

## Troubleshooting

Run tidings in a terminal with more logging to see what it is doing:

```sh
pkill -x tidings; TIDINGS_LOG=debug tidings
```

- **"org.freedesktop.Notifications is already owned by another notification daemon"**: another daemon is running. See [Set up, step 1](#1-stop-your-old-notification-daemon).
- **"this compositor does not support wlr-layer-shell"**: tidings can't run on this desktop (GNOME, for one). See [Supported desktops](#supported-desktops).
- **No blur**: the compositor may not blur on request (see [Blur](#blur)); the log says `compositor blur on request: no` then. Check the theme has `blur = true`. Screenshots don't show compositor blur, so look at the screen itself.
- **The swipe does nothing**: the log says whether a trackpad was found. Without the udev rule ([step 3](#3-the-trackpad-gesture-optional)) it can't read one.
- **Settings don't apply**: run `tidings config check` (and `tidings theme check`), then restart tidings. Settings are read at startup; only the colours are followed live.
- **The wrong colours**: the log's `colours:` line says where they come from; see [colours](#colours).
- **Chrome shows Reply as a plain button**: see the Chrome note under [Using it](#using-it).

## Contributing

Bug reports, reports from desktops not tested yet, and code are all welcome. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
