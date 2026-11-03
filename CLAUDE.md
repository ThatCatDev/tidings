# tidings: notes for Claude

Read [CONTRIBUTING.md](CONTRIBUTING.md) first: layout, rules, checks, tests and the release process are there, and they all apply. [DESIGN.md](DESIGN.md) is the behaviour spec; [README.md](README.md) is for users only, so keep developer material out of it.

Extra notes for working here:

- **The bar is high:** design patterns, small single-purpose modules, tests that check behaviour. Keep every file at 80%+ line coverage (aim 85%+) and run `scripts/coverage.sh` before calling something done.
- **No compositor or shell is assumed.** Anything specific to Hyprland, Noctalia or another desktop is an integration used when present, never a requirement. Keep the README's supported-desktops table honest.
- **Coverage with a system LLVM** (Arch/CachyOS): `LLVM_COV=/usr/bin/llvm-cov LLVM_PROFDATA=/usr/bin/llvm-profdata scripts/coverage.sh`.
- **UI tests wait on the UI, not on sleeps.** The harness (`crates/ui/tests/harness`) captures the UI's own log lines and waits for things like "the panel is drawn" or "the reply field has the keyboard"; `TIDINGS_TEST_LOG=1` prints them. The tests take turns (parallel software-rendering compositors make timing flaky), and sway's virtual keyboards don't repeat keys.
- **Pull request titles** are the squash commit: start them with `feat:` or `fix:` when the change should be released.

Handy tools:

```sh
cargo run -p tidings-ui --example demo --release -- themes DIR [SCALE]  # every theme to PNGs
cargo run -p tidings-ui --example demo --release -- states DIR          # a toast's states
cargo run -p tidings --example web_notify                               # Chrome web-notification test page, localhost:8317
```

README images: `themes docs/screenshots/themes 1` (then drop the `theme-` prefix); `overview.png` is `theme-glass.png` from `themes DIR 1.5`; `toast-*.png` come from `states DIR`.
