# Wassup

An **unofficial** WhatsApp client for Linux (GNOME), with a native protocol
implementation and low RAM usage.

> Early development: there are no packages yet, and it is built from source.

**Warning:** since this is an unofficial client, Meta could restrict accounts that use it. Try it with a secondary number first.

## What it does

- Links your account by QR code, as another device of your phone.
- Sends and receives text, photos, documents, stickers, audio and voice
  notes, with replies that show what they quote.
- Shows delivery and read marks, unread counts, and profile and group pictures.
- Creates stickers from images, and offers your favorite stickers from the phone.
- Notifies through the desktop, opens the chat when you click one, and can
  hide the message text.
- Keeps receiving messages with the window closed.

It does not store the history WhatsApp sends when linking, and supports
one account.

## How it works

Two processes talk over a Unix socket with Protocol Buffers:

- **`daemon/`**, in Go, uses [whatsmeow](https://github.com/tulir/whatsmeow)
  for the protocol and SQLite for storage. It runs as a systemd user
  service, so it keeps receiving messages when the UI is closed.
- **`ui/`**, in Rust, is a GTK 4 and libadwaita window that draws in software
  by default to save memory.

## Memory

With one account and one chat, the daemon and the UI together use about
64 MB (PSS), against 384 MB for Whats4Linux and over 1 GB for the clients
that wrap WhatsApp Web. The accounts differ in history, so read it as an
order of magnitude. Method, caveats and every figure are in
[`docs/performance.md`](docs/performance.md).

## Building

Packages for Fedora and the daemon and UI commands are in
[`docs/development.md`](docs/development.md).

## Documentation

- Development setup, configuration and how media works: [`docs/development.md`](docs/development.md).
- IPC between the daemon and the UI: [`docs/ipc.md`](docs/ipc.md).
- Memory measurements: [`docs/performance.md`](docs/performance.md).
- Icons and look: [`docs/visual-identity.md`](docs/visual-identity.md).

Copyright © 2026 Mauricio Cerón Medina. Licensed under the [GNU GPL v3 or later](LICENSE).
