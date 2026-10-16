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
| Format | `gofmt -w .` |
| Check formatting | `test -z "$(gofmt -l .)"` |
| Static analysis | `go vet ./...` |

## UI (Rust)

Run from `ui/`:

| Task | Command |
|---|---|
| Build | `cargo build` |
| Format | `cargo fmt` |
| Check formatting | `cargo fmt --check` |
| Static analysis | `cargo clippy -- -D warnings` |

Formatting and lint rules are the tools' defaults; no config files.
