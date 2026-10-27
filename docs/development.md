# Development

## Requirements

Fedora 44 packages:

```sh
sudo dnf install golang rust cargo rustfmt clippy protobuf-compiler gtk4-devel libadwaita-devel libwebp-devel
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
receiving messages. If the daemon restarts, for example after an update, the
UI reconnects on its own and reloads the open chat; there is no need to
close it, let alone to unlink the device. The window size and the open chat are saved to
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
linked and used without the UI, for example to debug the daemon. Run
from `daemon/`:

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
  "hide_notification_content": true,
  "media_cache_mb": 500,
  "gpu_rendering": false
}
```

| Option | Effect |
|---|---|
| `hide_notification_content` | Notifications show the chat but not the message text. |
| `media_cache_mb` | Size of the media cache; the least recently used files go first. Default 500. |
| `gpu_rendering` | The UI draws with the GPU instead of in software. Off by default; see below. |

An unknown option or a syntax error stops the daemon instead of being
ignored, so a misspelled privacy option cannot silently turn itself off.

The UI draws in software by default: GPU rendering loads the graphics
drivers into its process, over 50 MB on some machines, and software
rendering keeps scrolling at 60 fps, at the cost of more CPU while
scrolling. `gpu_rendering` switches back, from the next time the UI
starts. Setting `GSK_RENDERER` or `GDK_DISABLE` in the environment also
overrides the default. Figures are in [`performance.md`](performance.md).

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
on screen. Documents are downloaded when clicked, then opened with the
default application. Files are cached in `$XDG_CACHE_HOME/wassup/media/<account>/`,
named after their content hash, and the least recently used ones are
deleted once the cache passes `media_cache_mb` (500 MB by default); they
download again if shown. Temporary files left by an interrupted download
or upload are removed when the daemon or the UI starts.

The UI decodes images already scaled to their on-screen size, through
gdk-pixbuf, which on Fedora runs glycin's sandboxed loaders. Clicking an
image opens the cached file in the default image viewer.

Audio and voice notes download when their play button is pressed, and
play through GTK's media support, which uses GStreamer. One plays at a
time, and the player only exists while it does.

## Linked device name

The phone lists Wassup under Linked devices as "Wassup" with the desktop
icon; WhatsApp does not allow a custom one. The name is sent only when
linking, so a device linked by an older build keeps its old name until it
is unlinked and linked again.

## Locked chats

Chats locked on the phone stay out of the chat list, unread counts
included, behind a "Restricted chats" entry that asks for the secret code
set on the phone. The phone shares only a salted PBKDF2 hash of the code;
the daemon checks what the user types against it and pauses a second
after a wrong code. Going back locks the chats again and closes an open
one. Notifications for locked chats only say there is a new message.

This is a screen against people looking over your shoulder, as in
WhatsApp Web: the messages are stored like any others, so it does not
protect them from someone with access to your user account.

## Disappearing messages

Chats with disappearing messages are respected. Each message keeps the
timer it was sent with, and once that time has passed since it was sent,
the daemon deletes it and its cached photo, document or audio; it checks
at startup and then every minute, and open UIs drop the messages too.
Stickers stay cached, since other chats and favorites share them. Own
messages are sent with the chat's timer. Messages stored before Wassup
learned about timers kept none and do not expire.

Profile and group pictures are fetched when a chat first shows up on
screen, as small previews cached in `$XDG_CACHE_HOME/wassup/avatars/`.
The daemon asks the server again only after a picture change event or a
day, and remembers when a chat has no picture, since frequent picture
requests are among the easiest ways for an unofficial client to get rate
limited. Until a picture arrives, or when there is none, the avatar shows
the contact's initials or an icon.

To send a file, the UI passes the daemon its path, never its content.
JPEG and PNG files go as photos, anything else as a document, with the
composer's text as caption. The daemon uploads through a temporary file,
so large files are not held in memory, and replies when the upload ends.
Photos are sent as they are, without being recompressed.

The sticker button offers your favorite stickers and the ones you sent
recently, which are sent again by reusing the file already on WhatsApp's servers, so
nothing is uploaded. It can also turn an image into a sticker: the UI
centres it on a transparent 512x512 canvas and encodes it as WebP with
the system's libwebp, lowering the quality until it fits WhatsApp's
100 KB limit for stickers.

Favorite stickers come from the phone through WhatsApp's app state. The
daemon replays the whole app state once to learn the existing ones, then
follows changes. The sync carries each favorite's file hash as its id,
which is what whatsmeow needs to verify the download. Favorites whose
file has expired on WhatsApp's servers, or that only the phone holds,
cannot be fetched and are left out, unless the same sticker was also
received in a chat.
