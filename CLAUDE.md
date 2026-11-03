# tidings: notes for working on the code

User-facing docs are in [README.md](README.md); the behaviour spec is [DESIGN.md](DESIGN.md). This file is for whoever changes the code (people or Claude).

## Layout

```
crates/core      domain model, store, policies, view building   (pure, no I/O)
crates/gesture   evdev multitouch decoder + gesture recogniser   (pure) + device reader
crates/ui        Wayland layer-shell UI: sctk + tiny-skia + cosmic-text
crates/tidings   the binary: D-Bus server (zbus), app actor, IPC socket, CLI
```

- **A single actor owns all state.** D-Bus calls, gesture events, UI interactions, IPC requests and timers are messages on one channel.
- **The UI is a passive renderer.** It receives a `ViewModel` snapshot and reports interactions back as `UiEvent`s.
- **I/O sits behind traits.** Clock, input device, UI handle and signal sink are traits, so logic is testable without hardware, a compositor or the session bus.
- **No compositor or shell is assumed.** Hyprland, Noctalia and others are integrations used when present (blur via `ext-background-effect-v1`, colours via `tidings::colors::ColorSource`), never requirements.

## Rules

- Rust only. No Python or Node in the repo; POSIX shell is fine for small scripts.
- Clippy pedantic with `-D warnings` (workspace lints). No `unwrap()`/`expect()` outside tests.
- Design patterns and small, single-purpose modules; doc comments on public items; no dead or commented-out code.
- Every source file keeps at least **80%** line coverage (CI fails below; the aim is **85%+**). New code comes with tests that check behaviour, not just lines.
- Tests are hermetic: never the real session bus, the live compositor, the user's `~/.config` or `/dev/input`.

## Commands

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/coverage.sh        # coverage report + the 80%-per-file gate (cargo-llvm-cov, jq)
```

- With a system LLVM (Arch/CachyOS), prefix coverage with `LLVM_COV=/usr/bin/llvm-cov LLVM_PROFDATA=/usr/bin/llvm-profdata`.
- The UI's end-to-end tests (`crates/ui/tests/wayland.rs`) run the real UI in a private headless sway, driven by a virtual pointer, `wtype` and `wl-copy`; they need `sway`, `swaymsg`, `wtype`, `wl-clipboard` and `grim`, and skip without them unless `TIDINGS_REQUIRE_COMPOSITOR` is set (CI sets it). They take turns, since parallel software-rendering compositors make timing flaky. Sway's virtual keyboards don't repeat keys.
- `crates/ui/tests/blur.rs` runs the UI against a tiny fake compositor (`tests/fake`, on `wayland-server`) for protocols sway lacks, such as blur on request; it records what the UI asks for. Extend it when the UI uses another protocol CI's sway doesn't have.
- The UI tests wait on the UI's own log lines (the harness captures them; `TIDINGS_TEST_LOG=1` prints them) for things like "the panel is drawn" or "the reply field has the keyboard", instead of sleeping.
- The integration tests in `crates/tidings/tests` start a private `dbus-daemon`.

Other tools:

```sh
cargo run -p tidings-ui --example demo --release                       # the UI on the live compositor
cargo run -p tidings-ui --example demo --release -- themes DIR [SCALE]  # every theme to PNGs
cargo run -p tidings-ui --example demo --release -- states DIR          # a toast's states
cargo run -p tidings --example web_notify                               # Chrome web-notification test page, localhost:8317
```

README images: `themes docs/screenshots/themes 1` (then drop the `theme-` prefix), `overview.png` is `theme-glass.png` from `themes DIR 1.5`, and `toast-*.png` come from `states DIR`.

## Releases

Fully automatic via [cocogitto](https://docs.cocogitto.io): commit messages must be [Conventional Commits](https://www.conventionalcommits.org) (CI checks).

| commit | release |
|---|---|
| `fix:`, `perf:` | patch |
| `feat:` | minor |
| `feat!:` or a `BREAKING CHANGE:` footer | major |
| `docs:`, `refactor:`, `test:`, `ci:`, `build:`, `chore:`, `style:` | none |

Pull requests are squash-merged, so **the PR title is the commit message** and decides the release.

After CI passes on `main`, `release.yml` bumps the version in `Cargo.toml`/`Cargo.lock`, updates `CHANGELOG.md`, commits `chore(version): vX.Y.Z`, tags it, and publishes a release with a `tidings-vX.Y.Z-x86_64-linux.tar.gz` (binary, `install.sh`, `config.example.toml`, README, licence), built on Ubuntu 22.04 so it runs there and on anything newer.

Every pull request gets a coverage comment (`coverage.yml`) compared with its base branch.
