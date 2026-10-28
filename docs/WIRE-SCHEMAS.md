# Wire schema map

This map describes the pinned, source-derived LAN protocol implemented here.
It accompanies [the protocol guide](PROTOCOL.md); it does not define Google's
account, cloud, or Wi-Fi Direct protocols. The linked proto2 files retain the
actual field numbers, enum values, comments and upstream notices.

| Layer | Schema | Relevant messages |
|---|---|---|
| Nearby connection transport | [offline_wire_formats.proto](../vendor/rqs_lib/src/proto_src/offline_wire_formats.proto) | OfflineFrame, V1Frame, ConnectionRequestFrame, ConnectionResponseFrame, PayloadTransferFrame, KeepAliveFrame, DisconnectionFrame |
| Key exchange | [ukey.proto](../vendor/rqs_lib/src/proto_src/ukey.proto) | Ukey2Message, Ukey2ClientInit, Ukey2ServerInit, Ukey2ClientFinished, Ukey2Alert |
| Authenticated envelope | [securemessage.proto](../vendor/rqs_lib/src/proto_src/securemessage.proto) | SecureMessage, HeaderAndBody, Header, GenericPublicKey, EcP256PublicKey |
| Encrypted message body | [device_to_device_messages.proto](../vendor/rqs_lib/src/proto_src/device_to_device_messages.proto) | DeviceToDeviceMessage and its sequence number |
| Envelope metadata | [securegcm.proto](../vendor/rqs_lib/src/proto_src/securegcm.proto) | GcmMetadata with DEVICE_TO_DEVICE_MESSAGE type |
| Sharing application | [wire_format.proto](../vendor/rqs_lib/src/proto_src/wire_format.proto) | Frame, V1Frame, IntroductionFrame, ConnectionResponseFrame, PairedKeyEncryptionFrame, PairedKeyResultFrame, WifiCredentials |

## Decoding a captured stream

Each TCP message begins with a four-byte unsigned big-endian length, excluding
the prefix itself. TCP packet boundaries do not delimit protocol messages.
The current handlers use a bounded persistent reader before protobuf decoding.
The initial plaintext frames are interpreted according to the handshake state:
Nearby connection request/response and UKEY2 envelopes are different schemas.

After key establishment, the outer message is SecureMessage. Its signature
covers the encoded header_and_body bytes. The engine verifies that signature
with the receive HMAC key before decoding HeaderAndBody and decrypting its body.
The plaintext decodes as DeviceToDeviceMessage; its message field contains an
OfflineFrame and its sequence number must match the next expected value.
See the two request handlers for the exact state-dependent interpretation:
[inbound](../vendor/rqs_lib/src/hdl/inbound.rs) and
[outbound](../vendor/rqs_lib/src/hdl/outbound.rs).

A PAYLOAD_TRANSFER OfflineFrame identifies BYTES, FILE or STREAM payloads.
BYTES payloads also carry the sharing application's serialized Frame messages;
not every BYTES payload is user text. Introduction metadata maps attachment IDs
to later text/file/Wi-Fi payloads. The sharing ConnectionResponseFrame is the
user's consent response and is distinct from the transport connection response.

## File payload fields

PayloadTransferFrame uses packet_type=1, payload_header=2, payload_chunk=3 and
control_message=4. PayloadHeader has id=1, type=2 and total_size=3. PayloadChunk
has flags=1, offset=2 and body=3; LAST_CHUNK is flag bit 0x1. These are protobuf
field numbers, not byte offsets in an encoded packet.

The receiver requires the offset to equal the bytes already written, bounds the
chunk by the approved file size, and accepts the empty final chunk only after
all declared bytes arrive. A completed file leaves the pending-file map.
Request teardown removes still-pending files on a best-effort basis.

The schema contains fields and transports that this engine does not implement.
Their presence is not evidence of support: STREAM payloads, transport upgrades,
certificate/account handling and other schema messages need implementation and
independent interoperability evidence before being advertised as features.
