#!/usr/bin/env bash
# Opt-in live-session test: presents only Quickshare's own windows, uses its
# accessibility actions, and grants no Files/Shell integration or login startup.
set -euo pipefail
binary="${1:?Usage: desktop-smoke.sh BINARY PROBE OUTPUT}"
probe="${2:?Choose the native test peer}"
output="${3:?Choose an evidence directory}"
project_dir=$(cd "$(dirname "$0")/.." && pwd)
gjs="${QUICKSHARE_GJS:-gjs}"
app_id=io.github.oddship.GnomeQuickshare
object_path=/io/github/oddship/GnomeQuickshare
[[ "${XDG_SESSION_TYPE:-}" == wayland ]] || { echo 'This check requires a live Wayland session.' >&2; exit 1; }
owner=$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner "$app_id")
[[ "$owner" == '(false,)' ]] || { echo 'An existing Quickshare instance is running; refusing to touch it.' >&2; exit 1; }
tray=false
background=false
if [[ "${QUICKSHARE_SMOKE_TRAY_ACTIONS:-0}" == 1 ]]; then
    [[ "${QUICKSHARE_SMOKE_TRAY:-0}" == 1 ]] || { echo 'Tray actions require the existing tray-host check.' >&2; exit 1; }
    if [[ "${QUICKSHARE_FLATPAK_LIVE:-0}" == 1 ]]; then
        [[ "${QUICKSHARE_SMOKE_PORTALS:-0}" == 1 ]] || { echo 'Sandbox tray actions require the background portal check.' >&2; exit 1; }
    else
        background=true
    fi
fi
if [[ "${QUICKSHARE_SMOKE_TRAY:-0}" == 1 ]]; then
    watcher=$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner org.kde.StatusNotifierWatcher)
    [[ "$watcher" == '(true,)' ]] || { echo 'No existing tray host; refusing to install or enable one.' >&2; exit 1; }
    tray=true
fi
if [[ "${QUICKSHARE_SMOKE_PORTALS:-0}" == 1 ]]; then
    [[ "${QUICKSHARE_FLATPAK_LIVE:-0}" == 1 ]] || { echo 'Portal mode requires the live Flatpak launcher.' >&2; exit 1; }
    [[ ! -e "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/$app_id.desktop" ]] || { echo 'An existing login-startup entry must be preserved; skipping portal mode.' >&2; exit 1; }
fi
mkdir -p "$output"
output=$(cd "$output" && pwd)
sandbox=$(mktemp -d -t quickshare-desktop.XXXXXX)
host_config_home="${XDG_CONFIG_HOME:-$HOME/.config}"
host_data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
launcher="$host_data_home/applications/$app_id.desktop"
export QUICKSHARE_DESKTOP_LAUNCHER="$launcher" QUICKSHARE_DESKTOP_BINARY="$binary" QUICKSHARE_DESKTOP_PROJECT="$project_dir"
cleanup() {
    if [[ -n "${unique:-}" ]]; then
        timeout 3s gdbus call --session --dest "$unique" --object-path "$object_path" --method org.gtk.Actions.Activate visible '[<false>]' '{}' >/dev/null 2>&1 || true
        timeout 3s gdbus call --session --dest "$unique" --object-path "$object_path" --method org.gtk.Actions.Activate quit '[]' '{}' >/dev/null 2>&1 || true
    fi
    if [[ -f "$output/notifications.log" ]]; then
        while IFS= read -r notification_id; do
            timeout 3s gdbus call --session --dest org.gtk.Notifications --object-path /org/gtk/Notifications --method org.gtk.Notifications.RemoveNotification "$app_id" "$notification_id" >/dev/null 2>&1 || true
        done < <(python3 "$project_dir/scripts/desktop/notification-ids.py" "$output/notifications.log")
    fi
    kill "${peer_pid:-}" "${app_pid:-}" "${monitor_pid:-}" "${portal_monitor_pid:-}" 2>/dev/null || true
    if [[ -f "$sandbox/launcher" ]] && cmp -s "$sandbox/launcher" "$launcher"; then rm -f "$launcher"; fi
    if [[ -n "${portal_fixture_file:-}" ]]; then
        "$gjs" "$project_dir/scripts/desktop/document-grant.js" cleanup "$portal_fixture_file" "$output/document-cleanup.json" >> "$output/file-chooser.log" 2>&1 || true
        env XDG_DATA_HOME="$host_data_home" XDG_CONFIG_HOME="$host_config_home" GSETTINGS_BACKEND=memory "$gjs" "$project_dir/scripts/desktop/chooser-recents.js" "$portal_fixture_file" "$output/fixture-recents-cleanup.json" >> "$output/file-chooser.log" 2>&1 || true
        if [[ -f "$sandbox/portal-lastfolder.before" ]]; then
            current_folder=$(env XDG_CONFIG_HOME="$host_config_home" dconf read "$portal_folder_key")
            expected_folder=$("$gjs" -c 'print(new imports.gi.GLib.Variant("s", ARGV[0]).print(true));' "$portal_fixture_dir")
            if [[ "$current_folder" == "$expected_folder" ]]; then
                previous_folder=$(cat "$sandbox/portal-lastfolder.before")
                if [[ -n "$previous_folder" ]]; then
                    env XDG_CONFIG_HOME="$host_config_home" dconf write "$portal_folder_key" "$previous_folder"
                else
                    env XDG_CONFIG_HOME="$host_config_home" dconf reset "$portal_folder_key"
                fi
                printf 'Restored the test app chooser folder preference.\n' > "$output/chooser-preference-cleanup.txt"
            fi
        fi
    fi
    rm -rf "$sandbox" "${portal_fixture_dir:-}"
}
trap cleanup EXIT
# Exclusive creation refuses to overwrite any existing launcher. The temporary
# entry lets GNOME resolve this test app's notification identity and icon.
python3 - <<'PY'
import os,pathlib
p=pathlib.Path(os.environ['QUICKSHARE_DESKTOP_LAUNCHER'])
p.parent.mkdir(parents=True,exist_ok=True)
s=(pathlib.Path(os.environ['QUICKSHARE_DESKTOP_PROJECT'])/'data/io.github.oddship.GnomeQuickshare.desktop').read_text()
s=s.replace('Exec=gnome-quickshare %F','Exec='+os.environ['QUICKSHARE_DESKTOP_BINARY']+' %F')
s=s.replace('Icon=io.github.oddship.GnomeQuickshare','Icon='+os.environ['QUICKSHARE_DESKTOP_PROJECT']+'/data/icons/hicolor/scalable/apps/io.github.oddship.GnomeQuickshare.svg')
with p.open('x') as f:f.write(s)
PY
cp "$launcher" "$sandbox/launcher"
export XDG_CONFIG_HOME="$sandbox/config" XDG_STATE_HOME="$sandbox/state" XDG_DATA_HOME="$sandbox/share" XDG_CACHE_HOME="$sandbox/cache"
mkdir -p "$XDG_CONFIG_HOME/gnome-quickshare" "$XDG_DATA_HOME/applications" "$XDG_DATA_HOME/icons" "$XDG_CACHE_HOME" "$sandbox/received"
cp "$launcher" "$XDG_DATA_HOME/applications/"
cp -r "$project_dir/data/icons/." "$XDG_DATA_HOME/icons/"
printf '{"name":"Desktop integration test","downloads":"%s","background":%s,"autostart":false,"tray":%s,"port":null}\n' "$sandbox/received" "$background" "$tray" > "$XDG_CONFIG_HOME/gnome-quickshare/settings.json"
# Keep the real compositor, session/a11y buses and notification daemon. Avoid
# advertising over Bluetooth. Visibility is enabled briefly for the verified
# local peer; the accessibility driver refuses any other peer/code combination.
export GDK_BACKEND=wayland GSK_RENDERER=cairo GTK_A11Y=atspi LANGUAGE=en
export DBUS_SYSTEM_BUS_ADDRESS="unix:path=$sandbox/no-system-bus"
dbus-monitor --session "type='method_call',interface='org.gtk.Notifications',member='AddNotification',arg0='$app_id'" > "$output/notifications.log" 2>&1 &
monitor_pid=$!
"$binary" --background > "$output/application.log" 2>&1 &
app_pid=$!
gdbus wait --session --timeout 15 "$app_id"
sleep 0.5
kill -0 "$app_pid"
unique=$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.GetNameOwner "$app_id" | sed -n "s/.*'\([^']*\)'.*/\1/p")
host_pid=$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.GetConnectionUnixProcessID "$unique" | sed -n 's/.*uint32 \([0-9]*\).*/\1/p')
[[ -n "$unique" && -n "$host_pid" ]]
printf 'shell_pid=%s
bus_pid=%s
unique=%s
' "$app_pid" "$host_pid" "$unique" > "$output/process.txt"
app_call() { gdbus call --session --dest "$unique" --object-path "$object_path" --method "$@"; }
# A registered tray can precede backend readiness. Wait for the listener before
# testing menu visibility commands, so startup cannot masquerade as a lost action.
port=
for _ in $(seq 1 100); do
    if described=$(app_call org.gtk.Actions.Describe listen-port 2>/dev/null); then
        port=$(sed -n 's/.*uint32 \([0-9]*\).*/\1/p' <<< "$described")
        [[ -n "$port" && "$port" != 0 ]] && break
    fi
    sleep 0.1
done
[[ -n "$port" && "$port" != 0 ]]
app_call org.freedesktop.Application.Activate '{}'
if [[ "${QUICKSHARE_SMOKE_FILE_CHOOSER:-0}" == 1 ]]; then
    [[ "${QUICKSHARE_FLATPAK_LIVE:-0}" == 1 ]] || { echo 'File chooser mode requires the installed live sandbox.' >&2; exit 1; }
    portal_pid=$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.GetConnectionUnixProcessID org.freedesktop.impl.portal.desktop.gnome | sed -n 's/.*uint32 \([0-9]*\).*/\1/p')
    [[ -n "$portal_pid" ]]
    portal_namespace=${unique#:}
    portal_namespace=${portal_namespace//./_}
    dbus-monitor --session "type='method_call',interface='org.freedesktop.impl.portal.FileChooser',member='OpenFile',arg1='$app_id'" "type='signal',interface='org.freedesktop.portal.Request',member='Response',path_namespace='/org/freedesktop/portal/desktop/request/$portal_namespace'" > "$output/file-chooser-portal.log" 2>&1 &
    portal_monitor_pid=$!
    sleep 0.2
    portal_fixture_dir=$(mktemp -d "$output/../quickshare-portal-input.XXXXXX")
    portal_fixture_dir=$(realpath "$portal_fixture_dir")
    portal_fixture_name="quickshare-portal-fixture-${portal_fixture_dir##*.}.txt"
    portal_fixture_file="$portal_fixture_dir/$portal_fixture_name"
    portal_folder_key="/org/gnome/portal/filechooser/$app_id/last-folder-path"
    env XDG_CONFIG_HOME="$host_config_home" dconf read "$portal_folder_key" > "$sandbox/portal-lastfolder.before"
    printf 'Only a synthetic Quickshare chooser fixture.\n' > "$portal_fixture_file"
    cp "$portal_fixture_file" "$sandbox/fixture-reference.txt"
    "$gjs" "$project_dir/scripts/desktop/document-grant.js" absent "$portal_fixture_file" "$output/document-before.json"
    env XDG_RUNTIME_DIR="$QUICKSHARE_FLATPAK_RUNTIME_DIR" flatpak run --user --filesystem="$sandbox" --command=sh "$app_id" -c 'test ! -r "$1"' sh "$portal_fixture_file"
    printf 'Fixture unreadable before chooser grant.\n' > "$output/file-access-before.txt"
    fixture_folder_variant=$("$gjs" -c 'print(new imports.gi.GLib.Variant("s", ARGV[0]).print(true));' "$portal_fixture_dir")
    env XDG_CONFIG_HOME="$host_config_home" dconf write "$portal_folder_key" "$fixture_folder_variant"
    app_call org.gtk.Actions.Activate choose-files '[]' '{}'
    "$gjs" "$project_dir/scripts/desktop/file-chooser.js" "$portal_pid" "$output/file-chooser-accessibility.json" "$portal_fixture_file" > "$output/file-chooser.log" 2>&1
    "$gjs" "$project_dir/scripts/desktop/accessibility.js" selected-file "$host_pid" "$output/file-selected-accessibility.json" "$portal_fixture_name" >> "$output/file-chooser.log" 2>&1
    "$gjs" "$project_dir/scripts/desktop/document-grant.js" verify "$portal_fixture_file" "$output/document-grant.json"
    document_path=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["documentPath"])' "$output/document-grant.json")
    # Quit the GUI before testing read permission in a fresh sandbox instance.
    app_call org.gtk.Actions.Activate quit '[]' '{}'
    wait "$app_pid"
    app_pid=
    env XDG_RUNTIME_DIR="$QUICKSHARE_FLATPAK_RUNTIME_DIR" flatpak run --user --filesystem="$sandbox" --command=sh "$app_id" -c 'cmp "$1" "$2"' sh "$document_path" "$sandbox/fixture-reference.txt"
    printf 'Document readable with matching bytes after GUI exit.\n' > "$output/file-access-after.txt"
    echo 'Real file chooser selection and persistent document read grant passed.'
    exit 0
fi
if [[ "$tray" == true ]]; then
    "$gjs" "$project_dir/scripts/desktop/tray.js" present "$output/tray-registered.json" > "$output/tray.log" 2>&1
fi
if [[ "${QUICKSHARE_SMOKE_TRAY_ACTIONS:-0}" == 1 ]]; then
    tray_invoke() {
        "$gjs" "$project_dir/scripts/desktop/tray.js" invoke "$output/tray-$1.json" "$host_pid" "$unique" "$2" >> "$output/tray.log" 2>&1
    }
    wait_visibility() {
        for _ in $(seq 1 100); do
            local state
            state=$(app_call org.gtk.Actions.Describe visible)
            if rg -Fq "[<$1>]" <<< "$state"; then
                printf '%s\n' "$state" > "$output/tray-visibility-$1.txt"
                return
            fi
            sleep 0.1
        done
        echo 'Tray action did not update visibility.' >&2
        return 1
    }
    wait_visibility false
    tray_invoke visible 'Visible for 10 minutes'
    wait_visibility true
    tray_invoke hidden 'Hide this computer'
    wait_visibility false
    if [[ "${QUICKSHARE_FLATPAK_LIVE:-0}" != 1 ]]; then
        app_call org.gtk.Actions.Activate close-window '[]' '{}'
        kill -0 "$app_pid"
        "$gjs" "$project_dir/scripts/desktop/accessibility.js" window-visible "$host_pid" "$output/tray-window-hidden.json" false >> "$output/accessibility.log" 2>&1
        tray_invoke open 'Open Quickshare'
        "$gjs" "$project_dir/scripts/desktop/accessibility.js" window-visible "$host_pid" "$output/tray-window-open.json" true >> "$output/accessibility.log" 2>&1
    fi
fi
app_call org.gtk.Actions.List > "$output/actions.txt"
app_call org.gtk.Actions.Describe visible > "$output/visibility.txt"
port=$(app_call org.gtk.Actions.Describe listen-port | sed -n 's/.*uint32 \([0-9]*\).*/\1/p')
[[ -n "$port" && "$port" != 0 ]]
app_call org.gtk.Actions.Activate visible '[<true>]' '{}'
app_call org.gtk.Actions.Describe visible > "$output/visibility-enabled.txt"
printf 'A local test share for live GNOME integration.\n' > "$XDG_CACHE_HOME/sample.txt"
"$probe" "127.0.0.1:$port" "$XDG_CACHE_HOME/sample.txt" > "$output/peer.log" 2>&1 &
peer_pid=$!
for _ in $(seq 1 100); do
    code=$(sed -n 's/^Authentication code: \([0-9]\{4\}\)$/\1/p' "$output/peer.log" | head -1)
    [[ -n "$code" ]] && break
    sleep 0.1
done
[[ -n "${code:-}" ]]
"$gjs" "$project_dir/scripts/desktop/accessibility.js" accept "$host_pid" "$output/consent-accessibility.json" "$code" > "$output/accessibility.log" 2>&1
rm -f "$output/consent-accessibility.json.pending" "$output/consent-accessibility.json.failure"
wait "$peer_pid"
peer_pid=
cmp "$XDG_CACHE_HOME/sample.txt" "$sandbox/received/sample.txt"
app_call org.gtk.Actions.Activate visible '[<false>]' '{}'
app_call org.gtk.Actions.Activate preferences '[]' '{}'
sleep 0.5
"$gjs" "$project_dir/scripts/desktop/accessibility.js" snapshot "$host_pid" "$output/preferences-accessibility.json" >> "$output/accessibility.log" 2>&1
rm -f "$output/preferences-accessibility.json.pending" "$output/preferences-accessibility.json.failure"
if [[ "${QUICKSHARE_SMOKE_PORTALS:-0}" == 1 ]]; then
    flatpak permission-show "$app_id" > "$output/permissions-before.txt"
    "$gjs" "$project_dir/scripts/desktop/accessibility.js" set-background "$host_pid" "$output/background-on-accessibility.json" true >> "$output/accessibility.log" 2>&1
    wait_setting() {
        local expected="$1"
        for _ in $(seq 1 100); do
            if python3 - "$XDG_CONFIG_HOME/gnome-quickshare/settings.json" "$expected" <<'PYCHECK'
import json,sys
x=json.load(open(sys.argv[1]))
sys.exit(0 if x['background'] is (sys.argv[2]=='true') and x['autostart'] is False else 1)
PYCHECK
            then return; fi
            sleep 0.1
        done
        echo 'The portal setting did not settle; a permission prompt may require a user decision.' >&2
        return 1
    }
    wait_setting true
    flatpak permission-show "$app_id" > "$output/permissions-granted.txt"
    "$gjs" "$project_dir/scripts/desktop/accessibility.js" close-preferences "$host_pid" "$output/preferences-closed.json" >> "$output/accessibility.log" 2>&1
    app_call org.gtk.Actions.Activate close-window '[]' '{}'
    sleep 0.3
    kill -0 "$app_pid"
    if [[ "${QUICKSHARE_SMOKE_TRAY_ACTIONS:-0}" == 1 ]]; then
        "$gjs" "$project_dir/scripts/desktop/accessibility.js" window-visible "$host_pid" "$output/tray-window-hidden.json" false >> "$output/accessibility.log" 2>&1
        tray_invoke open 'Open Quickshare'
        "$gjs" "$project_dir/scripts/desktop/accessibility.js" window-visible "$host_pid" "$output/tray-window-open.json" true >> "$output/accessibility.log" 2>&1
    else
        app_call org.freedesktop.Application.Activate '{}'
    fi
    app_call org.gtk.Actions.Activate preferences '[]' '{}'
    "$gjs" "$project_dir/scripts/desktop/accessibility.js" set-background "$host_pid" "$output/background-off-accessibility.json" false >> "$output/accessibility.log" 2>&1
    wait_setting false
fi
# Actual notification calls must have reached the live session daemon, and GTK
# must not have logged a delivery failure or crash. This does not prove a banner
# was visible when GNOME's Do Not Disturb or focus policies suppress it.
rg -q 'Incoming share' "$output/notifications.log"
rg -q 'Transfer complete' "$output/notifications.log"
if rg -i 'panic|critical|unable to send notifications|Portal operation not allowed|Wayland display.*failed' "$output/application.log"; then exit 1; fi
if [[ "${QUICKSHARE_SMOKE_TRAY_ACTIONS:-0}" == 1 ]]; then
    tray_invoke quit 'Quit'
    for _ in $(seq 1 100); do
        kill -0 "$app_pid" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$app_pid" 2>/dev/null; then echo 'Tray Quit did not terminate the application.' >&2; exit 1; fi
else
    app_call org.gtk.Actions.Activate quit '[]' '{}'
fi
wait "$app_pid"
app_pid=
if [[ "$tray" == true ]]; then
    "$gjs" "$project_dir/scripts/desktop/tray.js" absent "$output/tray-removed.json" >> "$output/tray.log" 2>&1
fi
cp "$XDG_CONFIG_HOME/gnome-quickshare/settings.json" "$output/settings.json"
echo 'Live Wayland startup, accessibility consent, local bytes and notification dispatch passed.'
if [[ "${QUICKSHARE_SMOKE_TRAY_ACTIONS:-0}" == 1 ]]; then echo 'Tray visibility, hide, reopen and Quit actions passed.'; fi
