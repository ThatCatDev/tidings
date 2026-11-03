# Contributing to tidings

Thanks for helping. Reports from desktops we haven't tested yet are just as welcome as code.

## Reporting a problem, or how it went on your desktop

[Open an issue](https://github.com/ThatCatDev/tidings/issues) with:

- your desktop and its version (`hyprctl version`, `sway --version`, `niri --version`, ...) and your distribution;
- the tidings version (`tidings --version`);
- what you did, what you expected, and what happened;
- the log, from running tidings in a terminal with more logging:

  ```sh
  pkill -x tidings; TIDINGS_LOG=debug tidings
  ```

  The first lines say which colours, theme and blur tidings found.

If tidings works on a desktop the README lists as "not tested yet", an issue saying so helps too: it moves that desktop up to tested.

## Building

You need Rust 1.89 or newer ([rustup](https://rustup.rs)), a C toolchain and the `libxkbcommon` and Wayland development files (the README lists the packages for each distribution). Then:

```sh
git clone https://github.com/ThatCatDev/tidings.git
cd tidings
cargo build
cargo run -p tidings-ui --example demo --release   # the UI on your desktop, with sample notifications
```

To try your build as your real notification daemon, `./install.sh` and restart tidings.

## How the code is organised

```
crates/core      domain model, store, policies, view building   (pure, no I/O)
crates/gesture   evdev multitouch decoder + gesture recogniser   (pure) + device reader
crates/ui        Wayland layer-shell UI: sctk + tiny-skia + cosmic-text
crates/tidings   the binary: D-Bus server (zbus), app actor, IPC socket, CLI
```

- **One actor owns all state.** D-Bus calls, gesture events, UI interactions, IPC requests and timers are messages on one channel.
- **The UI only renders.** It gets a `ViewModel` snapshot and reports what the user did as `UiEvent`s.
- **I/O sits behind traits** (clock, input device, UI handle, signal sink), so the logic is tested without hardware, a compositor or the session bus.
- **No desktop is assumed.** Hyprland, Noctalia and the rest are integrations tidings uses when they are there, never requirements.

[DESIGN.md](DESIGN.md) describes the behaviour in detail.

## What a change needs

- **Rust only.** No Python or Node in the repository; small POSIX shell scripts are fine.
- **Tests** for what it changes, checking behaviour rather than just running lines. Every source file must keep at least **80%** line coverage (CI fails below that); aim for **85%** or more.
- **Clean checks:**

  ```sh
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets -- -D warnings   # clippy::pedantic is on
  cargo test --workspace
  scripts/coverage.sh                                     # needs cargo-llvm-cov and jq
  ```

- **Code style:** small modules with one job each, doc comments on public items, no `unwrap()` or `expect()` outside tests, no dead or commented-out code.

### Tests

- Tests never touch your real session bus, compositor, `~/.config` or `/dev/input`. The daemon's integration tests start a private `dbus-daemon`.
- The UI's end-to-end tests run the real UI in a private, headless sway and drive it with a virtual mouse and keyboard. They need `sway`, `swaymsg`, `wtype`, `wl-clipboard` and `grim`, and are skipped without them (CI has them all).
- For Wayland features sway lacks (blur on request, for one), `crates/ui/tests/fake` is a tiny fake compositor that records what the UI asks for. Extend it when the UI starts using another one.

## Commits and pull requests

- Work on a branch and open a pull request against `main`. Each pull request gets a comment with its test coverage compared with `main`.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org) (`fix: ...`, `feat: ...`, `docs: ...`); CI checks them.
- Pull requests are squash-merged, so **the pull request's title becomes the commit**, and it decides the release:

  | title starts with | release |
  |---|---|
  | `fix:`, `perf:` | patch |
  | `feat:` | minor |
  | `feat!:`, or a `BREAKING CHANGE:` footer | major |
  | `docs:`, `refactor:`, `test:`, `ci:`, `build:`, `chore:`, `style:` | none |

- Releases are automatic: once CI passes on `main`, a workflow bumps the version, updates the changelog, tags it and publishes a prebuilt Linux build (made on Ubuntu 22.04, so it runs there and on anything newer).

## Licence

tidings is [MIT licensed](LICENSE). By contributing, you agree that your contributions are licensed the same way.
