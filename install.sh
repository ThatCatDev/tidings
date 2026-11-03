#!/bin/sh
# Install tidings for the current user (idempotent, no root needed), or
# remove it with --uninstall.
#
# Run it from an unpacked release (it installs the prebuilt binary next to
# it) or from a source checkout (it builds with cargo). It installs:
#
#   ~/.local/bin/tidings                    the daemon + `tidings ctl`
#   ~/.local/bin/tidings-noctalia-colors    Noctalia palette -> colors.json
#   ~/.local/share/dbus-1/services/...      D-Bus activation for
#                                           org.freedesktop.Notifications, so a
#                                           notification sent before the
#                                           compositor autostart runs starts
#                                           tidings instead of another daemon
#                                           (user service files win over
#                                           /usr/share ones).
#
# PREFIX (default ~/.local) and XDG_DATA_HOME change where things go. The
# trackpad gesture also needs read access to the touchpad; see README.md
# ("Trackpad gesture") for the one-time udev rule.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PREFIX=${PREFIX:-"$HOME/.local"}
DATA_HOME=${XDG_DATA_HOME:-"$HOME/.local/share"}
BIN="$PREFIX/bin"
SERVICE="$DATA_HOME/dbus-1/services/dev.thatcat.tidings.service"

usage() {
    echo "usage: $0 [--uninstall]"
}

uninstall() {
    rm -f "$BIN/tidings" "$BIN/tidings-noctalia-colors" "$SERVICE"
    # Drop cargo's record of the install too, if it made one.
    if command -v cargo >/dev/null 2>&1; then
        cargo uninstall --root "$PREFIX" tidings >/dev/null 2>&1 || true
    fi
    echo "removed tidings from $BIN (your settings in ~/.config/tidings are kept)"
}

case "${1:-}" in
    "") ;;
    --uninstall)
        uninstall
        exit 0
        ;;
    -h | --help)
        usage
        exit 0
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac

if [ -x "$ROOT/tidings" ]; then
    # An unpacked release: the binary is right here.
    install -Dm755 "$ROOT/tidings" "$BIN/tidings"
    colors="$ROOT/noctalia-colors.sh"
else
    if ! command -v cargo >/dev/null 2>&1; then
        echo "install.sh: cargo not found. Install Rust (https://rustup.rs)," >&2
        echo "or use a prebuilt release instead." >&2
        exit 1
    fi
    cargo install --locked --path "$ROOT/crates/tidings" --root "$PREFIX"
    colors="$ROOT/scripts/noctalia-colors.sh"
fi

install -Dm755 "$colors" "$BIN/tidings-noctalia-colors"

install -d "$(dirname "$SERVICE")"
cat > "$SERVICE" <<EOF
[D-BUS Service]
Name=org.freedesktop.Notifications
Exec=$BIN/tidings daemon
EOF

echo "installed: $BIN/tidings"
case ":$PATH:" in
    *":$BIN:"*) ;;
    *) echo "note: $BIN is not on your PATH; add it, or use the full path" ;;
esac
echo "next: README.md, \"Set up\" (stop your old notification daemon, autostart tidings)"
