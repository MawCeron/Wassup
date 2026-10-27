# Changelog

All notable changes to Wassup are listed here, newest first. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
version numbers follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-10-27

First release.

### Added

- Linking one account by QR code, as another device of the phone. Recent
  history comes with it, and older messages load on demand as you scroll up.
- Sending and receiving text, photos, videos, documents, stickers, GIFs and
  voice notes, with replies, reactions and link previews.
- Mentions: the name of a mentioned person in the text, a notification for a
  mention of the user in a muted chat, and a list of the members of a group
  when typing `@`.
- Stickers: favorites kept in sync with the phone, adding and removing a
  favorite from a chat, recent stickers, animated stickers, and new stickers
  made from images.
- Delivery and read marks, unread counts, and profile and group pictures.
- Disappearing messages, and locked chats kept behind the phone's secret
  code.
- Search for chats, and for messages inside a chat.
- A photo and video viewer inside the window, and a preview before sending a
  file.
- Notifications with the window closed, which can hide the message text,
  and a status icon with the unread count.
- Preferences, light and dark styles, a choice of chat background, and a
  Spanish translation.
- Keyboard use and screen reader support, following the GNOME guidelines.
- A daemon that runs as a systemd user service, so messages keep arriving
  with the window closed, and a UI that draws in software by default to save
  memory.
- An About dialog that lists the licenses of the code and the emoji font
  Wassup bundles.
- An RPM for Fedora, built offline from vendored dependencies.

[Unreleased]: https://github.com/MawCeron/Wassup/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/MawCeron/Wassup/releases/tag/v0.1.0
