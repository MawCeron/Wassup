# IPC between the daemon and the UI

The daemon and the UI are separate processes. They talk over a Unix
socket using Protocol Buffers. The schema is
[`proto/ipc.proto`](../proto/ipc.proto); this document covers what the
schema cannot express.

## Socket

- **Path:** `$XDG_RUNTIME_DIR/wassup.sock`. If `XDG_RUNTIME_DIR` is not
  set, both sides fail instead of falling back to `/tmp`, which other
  users share.
- **Permissions:** the socket is `0600`, inside a runtime directory that
  is `0700` by the XDG spec. Only the user who started the daemon can
  connect.
- **Startup:** the daemon first tries to connect to the path. If another
  daemon answers, it refuses to start. Otherwise any file at the path is
  left over from a crash and is replaced.

## Framing

Each frame is one Protobuf message preceded by its length in bytes,
encoded as a varint. This is the standard Protobuf length-delimited
format (`protodelim` in Go, `encode_length_delimited` in prost).

Frames larger than 4 MiB are rejected on both sides, so a corrupt length
cannot make a reader allocate unbounded memory. Media never travels over
the socket; the daemon sends a file path instead.

## Messages

Each direction has a single envelope, so a reader always knows the type
of the next frame:

| Direction | Envelope |
|---|---|
| UI → daemon | `Request` |
| daemon → UI | `Event` |

The concrete content is chosen with a `oneof` inside the envelope.

- Every `Request` carries an `id` chosen by the UI. It must not be `0`.
- The daemon answers each request with one `Event` whose `request_id`
  equals that `id`. Failures are answered with an `error` event.
- Events the daemon sends on its own, such as a received message or a
  connection change, have `request_id` set to `0`.
- Replies may arrive interleaved with unsolicited events, so the UI pairs
  them by `request_id`, not by order.
