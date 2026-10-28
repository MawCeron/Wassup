<p align="center">
  <img src="data/brand/wassup-lockup.svg" alt="Wassup" height="96">
</p>

<p align="center"><em>Unofficial native WhatsApp client for Linux.</em></p>

Wassup is an **unofficial** WhatsApp client for GNOME. It speaks the protocol
natively, without a web view, so it stays light on memory: about 64 MB for the
daemon and the UI together, against over 1 GB for the clients that wrap
WhatsApp Web.

> [!CAUTION]
> Since this is an unofficial client, Meta could restrict accounts that use it. Try it with a secondary number first.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="data/screenshots/chat-dark.png">
    <img src="data/screenshots/chat-light.png" alt="A group chat in Wassup with a photo, a link preview, a reply and a mention" width="800">
  </picture>
</p>

## Features

- Links your account by QR code, as another device of your phone, and brings
  in your recent history; older messages load as you scroll up.
- Sends and receives text, photos, videos, documents, stickers, GIFs and voice
  notes, with replies, reactions, link previews and mentions.
- Plays animated stickers and GIFs, and keeps your favorite stickers in sync
  with the phone.
- Shows delivery and read marks, unread counts, and profile and group
  pictures; keeps disappearing messages and locked chats as the phone does.
- Searches chats, and messages inside a chat.
- Notifies through the desktop even with the window closed, can hide the
  message text, and shows the unread count as a status icon.
- Follows the GNOME guidelines, works with the keyboard and screen readers,
  and is translated (Spanish so far).

It supports one account. What comes next is in
[`docs/roadmap.md`](docs/roadmap.md).

## Installation

> [!NOTE]
> Early development: the first release is an RPM for Fedora 44. Other
> distributions build it from source.

Download the RPM from the
[releases page](https://github.com/MawCeron/Wassup/releases) and install it:

```sh
sudo dnf install ./wassup-*.x86_64.rpm
```

Open Wassup from the application launcher and scan the QR code with your
phone, as when linking any other device.

To build it yourself, see [`docs/development.md`](docs/development.md) for
the daemon and the UI, and [`docs/packaging.md`](docs/packaging.md) for the
RPM.

## Usage

Closing the window leaves the daemon running, so notifications keep arriving.
`Ctrl+Q` quits after confirming and stops both until you open Wassup again.
`Ctrl+,` opens Preferences and `Ctrl+?` lists the keyboard shortcuts.

`wassup --toggle` shows the window, or closes it if it has focus. On Wayland,
bind it to a key in Settings, Keyboard, Custom Shortcuts.

## Troubleshooting

If the window says it can't reach the Wassup service, start the daemon with
`systemctl --user start wassupd.socket`. Its log is in
`journalctl --user -u wassupd`.

## How it works

Two processes talk over a Unix socket with Protocol Buffers:

- **`daemon/`**, in Go, uses [whatsmeow](https://github.com/tulir/whatsmeow)
  for the protocol and SQLite for storage. It runs as a systemd user service,
  so it keeps receiving messages when the UI is closed.
- **`ui/`**, in Rust, is a GTK 4 and libadwaita window that draws in software
  by default to save memory.

## Memory

With a real account of 29 chats and about 3,500 messages and no chat open, the
daemon and the UI together use about 64 MB (PSS), against 384 MB for
Whats4Linux and 1.3 GB for Karere on the same account. The accounts differ in
history, so read it as an order of magnitude. Emoji are drawn with a bitmap
font that the package carries, because the default emoji font makes the UI
grow by hundreds of megabytes once emoji are shown. Method, caveats and every
figure are in [`docs/performance.md`](docs/performance.md).

## Documentation

- [Roadmap](docs/roadmap.md): what is done and what comes next.
- [Development](docs/development.md): setup, configuration and how media works.
- [Packaging](docs/packaging.md): building the RPM.
- [Translating](docs/translating.md)
- [IPC](docs/ipc.md): the protocol between the daemon and the UI.
- [Performance](docs/performance.md): memory measurements.
- [Visual identity](docs/visual-identity.md): icons and look.
- [Contributing](CONTRIBUTING.md) and the [changelog](CHANGELOG.md).
