# Development

## Requirements

Fedora 44 packages:

```sh
sudo dnf install golang rust cargo rustfmt clippy protobuf-compiler gtk4-devel libadwaita-devel
```

Versions in use: Go 1.26, Rust 1.99, protoc 3.19, GTK 4.22, libadwaita 1.9.
The Fedora toolchains are used instead of `rustup` so development matches
what the RPM is built with.

## Daemon (Go)

Run from `daemon/`:

| Task | Command |
|---|---|
| Build | `go build ./...` |
| Run with request logging | `go run . -debug` |
| Format | `gofmt -w .` |
| Check formatting | `test -z "$(gofmt -l .)"` |
| Static analysis | `go vet ./...` |
| Tests | `go test -race ./...` |

## UI (Rust)

Run from `ui/`:

| Task | Command |
|---|---|
| Build | `cargo build` |
| Format | `cargo fmt` |
| Check formatting | `cargo fmt --check` |
| Static analysis | `cargo clippy --all-targets -- -D warnings` |
| Tests | `cargo test` |
| Tests against the Go daemon | `cargo test -- --include-ignored` |

Formatting and lint rules are the tools' defaults; no config files.

## IPC code generation

`proto/ipc.proto` is the single source for both sides.

- **Go:** run `go generate ./...` from `daemon/` after changing the schema,
  and commit `daemon/ipc/ipc.pb.go`. `protoc-gen-go` is pinned as a tool in
  `daemon/go.mod`, so no global install is needed.
- **Rust:** `ui/build.rs` regenerates the types on every `cargo build`
  that sees a schema change; nothing is committed. It needs `protoc` in
  `PATH`.

## Test client

`wassupctl` talks to a running daemon over IPC, so an account can be
linked and used before the UI exists. Run from `daemon/`:

```sh
go run . -debug                            # terminal 1: the daemon
go run ./cmd/wassupctl accounts            # terminal 2: note the account id
go run ./cmd/wassupctl link <account>      # scan the QR with the phone
go run ./cmd/wassupctl watch               # print incoming messages
go run ./cmd/wassupctl send <account> 5215550000000 hello
go run ./cmd/wassupctl chats <account>
go run ./cmd/wassupctl messages <account> 5215550000000
```

`link` draws the QR in the terminal when `qrencode` is installed
(`sudo dnf install qrencode`); otherwise it prints the text to encode.

Use a secondary number: this is an unofficial client.
