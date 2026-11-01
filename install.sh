#!/bin/sh
# Install tidings for the current user (idempotent, no root needed).
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
# The trackpad gesture also needs read access to the touchpad's evdev node;
# see README.md ("Trackpad access") for the one-time udev rule.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PREFIX=${PREFIX:-"$HOME/.local"}
DATA_HOME=${XDG_DATA_HOME:-"$HOME/.local/share"}

cargo install --locked --path "$ROOT/crates/tidings" --root "$PREFIX"

install -Dm755 "$ROOT/scripts/noctalia-colors.sh" "$PREFIX/bin/tidings-noctalia-colors"

install -d "$DATA_HOME/dbus-1/services"
cat > "$DATA_HOME/dbus-1/services/dev.thatcat.tidings.service" <<EOF
[D-BUS Service]
Name=org.freedesktop.Notifications
Exec=$PREFIX/bin/tidings daemon
EOF

echo "installed: $PREFIX/bin/tidings"
