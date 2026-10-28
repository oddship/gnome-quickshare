#!/usr/bin/env bash
set -euo pipefail
output="$QUICKSHARE_SMOKE_OUTPUT"
launch_args=()
if [[ "${QUICKSHARE_SMOKE_BACKGROUND:-0}" == 1 ]]; then launch_args=(--background); fi
"$QUICKSHARE_SMOKE_BINARY" "${launch_args[@]}" > "$output/application.log" 2>&1 &
app_pid=$!
trap 'kill "$app_pid" 2>/dev/null || true' EXIT
if [[ "${QUICKSHARE_SMOKE_BACKGROUND:-0}" == 1 ]]; then
    gdbus wait --session --timeout 15 io.github.oddship.GnomeQuickshare
    sleep 0.5
    if xdotool search --onlyvisible --name '^Quickshare$' >/dev/null; then echo 'Background launch presented a window' >&2; exit 1; fi
    gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.freedesktop.Application.Activate '{}'
fi
window=$(timeout 20s xdotool search --sync --onlyvisible --name '^Quickshare$' | head -1)
# Identify dialogs through the real transient relationship; sandbox launchers
# may have a different PID and GTK need not export _NET_WM_PID.
consent_windows() {
    local candidate parent
    while read -r candidate; do
        parent=$(xprop -id "$candidate" WM_TRANSIENT_FOR 2>/dev/null || true)
        if [[ "$parent" == *"$(printf '0x%x' "$window")"* ]]; then
            echo "$candidate"
        fi
    done < <(xdotool search --onlyvisible --name '.')
}
wait_for_transient() {
    local candidate
    for _ in $(seq 1 200); do
        candidate=$(consent_windows | head -1)
        if [[ -n "$candidate" ]]; then echo "$candidate"; return; fi
        sleep 0.05
    done
    echo 'Timed out waiting for a transient dialog' >&2
    return 1
}
refresh_capture_surface() {
    # Xvfb without a compositor can retain stale pixels after stack changes.
    # Resize away and back to request a full surface repaint before capture.
    local geometry capture_width capture_height
    geometry=$(xdotool getwindowgeometry --shell "$window")
    capture_width=$(sed -n 's/^WIDTH=//p' <<< "$geometry")
    capture_height=$(sed -n 's/^HEIGHT=//p' <<< "$geometry")
    xdotool windowsize "$window" "$((capture_width + 1))" "$capture_height"
    sleep 0.05
    xdotool windowsize "$window" "$capture_width" "$capture_height"
    sleep 0.2
}
xdotool windowfocus --sync "$window"
sleep 1.5
magick import -window "$window" "$output/send.png"
if [[ "${QUICKSHARE_SMOKE_WIFI:-0}" == 1 ]]; then
    gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.Activate compose-wifi '[]' '{}'
    wifi_dialog=$(wait_for_transient)
    sleep 0.3
    magick import -window "$wifi_dialog" "$output/wifi-form.png"
    xdotool windowfocus "$wifi_dialog"
    xdotool mousemove --window "$wifi_dialog" 135 192 click 1
    xdotool type --clearmodifiers 'Quickshare test network'
    xdotool mousemove --window "$wifi_dialog" 140 300 click 1
    xdotool type --clearmodifiers 'not-a-real-password'
    xdotool mousemove --window "$wifi_dialog" 250 352 click 1
    sleep 0.2
    magick import -window "$wifi_dialog" "$output/wifi-filled.png"
    # Enter in the password entry activates the validated primary response.
    xdotool mousemove --window "$wifi_dialog" 140 300 click 1
    xdotool key Return
    sleep 0.3
    [[ $(consent_windows | wc -l) == 0 ]]
    magick import -window "$window" "$output/wifi-preview.png"
    xdotool windowsize "$window" 360 640
    sleep 0.3
    magick import -window "$window" "$output/wifi-small.png"
    xdotool windowsize "$window" 820 820
fi
gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.Activate show-activity '[]' '{}'
refresh_capture_surface
magick import -window "$window" "$output/activity-empty.png"
gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.Activate compose-text '[]' '{}'
xdotool windowfocus "$window"
xdotool type --clearmodifiers 'A link or a message to share.'
sleep 0.2
magick import -window "$window" "$output/text.png"
# Exercise real session-bus action state and capture Receive.
gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.List > "$output/actions.txt"
gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.Activate visible '[<true>]' '{}' > "$output/visible-on.txt"
gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.Describe visible > "$output/visible-state.txt"
gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.Activate show-receive '[]' '{}'
sleep 0.3
magick import -window "$window" "$output/receive.png"
# Exercise file opening through GApplication.
if [[ "${QUICKSHARE_SMOKE_EMPTY:-0}" == 1 ]]; then
    : > "$XDG_CACHE_HOME/sample.txt"
else
    printf 'A sample file for the native file-selection view.\n' > "$XDG_CACHE_HOME/sample.txt"
fi
"$QUICKSHARE_SMOKE_BINARY" "$XDG_CACHE_HOME/sample.txt"
sleep 0.3
magick import -window "$window" "$output/files.png"
if [[ -n "${QUICKSHARE_SMOKE_PROBE:-}" ]]; then
    port=$(gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.Describe listen-port | sed -n 's/.*uint32 \([0-9]*\).*/\1/p')
    probe_payload="$XDG_CACHE_HOME/sample.txt"
    if [[ "${QUICKSHARE_SMOKE_WIFI:-0}" == 1 ]]; then probe_payload=--wifi; fi
    "$QUICKSHARE_SMOKE_PROBE" "127.0.0.1:$port" "$probe_payload" > "$output/peer.log" 2>&1 &
    peer_pid=$!
    for _ in $(seq 1 200); do
        count=$(consent_windows | wc -l)
        [[ "$count" -gt 0 ]] && break
        sleep 0.05
    done
    consent=$(consent_windows | tail -1)
    [[ -n "$consent" ]]
    [[ "$consent" != "$window" ]]
    sleep 0.3
    magick import -window "$consent" "$output/consent.png"
    # Decline has initial focus. Tab selects Accept even when translated
    # labels cause libadwaita to stack the response buttons vertically.
    xdotool windowfocus "$consent"
    xdotool key Tab space
    wait "$peer_pid"
    if [[ "${QUICKSHARE_SMOKE_WIFI:-0}" == 1 ]]; then
        review=$(wait_for_transient)
        sleep 0.3
        magick import -window "$review" "$output/wifi-received.png"
        xdotool windowfocus "$review"
        xdotool mousemove --window "$review" 250 280 click 1
        sleep 0.2
        magick import -window "$review" "$output/wifi-revealed.png"
        xdotool mousemove --window "$review" 250 280 click 1
        xdotool mousemove --window "$review" 150 400 click 1
        sleep 0.2
        [[ $(consent_windows | wc -l) == 0 ]]
        # Read only this isolated Xvfb clipboard and never print its contents.
        gjs -c '
            imports.gi.versions.Gtk = "4.0";
            const { Gtk, Gdk, GLib } = imports.gi;
            Gtk.init();
            let ok = false;
            const loop = new GLib.MainLoop(null, false);
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 3000, () => { loop.quit(); return GLib.SOURCE_REMOVE; });
            Gdk.Display.get_default().get_clipboard().read_text_async(null, (clipboard, result) => {
                ok = clipboard.read_text_finish(result) === "not-a-real-password";
                loop.quit();
            });
            loop.run();
            if (!ok) throw new Error("Clipboard does not contain only the fixture password");
        '
        # Wi-Fi details stay out of persisted settings/history.
        if rg -F 'not-a-real-password' "$XDG_STATE_HOME" "$XDG_CONFIG_HOME"; then exit 1; fi
    else
        received=$(find "$(dirname "$XDG_CONFIG_HOME")/received" -type f -name sample.txt -print -quit)
        cmp "$XDG_CACHE_HOME/sample.txt" "$received"
    fi
    gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.Activate show-activity '[]' '{}'
    # Allow the stack transition and terminal row redraw to settle before capture.
    sleep 1
    refresh_capture_surface
    magick import -window "$window" "$output/activity.png"
fi
gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.Activate preferences '[]' '{}'
prefs=$(wait_for_transient)
sleep 0.4
magick import -window "$prefs" "$output/preferences.png"
if [[ "${QUICKSHARE_SMOKE_SETTINGS:-0}" == 1 ]]; then
    xdotool windowfocus "$prefs"
    xdotool mousemove --window "$prefs" 170 163 click 1
    xdotool key ctrl+a
    xdotool type --clearmodifiers 'Review workstation'
    xdotool key Return
    sleep 0.5
    xdotool mousemove --window "$prefs" 475 327 click 1
    sleep 0.5
    python3 - <<'PYSETTINGS'
import json,os
p=os.path.join(os.environ['XDG_CONFIG_HOME'],'gnome-quickshare/settings.json')
x=json.load(open(p))
assert x['name']=='Review workstation',x
assert x['background'] is True,x
PYSETTINGS
    xdotool key Escape
    gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.Activate preferences '[]' '{}'
    prefs=$(wait_for_transient)
    sleep 0.3
    magick import -window "$prefs" "$output/preferences-saved.png"
    xdotool windowfocus "$prefs"
    xdotool mousemove --window "$prefs" 475 327 click 1
    sleep 0.3
    xdotool mousemove --window "$prefs" 170 163 click 1
    xdotool key ctrl+a BackSpace Return
    sleep 0.3
    magick import -window "$prefs" "$output/preferences-error.png"
    python3 - <<'PYSETTINGS'
import json,os
x=json.load(open(os.path.join(os.environ['XDG_CONFIG_HOME'],'gnome-quickshare/settings.json')))
assert x['name']=='Review workstation',x
assert x['background'] is False,x
PYSETTINGS
fi
xdotool windowfocus "$prefs"
xdotool key Escape
# Resize the real app to a narrow mobile-like layout.
xdotool windowsize "$window" 390 800
sleep 0.4
magick import -window "$window" "$output/narrow.png"
for page in send receive; do
    gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.Activate "show-$page" '[]' '{}'
    sleep 0.2
    magick import -window "$window" "$output/narrow-$page.png"
done
xdotool windowsize "$window" 360 640
for page in send receive; do
    gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.Activate "show-$page" '[]' '{}'
    sleep 0.2
    magick import -window "$window" "$output/small-$page.png"
done
if ! rg -q 'true' "$output/visible-state.txt"; then echo 'Visibility action did not become active' >&2; exit 1; fi
if rg -i 'panic|critical|read-only file system' "$output/application.log"; then exit 1; fi
gdbus call --session --dest io.github.oddship.GnomeQuickshare --object-path /io/github/oddship/GnomeQuickshare --method org.gtk.Actions.Activate quit '[]' '{}'
wait "$app_pid"
