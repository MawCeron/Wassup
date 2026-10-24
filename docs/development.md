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

Run the UI with `cargo run` while the daemon is running. The app is
single-instance (`io.github.MawCeron.Wassup`): launching it again only
brings the existing window to the front. An unlinked account shows the
QR code to link it.

Closing the window ends the UI process but not the daemon, which keeps
receiving messages. The window size and the open chat are saved to
`$XDG_STATE_HOME/wassup/ui.ini` and restored on the next launch.

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

## Configuration

The daemon reads `$XDG_CONFIG_HOME/wassup/config.json` at startup. The
file is optional; every option defaults to off.

```json
{
  "hide_notification_content": true
}
```

| Option | Effect |
|---|---|
| `hide_notification_content` | Notifications show the chat but not the message text. |

An unknown option or a syntax error stops the daemon instead of being
ignored, so a misspelled privacy option cannot silently turn itself off.

Notifications go through `org.freedesktop.Notifications` on the session
bus. The chat open in a focused UI window is not notified, and neither
are messages sent while the daemon was offline: they only raise the
chat's unread count. Muted chats do not notify, each chat keeps a single notification
that newer messages replace, and reading the chat withdraws it.

## Desktop integration

Clicking a notification opens its chat. The daemon asks the UI to do it
through D-Bus (`org.freedesktop.Application.ActivateAction`, action
`open-chat`), which also starts the UI when it is closed. That needs the
files in `packaging/` installed; for development, point them at the
debug build:

```sh
mkdir -p ~/.local/share/dbus-1/services ~/.local/share/applications
sed "s|/usr/bin/wassup|$PWD/ui/target/debug/wassup|" \
  packaging/io.github.MawCeron.Wassup.service \
  > ~/.local/share/dbus-1/services/io.github.MawCeron.Wassup.service
sed "s|^Exec=wassup|Exec=$PWD/ui/target/debug/wassup|" \
  packaging/io.github.MawCeron.Wassup.desktop \
  > ~/.local/share/applications/io.github.MawCeron.Wassup.desktop
```

`wassup --toggle` shows the window, or closes it if it has focus. On
Wayland, bind it to a key in Settings → Keyboard → Custom Shortcuts.

## Running the daemon as a service

`packaging/wassupd.service` is a systemd user unit that starts the daemon
at login and restarts it if it fails. Without network at login, the
daemon keeps retrying in the background. For development, install it
against a local build:

```sh
go -C daemon build -o ~/.local/bin/wassupd .
mkdir -p ~/.config/systemd/user
sed "s|/usr/bin/wassupd|$HOME/.local/bin/wassupd|" packaging/wassupd.service \
  > ~/.config/systemd/user/wassupd.service
systemctl --user daemon-reload
systemctl --user enable --now wassupd
journalctl --user -u wassupd -f   # logs
```

## Media

Images and stickers are downloaded on demand: the daemon stores what it
needs to fetch them, and the UI asks for a file only when its message is
on screen. Files are cached in `$XDG_CACHE_HOME/wassup/media/<account>/`,
named after their content hash, and the least recently used ones are
deleted once the cache passes 500 MB; they download again if shown.

The UI decodes images already scaled to their on-screen size, through
gdk-pixbuf, which on Fedora runs glycin's sandboxed loaders. Clicking an
image opens the cached file in the default image viewer.
