#!/usr/bin/env bash
set -euo pipefail
binary="${1:-target/debug/gnome-quickshare}"
output="${2:-/tmp/gnome-quickshare-ui}"
mkdir -p "$output"
sandbox=$(mktemp -d -t quickshare-ui.XXXXXX)
trap 'kill "${app_pid:-}" "${xvfb_pid:-}" 2>/dev/null || true; rm -rf "$sandbox"' EXIT
export XDG_CONFIG_HOME="$sandbox/config" XDG_STATE_HOME="$sandbox/state" XDG_DATA_HOME="$sandbox/share" XDG_CACHE_HOME="$sandbox/cache"
export GSETTINGS_BACKEND=memory GTK_A11Y=none GSK_RENDERER=cairo GDK_BACKEND=x11 GIO_USE_VFS=local
export XDG_RUNTIME_DIR="$sandbox/runtime"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CACHE_HOME"
chmod 700 "$XDG_RUNTIME_DIR"
export DBUS_SYSTEM_BUS_ADDRESS="unix:path=$sandbox/no-system-bus"
mkdir -p "$XDG_CONFIG_HOME/gnome-quickshare" "$sandbox/received"
printf '{"name":"My computer","downloads":"%s","background":false,"autostart":false,"port":null}\n' "$sandbox/received" > "$XDG_CONFIG_HOME/gnome-quickshare/settings.json"
mkdir -p "$XDG_DATA_HOME/icons" "$XDG_DATA_HOME/applications"
cp -r data/icons/. "$XDG_DATA_HOME/icons/"
cp data/io.github.oddship.GnomeQuickshare.desktop "$XDG_DATA_HOME/applications/"
export XDG_DATA_DIRS="$XDG_DATA_HOME:${XDG_DATA_DIRS:-/run/current-system/sw/share}"
Xvfb -displayfd 3 -screen 0 1280x1000x24 3> "$sandbox/display" > "$output/xvfb.log" 2>&1 &
xvfb_pid=$!
for _ in $(seq 1 100); do test -s "$sandbox/display" && break; sleep 0.05; done
export DISPLAY=":$(cat "$sandbox/display")"
export QUICKSHARE_SMOKE_BINARY="$binary" QUICKSHARE_SMOKE_OUTPUT="$output"
dbus-run-session --config-file=scripts/dbus-test.conf -- bash scripts/ui-session.sh
