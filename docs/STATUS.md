# Implementation and release status

This is a development preview, not a claim to implement Google's entire
proprietary Quick Share product. See PROTOCOL.md for the source-derived protocol.

## Implemented

- Native GTK 4/libadwaita adaptive application, theme/accent following, keyboard
  navigation and accelerators, real native file dialogs and desktop activation.
- Vendored, hardened Quick Share library: BLE-assisted discovery, mDNS/LAN, UKEY2,
  encrypted file/text/URL/Wi-Fi sending and receiving.
- Explicit incoming consent and matching-code UI, hidden startup, ten-minute
  visibility, unsafe filename refusal, cancellation, outgoing queue, progress.
- Notifications, reviewed clipboard copying, open receive folder, local history
  without payload contents, clear history, atomic preferences.
- Background operation, native login autostart, sandbox background portal path.
- Files extension, optional status notifier tray, and optional top-bar visibility indicator source.
- Nix package, dev shell, optional NixOS firewall integration, desktop/AppStream
  metadata, launcher icon, and protocol/architecture documentation.

## Current UI review

The user-requested Claude screenshot review and two re-checks are recorded in
[UI-REVIEW.md](UI-REVIEW.md). The current UI passes workspace Clippy, all 41 tests,
a real local incoming transfer, background activation, and actual entry/switch
persistence checks across closing and reopening Preferences. Current light,
dark, high-contrast and 360px screenshots are saved with the report.

The reviewed UI also builds as a Nix release package, passes all 41 tests in
the Nix build sandbox, and passes installed-wrapper background activation,
settings persistence, invalid-name rejection, and real local transfer checks.

## Baseline package verification

Checked on the pinned x86_64-linux development environment:

- Cargo formatting and workspace Clippy checks pass with warnings denied.
- The pinned Nix release package builds and passes all 41 workspace tests in
  its build sandbox. The optional GNOME indicator package also builds.
- `nix run path:. -- --version` reports 0.1.0. Installed desktop, D-Bus,
  AppStream, Files extension, and icon paths are present. Fifty-four source/resource
  files were compared byte-for-byte with the Nix source snapshot.
- The installed wrapped binary also passes background activation and a real
  local incoming transfer through the GTK consent dialog.
- Model tests reject unsafe filenames, invalid names/ports, empty selections,
  and directory sends.
- Engine regression tests reject encrypted file/text payloads before consent,
  unsafe metadata, negative sizes, oversized chunks, and repeated approval.
- A real TCP protocol loopback test checks encrypted file delivery, collision
  renaming, Unicode text, URLs, explicit rejection, cancellation before consent,
  failed connections, and service recovery after invalid startup settings.
- Isolated Xvfb/D-Bus smoke checks launch real GTK windows in light and dark,
  open a file through GApplication, change visibility through the exported
  action group, open Preferences, and resize to 390 pixels wide. A background-start
  check confirms that a later desktop activation reveals the existing window.
- An independent protocol peer sends to the running GTK app. The smoke driver
  captures the matching-code dialog, clicks Accept, compares the received file
  byte-for-byte, and captures the completed Activity row.

The display sandbox has no GNOME Shell, portal host, notification service, or
system Bluetooth bus; those paths are not validated by the Xvfb check.
Physical devices are not available to the development environment. Local encrypted
loopback tests exercise both ends of the same protocol implementation; they do
not establish compatibility with a particular Android or Windows release.

## Transport hardening after the packaged UI review

Current source passes 41 workspace tests and workspace Clippy with warnings
denied. Regressions cover receive cleanup, fragmented/cancelled reads, invalid
frame lengths, truncated frames, idle timeouts and cancellation during a stalled
frame body. The encrypted loopback check passed three additional consecutive
runs. An earlier full-suite run timed out before the framing fix; these passes
are useful evidence, not proof that all intermittent transport failures are gone.

Current source additionally fixes pre-key cancellation, concurrent incoming
acceptance during an outgoing handshake, failure-to-transfer-ID mapping, and
incoming handler shutdown. Its tests exercise both cancellation directions,
concurrent stalled connections, service shutdown and connection-error reporting.
Current source also validates signed P-256 coordinates, accepts a deterministic
short-coordinate fixture, rejects invalid curve points without panicking, and
checks authenticated-envelope IV/block lengths, schemes and complete PKCS#7
padding. A real TCP malformed-ServerInit test verifies terminal failure reporting.
These lifecycle/cryptographic parsing changes are included in the refreshed
Flatpak and the refreshed Nix release package; both pass all 41 tests.

The refreshed Nix package includes these engine patches and passes all 41 tests
plus installed-wrapper GUI, consent/file-byte and settings checks. Its output is
`/nix/store/g8wbh841jbnxy5z3zqcgxsl36dgdzkwl-gnome-quickshare-0.1.0`.
The refreshed Flatpak passes all 41 tests in the SDK and installed sandbox GUI/local-transfer
checks. Its app commit is
`6012693133db1d35d965c66acbdcd3dcfe6936755d62820ecd986a93ed2f594a`.
Fifty-four source/resource inputs match a retained build snapshot. The
installed Flatpak binary is byte-identical to the exported build binary.
Fifty-four inputs also match the
Nix source snapshot.

## Flatpak development bundle

The GNOME 50 Flatpak builds offline from pinned dependencies and passes all 30
workspace tests in the SDK. The unsigned x86_64 development bundle was installed
in a separate temporary user installation. Its installed sandbox passes version,
hidden startup/desktop activation, visibility, file opening, real consent and
byte-for-byte incoming file comparison, Preferences opening, and 360px layout
checks. Screenshots are recorded alongside the native UI review.

This test uses a private Xvfb/session bus and a temporary-folder filesystem grant.
It does not validate document/background portals, default folder access on a real
desktop, notifications, Bluetooth hardware, or host Files/Shell integrations.
The private-bus harness cannot exercise desktop portals. A separate installed
sandbox check on the live GNOME 50.4 Wayland session passes consent, file-byte
comparison, notification dispatch, background portal on/off persistence, and
window close/reactivation with background enabled. Login startup remains off;
file-chooser grants and actual login startup are not validated by this pass.
The aarch64 manifest sources are pinned but have not been built here.

## Transport deadlines and request lifetime

Current source bounds handshakes to 30 seconds, no-progress file states to 60
seconds, frame writes to 30 seconds, and simultaneous incoming requests to 16.
Consent waits have no application deadline. File idle budgets reset only when
payload progress increases. Channel events and keep-alives cannot extend them.
Request tasks are tracked and aborted/joined on shutdown. Regression tests cover
silent TCP peers in both directions, outgoing terminal failure reporting,
blocked writers, payload-only progress resets, connection admission/slot reuse,
and shutdown releasing every request subscription and socket.

All 41 source tests, workspace Clippy and the native GTK consent/local-file
byte-comparison/settings-persistence check pass after these changes. They are
included in the refreshed Flatpak and still need physical network/interruption
testing. The refreshed Nix wrapper passes the same local GUI transfer/settings checks. Synchronous filesystem calls and cancellation
while a write is blocked have practical limits; see [PROTOCOL.md](PROTOCOL.md).

## Empty-file support

Current source accepts empty regular files and emits their explicit FILE final
chunk rather than silently skipping them. The encrypted loopback covers a lone
empty file, mixed empty/nonempty offers, collision renaming and rejection without
creating a destination. Source reads are bounded to the offered size, and a file
truncated after introduction fails instead of looping on zero-byte reads.
The native GTK smoke check also passes zero-byte file selection, matching-code
consent, receipt with byte comparison after the peer exits, and completed Activity.
At 360×640, Nearby Devices remains visible with the empty file selected.
The refreshed Nix and Flatpak development packages include these changes and
pass installed zero-byte consent/receipt checks. Android/Windows zero-byte
interoperability still needs physical validation.

## Joined application shutdown

Current source keeps the backend thread handle and joins it after the GTK main
loop returns, before process exit. Closing the last non-background window and
Quit already request Stop; the final join ensures that teardown is allowed to
finish. Shutdown is also safe after invalid startup settings and when called
again through another backend clone. Regression checks prove worker event-channel
closure, release of the listening port, and closure of a silent incoming TCP
connection. All 34 workspace tests and workspace Clippy pass. The rebuilt native
GTK check also passes background activation, zero-byte consent/receipt with byte
comparison, preferences persistence and invalid-name rejection, narrow layouts,
and Quit through the joined path. The discovery lifecycle follow-up below
addresses the mDNS errors observed in that earlier check. These changes are
included in the refreshed development packages; forced termination remains
outside the graceful shutdown guarantee.

## Discovery daemon lifecycle

A regression reproduced a surviving mDNS daemon after its discovery task had
finished: mdns-sd does not stop its OS thread when its handle is dropped. Current
source explicitly owns browser/advertiser daemon lifetimes, awaits shutdown on
normal exits, and requests exit from Drop on aborted/error paths. Browse/monitor
receivers remain alive until the daemon acknowledgement. Replacing discovery
cancels the previous worker. TCP reachability probing has a three-second bound
and responds to cancellation; unregister acknowledgements are asynchronous with
a one-second bound. Daemon shutdown acknowledgements have a two-second bound.

All 41 workspace tests and workspace Clippy pass. New regressions check daemon
status after discovery cancellation, after advertising stop over three fresh
instances, after dropping its owner with a surviving observer handle, and full
service shutdown after replacing discovery three times. The refreshed Nix and
Flatpak development packages include this change. The rebuilt native GTK smoke check
also passes zero-byte consent/receipt, background activation, settings changes
that restart the service, persistence/invalid-name checks and Quit. Its application
log has no mDNS errors, including the previous closed-channel and redundant
unregister messages. Physical discovery/network recovery still needs validation.

## Localization

The current source supports GNU gettext for interface text, CLI help, application
errors, consent, notifications, progress, plural counts and countdowns. Named
placeholders allow sentence reordering and do not recursively expand device names
or shared content. History keeps canonical outcomes and translates them on display.
Sizes use GLib's locale-aware formatter. Desktop and AppStream translations are
merged during installation, and catalogs are discovered under the install prefix.

The extraction template, catalog compiler and temporary accented-layout harness
are documented in [po/README.md](../po/README.md). No reviewed human-language
catalogs are included yet. Host Files/Shell extension strings and upstream/OS
error text are outside the current application catalog. The refreshed Flatpak
and Nix package include this localization infrastructure. The temporary accented
catalog passed CLI lookup,
desktop/AppStream merging, background activation, consent and a real local
incoming transfer with byte comparison. Desktop and 360px captures show the
recipient heading and longer labels fitting. English fallback also passed the
full GUI/transfer check and preferences entry/switch persistence checks.

## Live GNOME verification

The current development binary passes a real GNOME Shell 50.4 Wayland check
with the actual compositor, session bus, AT-SPI registry and notification daemon
(no Xvfb and no X11 input injection). The app starts with temporary settings,
responds to desktop activation, presents matching-code consent, accepts only the
expected local test peer via its accessible button, and receives matching file
bytes. Preferences is exposed through accessibility. Both incoming-share and
completion notifications reach GNOME's service without delivery errors.
This establishes dispatch, not visible banner presentation under focus or
Do Not Disturb policies. The client uses the Cairo renderer for this check.

The test refuses an already-running instance, uses an exclusively created
notification launcher and isolated configuration/history/cache, then removes the
launcher, withdraws the test notification and exits. No user launcher was
overwritten. GNOME may retain a default notification preference record for this
application ID. The reproducible opt-in harness is described in
[scripts/desktop/README.md](../scripts/desktop/README.md).

Read-only inspection also confirms background portal version 2 and one BlueZ
adapter. The optional tray has been checked against the existing live GNOME
tray host from the installed Flatpak: registration, icon identity, menu labels,
and disappearance after exit pass. The refreshed installed Flatpak additionally passes menu activation checks
described below. Visual rendering of the host tray icon remains unverified.

The refreshed installed Flatpak also passes real background portal
requests: enabling the setting persists, closing/reopening the window preserves
the running app, and disabling the setting persists. Autostart remains false
and no login-startup entry is created. File-chooser grants, physical Bluetooth
and phone/Windows interoperability remain untested. Bluetooth is disabled in the live desktop
check; its peer connects over loopback. The Xvfb limitations above apply to that
isolated harness, rather than the entire host.

## Release gates

- Physical-device send/receive interoperability, including large files,
  interruptions, device disappearance, and Bluetooth adapter changes.
- Physical network-change recovery and robust error propagation from upstream
  background tasks. The app retries setup, preflights TCP reachability, and
  defers discovery restart until transfers finish; some upstream handshake
  failures can still be logged without a terminal event.
- Transport audit: untrusted frame lengths/allocations, malformed protobufs,
  filename collision/symlink races, stalled frame reads, and crash recovery.
  Request teardown now removes incomplete files, with regression coverage for
  cancellation, EOF, task abortion, malformed completion and replaced paths.
  Forced process termination and atomic pathname-race protection remain open.
  Pre-key cancellation and service-stop handlers have source regressions now;
  signed-key/curve-point and envelope parsing also have source regressions.
  Handshake/file-idle/write deadlines and incoming admission limits are now
  covered in source tests. Broader malformed-message handling, discovery task
  failures, filesystem races and physical network changes still need audit.
- File-chooser portal grants, actual login autostart, Files/Shell extension and
  visual tray-icon validation in installed builds; assistive technology beyond the native
  matching-code/button/Preferences check. Notification dispatch and native
  Wayland local transfers pass in the current development binary.
- Distribution signing, aarch64 Flatpak build verification, translations, and
  release automation.

The app does not implement contacts-only/trusted-account mode, Wi-Fi Direct,
relay transport, resume, or directory recursion.
Empty file sending is now implemented in current source, beyond the upstream
engine's original behavior.

## Refreshed distribution verification

The current x86_64 Nix release and GNOME 50 Flatpak both pass 41 workspace tests
in their respective build environments. Both installed builds pass zero-byte file
selection, matching-code consent, receipt/byte comparison, background activation,
narrow layouts and normal Quit. The Nix wrapper additionally passes preferences
persistence and invalid-name rejection. The isolated Flatpak test cannot grant
background portal permissions; the separate live GNOME Wayland check verifies
background on/off persistence, window close/reactivation, local transfer,
notification dispatch, accessibility consent, and tray registration/menu labels/
removal. Login startup remains off. These checks retain the physical-device and
integration limits listed above.

The source archive is generated by `scripts/package-source.py`, which uses an
explicit source allowlist and normalizes archive metadata. Repeated generation
is byte-identical. All 54 checked source/resource inputs in the archive match both the
retained Flatpak source and Nix source snapshots. Package hashes and verification
scope are recorded in the accompanying `provenance.json` and `SHA256SUMS` files.

## Wi-Fi sharing in current source

Current source adds manual Wi-Fi credential sending through Share Wi-Fi… in the
main menu. A native form validates SSID, security and password, masks the password,
then prepares a Send preview. Users still choose a device and compare the normal
matching code. The engine sends credentials only after acceptance. It supports
Open, WPA-PSK and WEP metadata and preserves the hidden-network flag. Receiving
now uses the declared WifiCredentials protobuf and fails malformed payloads
instead of silently substituting an empty password. Payload Debug implementations
redact Wi-Fi passwords; decoding errors omit credential bytes.

All 41 source tests and workspace Clippy pass. The encrypted loopback covers
WPA, Open, WEP, hidden networks and rejection. The native GTK smoke check fills
the form, captures its masked draft and 360×640 preview, receives a dummy Wi-Fi
share through matching-code consent, captures its review dialog and completion,
verifies Enter-to-prepare and password-only copying using an isolated GTK
clipboard reader, and verifies that the dummy password is absent from
settings/history. Claude reviewed and re-checked the screenshots; its final
verdict confirms all Wi-Fi fixes, including a plain masked row with reveal/hide
and a one-line compact preview. Fresh Activity captures are clean after the
harness requests a full Xvfb repaint (see UI-REVIEW.md). The app
never reads saved NetworkManager credentials or connects to a received network.
Physical-device Wi-Fi acceptance and reviewed translations remain unverified.
The refreshed development packages and matching source archive include these
changes and pass 41 workspace tests in both build environments.

## Native tray actions and themed Wi-Fi checks

The live GNOME harness now activates the test application's actual dbusmenu
entries after checking the tray service PID against the application's unique
bus owner. Current native source, the refreshed Nix package and installed Flatpak pass
Visible, Hide, Open and Quit: exported visibility changes to true/false,
accessibility SHOWING state confirms the main window hides and reopens, Quit
terminates the process, and the tray item disappears. The harness uses temporary
background settings and leaves autostart false. The Flatpak check enables
background operation through the real portal for the hide/reopen test and
restores it to off. It does not visually capture the host's rendered tray icon.

The current source also passes Wi-Fi GUI/clipboard checks under dark,
high-contrast light and high-contrast dark themes. Claude reviewed the captures;
a custom drop-zone border fix now follows AdwStyleManager's high-contrast
property while preserving GTK 4.14 support. Its recheck found no remaining
visible theme issue. Source build and workspace Clippy pass. The refreshed 41-test
bundles include Wi-Fi sharing and this border fix. Physical
interoperability, broad accessibility and live GPU visual validation remain open.

## Wi-Fi distribution refresh

The refreshed x86_64 Nix and GNOME 50 Flatpak each pass all 41 workspace tests.
Both installed builds pass zero-byte file receipt plus Wi-Fi-specific consent,
masked credential review, reveal/hide, password-only clipboard copying and 360px
preview captures. The Nix wrapper additionally passes preference persistence and
invalid-name rejection. Both installed builds pass live GNOME local file-byte
comparison, matching-code accessibility consent, notification dispatch and all
four tray menu actions. The installed sandbox also passes the high-contrast
Wi-Fi capture and clipboard flow. The sandbox additionally passes real background portal
on/off persistence and hide/reopen through its tray menu. These checks disable
Bluetooth and do not establish physical Android/Windows interoperability.

All 54 checked application source/resource inputs match both build snapshots and
the matching source archive. `scripts/verify-source.py` records hashes and can
retain an ephemeral build snapshot. `scripts/package-source.py` creates the
normalized source archive; repeated generation remains byte-identical. The
previous 30-test and 38-test artifact sets remain separate historical snapshots.

## Real file chooser and persistent document permission

The installed Flatpak passes the real GNOME Wayland file chooser check. A unique
synthetic fixture outside its filesystem grant is unreadable before selection.
The GNOME portal delegates to Files; the accessibility driver selects the fixture
and submits Add Files, then confirms the filename appears in Send. The document
portal records read permission for this app. After the GUI exits, a fresh sandbox
process reads the granted document path and compares its bytes with the fixture.
Cleanup confirms the document grant and recent-file entry are removed and restores
the test app's chooser-folder preference. Evidence is recorded under
`ui-review/file-chooser-persistent-grant` in the thread's evidence directory.

The verifier explicitly NUL-terminates the filename passed to Documents.Lookup;
GJS byte conversion alone omits that required byte. The chooser driver retries
stale accessibility objects during view transitions. These are test-harness
changes; the installed application binaries remain unchanged. This validates file
selection and read access after process exit, not persistence across login or
reboot, folder grants, drag and drop, or automatic startup.
