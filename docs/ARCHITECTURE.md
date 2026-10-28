# Architecture

`main.rs` owns one AdwApplication and its single window. GApplication handles
single-instance activation, files, and session-bus actions. Widgets stay on the
GLib main thread. `window.rs` builds the three views and owns file selection, discovery presentation,
visibility, and desktop actions. `window/transfers.rs` presents transfers, consent,
queue dispatch, history, and received text. `window/preferences.rs` owns settings,
folder picking, background permission, and optional tray lifecycle.
`window/wifi.rs` owns the manual Wi-Fi draft form and masked credential review.
Wi-Fi drafts and transfer payloads stay in memory; persisted history excludes
SSID/password contents. The engine uses the WifiCredentials schema for the
sensitive byte payload and keeps SSID/security in pre-consent metadata.

`backend.rs` owns the protocol service on a dedicated two-worker Tokio runtime.
Unbounded commands travel from GTK to Tokio; a GLib-compatible async channel
returns discovery and transfer events. The backend starts hidden, keeps the
service alive for receiving, discovers outgoing devices, and forwards only client
messages to GTK. GTK sends consent/cancel commands back through the library's
message channel. After the GTK main loop returns, `main.rs` requests shutdown
and joins the backend thread before the process exits. Shutdown cancels discovery
and the protocol service, allowing tracked requests and incomplete-file guards
to finish teardown. A failed startup uses the same joined shutdown path.
The engine explicitly owns its mDNS daemon threads: normal task exits await
shutdown acknowledgements, while dropped owners request exit as a fallback.
Starting discovery again cancels the previous browser/advertiser first.

`model.rs` validates settings and files, rejects unsafe incoming names, formats
sizes, and persists configuration and bounded history as atomic JSON replacements
under the standard XDG directories. History contains only device name, direction,
size, outcome, and timestamp. Text and Wi-Fi secrets live only in the transfer UI
and explicit clipboard copy; upstream debug logs should not be shared blindly.

A queue freezes the chosen payload when enqueued. Only one outgoing request is
dispatched at a time, matching the engine's serialized outbound connection
handling. Transfer rows retain metadata across sparse progress events and dedupe
terminal notifications/history. New transfers to a previously used endpoint get
fresh rows. Saved history is capped at 100 records; current-session presentation
remains in memory until the app closes or history is cleared.

The optional Shell indicator uses the existing exported GApplication action group.
The optional Nautilus extension launches the app through its desktop file. Neither
starts its own protocol service or gains any authority to approve incoming shares.
The NixOS module adds only requested package/firewall configuration.
