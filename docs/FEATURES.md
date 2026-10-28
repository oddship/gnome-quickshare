# Reference feature coverage

Packet reference commit: `1e9fc400020b6fcdbae6ed586872a8d17b6fde26`.
NixOS Update Manager design/packaging reference commit:
`5f8a9289b88f31a51953db7cb51c288d6adeda2c`.
Protocol base revision and local changes: [engine provenance](../vendor/rqs_lib/PROVENANCE.md).

“Implemented” describes code present here. It does not mean every host, portal,
or Android vendor combination has been tested. See [validation](STATUS.md).

| Feature | GNOME Quickshare | Evidence / limit |
|---|---|---|
| Native adaptive GNOME UI | Implemented | Real GTK screenshots, light/dark and 390px layout |
| BLE-assisted discovery | Implemented | Engine integration; physical device verification needed |
| mDNS discovery and removal | Implemented | Live engine; same-LAN reachability required |
| File sending and receiving | Implemented | Encrypted TCP loopback and real GTK incoming transfer |
| Multiple files / queue | Implemented | Payload snapshots and serialized outgoing dispatch |
| Same-name collision handling | Implemented and hardened | Repeated transfers and duplicate names in one offer |
| Text and URL sending/receiving | Implemented | Unicode and URL loopback round trips |
| Wi-Fi credential receiving | Implemented and packaged | Protobuf parsing, hidden flag and review/copy UI; current source has encrypted WPA/Open/WEP loopback and GTK receipt evidence |
| Wi-Fi credential sending | Implemented and packaged | Manual form and Send preview; encrypted WPA/Open/WEP/hidden/rejection checks, installed Nix/Flatpak consent/review/copy checks; physical acceptance unverified |
| Consent / matching code | Implemented and hardened | GTK acceptance plus encrypted pre-consent regression tests |
| Reject / cancel | Implemented | Loopback rejection and cancellation before consent |
| Progress, rate and ETA | Implemented | Acknowledged bytes and moving transfer duration |
| Hidden / visible mode | Implemented | Starts hidden; ten-minute lease; D-Bus smoke test |
| Background / login startup | Implemented; sandbox background tested | Real GNOME portal on/off persistence and window close/reactivation; login startup untested |
| Notifications | Implemented | GNOME service dispatch tested; notification-click and visible banner checks pending |
| File picker / drag and drop | Implemented; sandbox chooser tested | Real GNOME chooser selection, document read grant and matching bytes in a fresh sandbox after GUI exit; Gdk FileList drop target and GApplication opening |
| Receive folder / static port / name | Implemented | Validation, native folder chooser, atomic settings |
| Nautilus integration | Implemented | Optional extension source; installed Files test needed |
| Status notifier tray | Implemented; registration and native actions tested | Live GNOME host registration/icon identity/menu labels/cleanup; native, Nix and Flatpak menu visibility, hide, reopen and Quit checks; host icon visual rendering pending |
| GNOME Shell indicator | Implemented | Separate Nix package; actual Shell testing needed |
| Session D-Bus API | Implemented | Visibility, configured name, listener port, desktop file opening |
| Transfer history / clearing | Implemented | Last 100 outcomes; excludes filenames and payload contents |
| Nix package / NixOS module | Implemented | Reproducible inputs and opt-in firewall rules |
| Flatpak distribution | Development bundle built and tested | GNOME 50; installed sandbox GUI and local incoming transfer; unsigned, x86_64 tested |
| Localization | Gettext infrastructure implemented | UI/CLI/errors/plurals and desktop/AppStream merging; synthetic-catalog layout check; reviewed language catalogs pending |
| Wi-Fi Direct / trusted contacts / cloud relay | Not supported by this engine | Packet is also a partial implementation of Quick Share |
| Empty files | Implemented and packaged | Standalone/mixed encrypted loopback, collision/rejection and installed Nix/Flatpak GTK consent/receipt checks |
| Folders / resume | Not implemented | Rejected early; directory traversal and reconnect/resume work remain |

Local hardening includes constant-time MAC verification, destination validation
inside the engine, exclusive destination creation, bounded byte buffers,
nonnegative sizes, consent gates, and premature completion rejection. This does
not establish that the entire transport has been security-audited.
