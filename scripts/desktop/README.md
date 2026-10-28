# Live GNOME check

This opt-in test runs the real application on the current GNOME Wayland
compositor, session bus, accessibility bus and notification daemon. It is
separate from the isolated Xvfb smoke check, and is not run in ordinary headless CI.
It presents Quickshare windows in the current desktop session.

In the development shell, build the application and non-Bluetooth test peer:

```sh
nix develop path:.
cargo build --bin gnome-quickshare
cargo build --no-default-features --features test-tools --bin quickshare-probe
scripts/desktop-smoke.sh \
  "$(realpath "${CARGO_TARGET_DIR:-target}/debug/gnome-quickshare")" \
  "$(realpath "${CARGO_TARGET_DIR:-target}/debug/quickshare-probe")" \
  /tmp/quickshare-desktop-evidence
```

The script refuses to run if Quickshare already owns its application bus name,
or a user-level launcher already exists at the test location. It temporarily
creates that launcher with exclusive creation so GNOME can resolve notification
identity, then removes it if unchanged. Settings, files, history and caches live
in a temporary directory. Login startup is disabled. The test briefly enables
visibility under the name “Desktop integration test”, then disables it and exits.
Bluetooth access is disabled for this check; no physical device receives a file.

The accessibility driver inspects only the application's process. It requires
the expected test peer and four-digit matching code in the same window before
activating Accept. It never sends keyboard input to another application. The
monitor filters notification calls to this application ID; cleanup withdraws only
recorded local test-peer notifications. GNOME may retain a default notification
preference entry for the application, as it does after normal app usage.

Evidence includes matching-code accessibility, received-file byte comparison,
Preferences accessibility, application actions, settings and notification calls.
Calls reaching GNOME's service do not establish that a banner was visible under
Do Not Disturb or focus policies. This check does not validate background/file
portal permissions, tray/Files/Shell extensions, Bluetooth hardware, or physical
Android/Windows interoperability.

`QUICKSHARE_GJS` can select an existing GJS binary. Its GLib/GI dependencies must
match `GI_TYPELIB_PATH`; using the project's shell avoids mixing incompatible
Nix package generations. A Wayland session and a working AT-SPI registry are
required; failures save diagnostics without treating them as app compatibility
proof.

Set `QUICKSHARE_SMOKE_PORTALS=1` with `QUICKSHARE_FLATPAK_LIVE=1` to check the
installed sandbox's background setting through the real portal, including window
close/reactivation and restoring the setting to off. An existing login-startup
entry is preserved by refusing this mode; startup is never enabled. The portal
may require a user decision, in which case the test fails and closes its request
rather than approving the prompt automatically.

Set `QUICKSHARE_SMOKE_TRAY=1` to check registration, icon identity, menu labels and
cleanup against an already-running tray host. This mode refuses to install or
enable a host, and does not exercise menu activation or prove icon rendering.

Set `QUICKSHARE_SMOKE_TRAY_ACTIONS=1` together with the tray check to exercise
native (including Nix-wrapped) menu actions. The driver compares the tray
service's bus PID with the test application's unique bus owner before sending
any menu event. It invokes Visible and Hide, checks the exported boolean state,
hides the app with temporary background settings, checks its accessibility
SHOWING state, invokes Open and checks the window is showing, then invokes Quit
and checks process termination and tray removal. Autostart remains false, and
background settings exist only in the test's temporary configuration. For the live Flatpak launcher, combine this mode with
`QUICKSHARE_SMOKE_PORTALS=1`; background operation is enabled through the actual
portal for hide/reopen and restored to off afterwards. This validates sandbox
menu actions but does not prove the host renders the tray icon correctly.

Set `QUICKSHARE_SMOKE_FILE_CHOOSER=1` with `QUICKSHARE_FLATPAK_LIVE=1` to check
the real file chooser and document permission instead of the transfer flow.
Use the actual session runtime directory for `QUICKSHARE_FLATPAK_RUNTIME_DIR`
(normally `/run/user/1000`), so the portal can identify the sandbox instance.
An isolated runtime directory used by headless checks cannot substitute for it.
The driver creates a uniquely named synthetic file outside the sandbox's
filesystem grant, verifies it is unreadable there, selects it through the real
chooser, and checks both the Send selection and the app's document read grant.
After quitting the GUI, a fresh sandbox process compares the granted document
with the reference bytes. Cleanup deletes only this fixture's document grant
and recent-file entry, restores the app's chooser-folder preference if still set
to the test directory, and removes the fixture and temporary launcher.
The accessibility driver tolerates disappearing chooser objects during GNOME
view transitions, while still refusing ambiguous chooser windows.
