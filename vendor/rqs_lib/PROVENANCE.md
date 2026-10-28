# Protocol engine provenance

Rust source, protobuf schemas, build.rs, and Cargo.toml come from
https://github.com/nozwock/rquickshare at commit
c9ac93f5584a4265e8da679ad689df5be1e80c6b, directory core_lib.
The original GPL-3.0 license is retained. Five protobuf schemas retain Google
Apache-2.0 notices (`LICENSE-APACHE`). `src/proto_src/wire_format.proto` instead
retains the Chromium Authors' BSD notice; its license is included in
`LICENSE-CHROMIUM-BSD`, copied verbatim from Chromium 83.0.4103.61's LICENSE:
https://chromium.googlesource.com/chromium/src/+/83.0.4103.61/LICENSE . The parent application does not claim authorship of
this engine. Bindings, frontend assets, and packaging from upstream are omitted.

Local changes:

- Enforce consent before receiving file or text payloads; reject repeated accepts.
- Validate incoming filenames, sizes, duplicate payload IDs and offer count inside
  the engine before constructing destination paths.
- Use exclusive file creation to refuse overwrite/symlink races after consent.
- Reject negative byte payload sizes, too many buffers, aggregate buffer growth,
  and chunks larger than their advertised payload.
- Use constant-time HMAC verification in both directions.
- Decode signed P-256 coordinates with left padding, positive sign/width checks,
  explicit key-type checks and curve-point validation. Return errors instead of
  panicking on invalid points or missing local handshake state.
- Share authenticated-envelope decryption in both directions. Validate schemes,
  IV/ciphertext lengths, unsupported associated data and every PKCS#7 padding
  byte before protobuf decoding; reject missing receive keys with errors.
- Reject file completion before the advertised size has arrived.
- Reserve distinct destination names for duplicate basenames in one offer.
- Send explicit zero-size FILE completion frames for empty files. Bound source
  reads to the offered size and reject premature EOF if a source is truncated.
- Remove incomplete, exclusively created receive files when a request is dropped
  (including cancellation, connection failure and task abortion). Retain completed
  files, and compare device/inode before unlinking to avoid deleting an already
  replaced path. This is best-effort cleanup, not an atomic pathname-race defense.
- Start requests with encryption disabled until key derivation succeeds, so
  cancelling before the key exchange cannot dereference absent encryption keys.
- Let outgoing requests run independently of the accept loop; report connection
  failures using their transfer IDs and cancel incoming handlers on service stop.
- Bound TCP connection setup to eight seconds and allow service shutdown to
  interrupt it.
- Share a persistent bounded frame reader between send/receive handlers. Preserve
  partial prefixes/bodies across channel actions, allow cancellation during a
  stalled body, and time out incomplete frames after 30 seconds without progress.
- Limit handshakes to 30 seconds and active file idle time to 60 seconds without
  payload progress. Leave human consent waits unbounded, and do not let channel
  traffic or keep-alives extend transport deadlines. Bound frame writes to 30s.
- Limit simultaneous incoming requests to 16. Track request tasks in a JoinSet
  and abort/join them on service stop so socket/file destructors finish first.
- Own the lifetime of mDNS daemon threads explicitly. Stop browsing/advertising
  and await daemon shutdown on normal exit; request exit when an owner is dropped.
  Cancel replaced discovery workers, bound/cancel discovery TCP reachability
  checks, and use bounded asynchronous unregister acknowledgements.
- Add outgoing Wi-Fi credential metadata and sensitive protobuf BYTES payloads.
  Decode received credentials through the schema, preserve hidden-network state,
  validate security-specific keys and redact passwords from payload Debug output.
- Add regressions for consent enforcement, metadata validation, receive cleanup,
  fragmented/cancelled reads, invalid lengths, truncated frames and idle timeouts.
- Omit the upstream diagnostic binary target from workspace builds.
- Apply mechanical Clippy cleanup for current Rust dependency/toolchain versions.
- Remove the child release profile in favor of the root workspace profile.

This is a narrow hardening patch, not a complete transport security audit.
