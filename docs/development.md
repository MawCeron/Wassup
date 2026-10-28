# Development

## Requirements

Fedora 44 packages:

```sh
sudo dnf install golang rust cargo rustfmt clippy protobuf-compiler gtk4-devel libadwaita-devel libwebp-devel fontconfig-devel
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

Preferences (Ctrl+,) edits these options. The daemon owns
`$XDG_CONFIG_HOME/wassup/config.json`: it reads the file at startup, and
saves it and applies the change at once when the UI sends new settings,
except GPU rendering, which the UI reads only when it starts. The file is
optional; a missing option is off, except `media_cache_mb`, which is 500, and
`language` and `chat_background`, which are empty.

```json
{
  "hide_notification_content": true,
  "media_cache_mb": 500,
  "gpu_rendering": false,
  "hide_tray_icon": false,
  "no_link_previews": false,
  "chat_background": "",
  "language": "es"
}
```

| Option | Effect |
|---|---|
| `hide_notification_content` | Notifications show the chat but not the message text. |
| `media_cache_mb` | Size of the media cache; the least recently used files go first. Default 500. |
| `gpu_rendering` | The UI draws with the GPU instead of in software. Off by default; see below. |
| `hide_tray_icon` | Turns off the status icon with the unread count. |
| `no_link_previews` | Sends links without a preview, so the daemon never visits them. |
| `chat_background` | `""` for the doodles, `"plain"`, or `"picture"` for the user's own, kept scaled to at most 1920 px in `$XDG_DATA_HOME/wassup/background/`. |
| `language` | Language code such as `es` or `pt_BR`; empty or missing follows the system. The daemon switches at once, the UI at its next start. |

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
daemon keeps retrying in the background. `packaging/wassupd.socket`
listens on its socket, so that opening Wassup starts the daemon again
after the user quit with Ctrl+Q: quitting exits cleanly, which systemd
does not restart. A package installed during the session leaves the
socket stopped until the next login, so the UI asks systemd to start it with
`systemctl --user start wassupd.socket` the first time it cannot connect. For development, install both against a local build:

```sh
go -C daemon build -o ~/.local/bin/wassupd .
mkdir -p ~/.config/systemd/user
sed "s|/usr/bin/wassupd|$HOME/.local/bin/wassupd|" packaging/wassupd.service \
  > ~/.config/systemd/user/wassupd.service
cp packaging/wassupd.socket ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable wassupd.socket wassupd
systemctl --user stop wassupd && systemctl --user start wassupd.socket wassupd
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
gdk-pixbuf, which on Fedora runs glycin's sandboxed loaders. Clicking a
photo or a video opens it in a viewer over the window, which steps through
the chat's photos and videos; videos play through GTK's media support.

Audio and voice notes download when their play button is pressed, and
play through GTK's media support, which uses GStreamer. One plays at a
time, and the player only exists while it does.

Profile and group pictures are fetched when a chat first shows up on
screen, as small previews cached in `$XDG_CACHE_HOME/wassup/avatars/`.
The daemon asks the server again only after a picture change event or a
day, and remembers when a chat has no picture, since frequent picture
requests are among the easiest ways for an unofficial client to get rate
limited. Until a picture arrives, or when there is none, the avatar shows
the contact's initials or an icon.

To send a file, the UI passes the daemon its path, never its content.
Choosing a file shows a preview with a caption field, and nothing is sent
until the user presses Send. JPEG and PNG files go as photos, MP4 files as
videos and anything else as a document, with the caption as text. The
attach menu can send a photo or a video as a document instead. The daemon
uploads through a temporary file, so large files are not held in memory, and
replies when the upload ends. Photos are sent as they are, without being
recompressed.

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

Clicking a sticker in a conversation opens a menu to add it to the favorites,
or to take it out if it is one. The daemon writes the same `favoriteSticker`
action the phone does, with the file's hash in base64 as its index and what
is needed to download the file again (`SetFavoriteSticker` in
`daemon/sticker.go`), and then records the change itself, since the server
does not send it back. The action's version is 2, which the phone accepted
when it was tried; it was not read from a mutation of the phone.

## History

When a device is linked, the phone sends the last three months of every
chat (`historyDays` in `daemon/history.go`), which the daemon stores
without notifying or counting them as unread; the phone's own unread
counts are kept instead. Older messages come on demand: when the user
scrolls past the oldest stored message, the UI asks the daemon, which asks
the phone for the 50 before it. The phone must be online. A device linked
before this existed did not keep its first history; scrolling up loads it
chat by chat, and linking again brings it all at once.

## Voice notes

The UI records with `gst-launch-1.0` (package `gstreamer1`) in its own
process: the default microphone through `autoaudiosrc`, encoded as Opus
in Ogg (`opusenc`, from `gstreamer1-plugins-base`), which is what WhatsApp
plays. Stopping sends it `SIGINT`, so `-e` closes the file properly. It
runs under `setpriv --pdeathsig KILL` (util-linux), so the kernel ends the
recording if the UI dies: the microphone never stays open after Wassup.
Recordings under a second are dropped. The daemon sends the file as a voice
note, with its length read from the last Ogg page.

## Link previews

Received links show the preview the sender's phone attached to the
message, so showing them needs no network. For links the user sends, the
daemon fetches the page before sending, as the phone does: its Open Graph
title, description and image, or its `<title>` and description. The fetch
is capped at 5 seconds, 512 KB of page and a 5 MB image of at most 8
megapixels, shrunk to a 200 px JPEG. Texts go out in the order they were
written even while a preview is fetched. If anything fails the text goes
without a preview. Preferences can turn this off.

## Emoji

The composer's emoji button opens GTK's own `GtkEmojiChooser`. Two things
keep its memory in check; the measurements are in
[`performance.md`](performance.md#emoji-picker).

- **Created on first use.** `GtkEmojiChooser` builds a widget for every
  emoji when it is created, so the button creates it the first time it is
  pressed (`set_create_popup_func` in `ui/src/window.rs`), not with the
  composer.
- **A bitmap emoji font instead of Noto Color Emoji.** Drawing the emoji of
  Noto Color Emoji fills a cache in cairo that reaches hundreds of
  megabytes and is never freed, whether the emoji are in the picker or in
  a message. Wassup carries
  [Twemoji CBDT](https://github.com/MawCeron/twemoji-cbdt), a bitmap font
  built from the current Twemoji artwork, in `wassup/fonts/` under the
  data directories (`/usr/share` once installed, `~/.local/share` to try
  it or to replace it). `ui/src/emoji_font.rs` adds it to the UI's
  fontconfig and takes the other emoji fonts out: both Noto ones, since
  without the color one Pango falls back to the monochrome one, other
  Twemoji fonts, so that Pango cannot pick them, and Symbola, a symbol font
  many systems have, which Pango uses to draw the emoji GTK measures to
  size the others, so the picker then drops the wide ones, such as flags.
  The font is private to the UI process: nothing is installed in the
  system's font directories, and the rest of the system is unchanged.
  Without the file, nothing changes and the emoji use Noto Color Emoji. It
  runs at the start of `main`, before GTK uses fonts.

Pango picks the font for emoji characters itself, so a `font-family` in
CSS does not change it.

The picker's size is adjusted in two places. libadwaita and GTK both scale
the emoji, which makes them about twice the text size; the CSS in
`ui/src/window.rs` sets the font size on the label inside each emoji
(`popover.emoji-picker emoji label`), because the same property on the
`emoji` node has no effect. GTK shows 7 emoji per row and CSS cannot
change that, so `set_emoji_columns` sets `max-children-per-line` on each
section's flow box to `EMOJI_COLUMNS`, 10.

To run a build from the repository with the font, download
`TwemojiCBDT.ttf` from a [release of twemoji-cbdt](https://github.com/MawCeron/twemoji-cbdt/releases)
and copy it into place:

```sh
mkdir -p ~/.local/share/wassup/fonts
cp TwemojiCBDT.ttf ~/.local/share/wassup/fonts/
```

The font covers Emoji 17: with it the picker shows all 1910 emoji, and the
UI goes from 37.6 MB to 81.6 MB once the picker has been opened.

A font built from the same artwork with
[twemoji-colr](https://github.com/mozilla/twemoji-colr) is a COLR font
whose base glyphs have no outline, and with it the picker shows no emoji:
GTK measures the ink width of one emoji to decide which ones to keep, and
for such a font it is 0.

## Status icon

The daemon shows the unread count as a StatusNotifierItem, the tray
protocol most desktops support. GNOME needs the AppIndicator extension
(`gnome-shell-extension-appindicator` in Fedora). Without a tray the
icon simply does not show; it appears once the extension is enabled.
Clicking it opens Wassup. The count leaves out muted chats and locked
ones, so the icon does not give locked chats away.

To test it without a tray, run the daemon on a private bus with a small
`org.kde.StatusNotifierWatcher` stand-in, and point every XDG directory of
the daemon at temporary ones so it does not open the real account.

## Linked device name

The phone lists Wassup under Linked devices as "Wassup" with the desktop
icon; WhatsApp does not allow a custom one. The name is sent only when
linking, so a device linked by an older build keeps its old name until it
is unlinked and linked again.

## Keyboard and screen readers

| Key | Where | Action |
|---|---|---|
| Arrow keys | Chat list | Open each chat while the focus stays in the list |
| Enter | Chat list | Move to the message box |
| Enter / Shift+Enter | Message box | Send / new line |
| Escape | Message box | Back to the chat list |
| Tab | Conversation | Reach photos, documents and audio controls, which open with Enter or Space |
| Ctrl+F | Anywhere | Search chats by name, ignoring case and accents |
| Ctrl+Shift+F | Open chat | Search its messages, ignoring case and accents; choosing a result jumps to it |
| Right click or long press | Message | Menu to react or reply; Escape cancels a reply |
| Ctrl+, | Anywhere | Preferences |
| Ctrl+? | Anywhere | Keyboard shortcuts |
| Ctrl+W | Anywhere | Close the window; the daemon keeps notifying |
| Ctrl+Q | Anywhere | Quit after confirming, stopping the daemon and notifications until Wassup is opened again |

Every control has a name for screen readers, and a chat row reads its
unread count. Messages that arrive in the open chat are announced. To
check names, list the window's accessibility tree over AT-SPI, for
example with Accerciser.

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

## Screenshots

The screenshots in `data/screenshots/`, used by the README and the
AppStream metadata, show the real UI against an invented account, so no
real chat appears in them. `daemon/cmd/demodaemon` answers the UI with that
account, and `packaging/screenshots/screenshots.sh` runs both in a virtual X
server and captures the window in the light and dark styles. It needs
`Xvfb`, ImageMagick, `python3-pillow` and the emoji font:

```sh
sudo dnf install xorg-x11-server-Xvfb ImageMagick python3-pillow
packaging/screenshots/screenshots.sh
```

To change what the screenshots show, edit the messages in
`daemon/cmd/demodaemon/main.go`.
