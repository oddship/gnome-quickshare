# Quick Share protocol research

Research snapshot: 28 October 2026. This is a source-derived interoperability guide,
not Google's complete proprietary specification. Packet explicitly calls its
implementation partial. No public implementation establishes complete parity with
Google's account, contacts, Wi-Fi Direct, and cloud functionality.

## Sources and revision

- [Packet](https://github.com/nozwock/packet): reference GNOME app, GPL-3.0.
- [nozwock/rquickshare](https://github.com/nozwock/rquickshare/tree/c9ac93f5584a4265e8da679ad689df5be1e80c6b/core_lib): base revision of the transport engine used by Packet; this app vendors a hardened copy, GPL-3.0. Upstream research derives from Martichou/rquickshare and NearDrop.
- [NearDrop](https://github.com/grishka/NearDrop): independent reverse engineering.
- [NixOS Update Manager](https://github.com/oddship/nixos-update-manager): native GTK/libadwaita, reproducible Nix packaging, and honest validation status reference. Its source is not incorporated.

The wire schemas are authoritative for field numbers. The local [wire schema
map](WIRE-SCHEMAS.md) links all six schemas and explains their nesting and
state-dependent interpretation.
The application calls the vendored engine, with the local consent, metadata, file-creation, buffer-bound, and MAC-verification patches documented in [PROVENANCE.md](../vendor/rqs_lib/PROVENANCE.md).

## Discovery

The supported medium is the IPv4 LAN. Both peers must have an IP route to one
another; guest isolation, VPN routing, filtered multicast, and firewalls can
prevent discovery or connection. Android may need its Quick Share receive screen
open. Internet access and a Google account are unnecessary for these transfers.

- mDNS/DNS-SD service: `_FC9F5ED42C8A._tcp.local.` on UDP 5353.
- A service instance carries a four-byte random endpoint ID and the endpoint
  information TXT property `n`; the encoded endpoint contains device name/type.
- The receiver advertises a TCP port; zero chooses an ephemeral port. This app
  optionally uses an explicit unprivileged port, useful for a firewall rule.
- The discovery engine resolves IPv4 addresses, rejects its own addresses, probes
  TCP reachability, and reports service removal events to the UI.
- BLE signals that a device wants to share; the experimental Linux implementation
  uses BlueZ and Fast Pair service UUID 0xFE2C. It wakes/encourages Android discovery.
  BLE does not carry file bytes. The engine can continue its LAN service if BLE
  fails, so service readiness alone does not prove a usable Bluetooth adapter.
- Visibility affects receive advertisements; outgoing discovery can operate while
  this computer is hidden. This app starts hidden and grants ten-minute windows.

## Connection and authentication

The sender opens TCP to the discovered address and port. Messages use protobuf
schemas above and a length-prefixed stream. Connection setup uses Nearby
Connections offline frames, followed by UKEY2 client-init, server-init, and
client-finish messages. The implementation negotiates P-256 ECDH and
`AES_256_CBC-HMAC_SHA256`, verifies the key-exchange commitment, derives directional
encryption/authentication keys using HKDF-SHA256 and protocol-defined labels and
salts, and derives a four-digit human authentication code. The key decoder checks
the EC_P256 type, positive signed coordinate widths and curve membership; shorter
positive encodings are padded to the 32-byte SEC1 coordinate width.

After UKEY2, DeviceToDevice messages carry per-direction sequence numbers and
SecureMessage envelopes. AES-256-CBC encrypts and HMAC-SHA256 authenticates them;
sequence checks reject unexpected order. The shared receive decoder authenticates
the raw envelope before parsing it, requires the negotiated schemes and a 16-byte
IV, and checks ciphertext alignment and every PKCS#7 padding byte. A matching code
is the user's check
against an active intermediary; encryption alone does not authenticate a named
person or a Google account. Paired-key encryption/result frames are exchanged,
but this engine reports that trusted account pairing is unavailable.

GNOME Quickshare displays the code before consent, disables acceptance without a
code, defaults dialogs to Decline, and never accepts automatically. An outgoing
transfer displays its code in Activity for comparison on the receiver.

## Sharing frames and consent

An Introduction frame describes payloads before bytes are delivered:

- Files: name, ID, MIME/type metadata, declared size, and payload ID.
- Text: type, preview/title, size, and payload ID. A URL is a text subtype.
- Wi-Fi: SSID, security type, and a separate password payload.

The receiver responds with Accept or Reject. The application exposes both
responses and cancellation through the library's broadcast channel. Incoming
metadata is retained per connection because progress/terminal messages may omit
it. Offers with absolute paths, path separators, dot paths, or NULs in filenames
are rejected before consent. Files are never launched automatically. Text, URLs,
and Wi-Fi credentials are shown in a review dialog with explicit Copy.

## Payloads and completion

Encrypted Nearby payload-transfer frames identify a payload and byte offsets;
file chunks are written to the configured receive directory. Progress is
acknowledged bytes divided by total advertised bytes. The engine
resolves existing and in-offer filenames by adding a numerical suffix. Completion,
cancellation, rejection, and disconnection are distinct states. TCP keep-alive
and disconnection frames close the exchange. Current source sends an empty file
as a zero-size FILE payload with an empty LAST_CHUNK at offset zero, after consent.
It works alone or alongside nonempty files. Directories remain unsupported and
are rejected early. Physical Android/Windows acceptance of zero-byte offers is
not established by same-engine loopback.
Current source extends the upstream engine with Wi-Fi credential sending. An
Introduction carries `WifiCredentialsMetadata` (SSID, security type and payload
ID); the accepted transfer sends a sensitive BYTES payload containing the schema's
`WifiCredentials` protobuf (password field 1, hidden-SSID flag field 2), followed
by LAST_CHUNK. Receiving uses the same protobuf decoder instead of assumptions
about byte positions. Malformed credentials fail the transfer rather than
silently becoming an empty password. Open, WPA-PSK and WEP types are supported;
unknown types and invalid network names are rejected. Sending validates SSID
length and security-specific key lengths before an offer is made.

The manual Share Wi-Fi dialog prepares a masked-password draft and then a Send
preview; selecting a nearby device starts the matching-code/consent flow. No
NetworkManager secrets are read, and receiving does not connect automatically.
SSID/security metadata precede consent; password bytes follow acceptance.
Persisted history excludes credentials. Wi-Fi payload Debug implementations redact
passwords and parse errors omit credential bytes. Physical Android/Windows
credential interoperability remains unverified. The refreshed 41-test Nix and
Flatpak development packages include this feature.

When a receive request is torn down, the engine removes files it created that
have not received a valid completion marker. It keeps files completed earlier
in a multi-file offer. Cleanup compares the open file's device/inode with the
current destination and skips replaced paths and symlinks. Cleanup is best-effort:
permission failures, process termination without destructors, and a concurrent
pathname replacement after the metadata check remain outside this guarantee.

## Transport framing

Both directions retain partial four-byte length prefixes and frame bodies across
channel actions. Frames must be nonempty and no larger than 5 MiB before buffer
allocation. A partially received prefix/body has a 30-second idle timeout;
unrelated channel traffic does not extend it. The entire handshake has a
30-second deadline. Receiving/sending states have a 60-second idle budget that
only increasing payload progress resets; keep-alives do not extend it. Waiting
for local or remote human consent has no application deadline. Frame writes and
flushes have a 30-second timeout. Cancellation can interrupt a stalled body read;
a cancellation queued during a blocked write may wait for its timeout.

The listener admits at most 16 incoming requests concurrently and closes excess
connections. Outgoing requests run independently. Service shutdown aborts and
joins request tasks, closing sockets and running partial-file cleanup before
returning. These bounds do not constitute a complete resource/security audit;
OS scheduling, synchronous filesystem operations and discovery tasks have their
own behavior.

## State mapping

The engine exposes Initial, connection request, UKEY2 and paired-key states,
Introduction, WaitingForUserConsent, ReceivingFiles/SendingFiles, Finished,
Rejected, Cancelled, and Disconnected. The UI groups the handshake as Connecting,
shows consent and byte progress explicitly, and preserves terminal outcomes.
Outgoing connection setup failures report a terminal event under the transfer
ID. Outgoing handlers run independently of the incoming accept loop, and service
shutdown cancels both directions. TCP connection setup has an eight-second bound.
Encryption starts disabled until key derivation succeeds; early cancellation
uses a plaintext disconnect. Some upstream task failures can still appear only
in diagnostic logs; a running listener is not an end-to-end health assertion.

## Boundaries of the implementation

Not implemented: Google account identity/trusted devices, contacts-only mode,
Wi-Fi Direct/hotspot negotiation, WebRTC/relay, Bluetooth file transport,
ChromeOS-specific integrations and resume after disconnection. Compatibility with Android/Windows vendor versions needs physical
hardware testing. No such tests have been performed during initial development.

The engine has local regression-tested hardening for pre-consent payloads, unsafe metadata, exclusive destination creation, negative sizes, buffer growth, early file completion, and constant-time MAC checks. It is not a security-audited implementation. It parses and allocates
untrusted network messages before application consent; UI filename checks do
not replace limits and validation inside the transport. Release requires a
transport-focused audit (length limits, resource exhaustion, symbolic-link and
collision races, and cancellation/cleanup of incomplete files).
