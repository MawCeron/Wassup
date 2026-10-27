# Roadmap

What Wassup does today and what comes next. Bugs and requests are tracked
in [GitHub issues](https://github.com/MawCeron/Wassup/issues).

## Done

**Messaging**
- Text, photos, videos, documents, stickers, audio and voice notes, sent
  and received; media downloads on demand into a size-limited cache.
- Replies that quote a message, emoji reactions and link previews, both
  ways.
- Delivery and read marks, unread counts, profile and group pictures.
- Disappearing messages, and chats locked on the phone kept behind its
  secret code.
- History from the phone: three months when linking, older messages on
  demand as you scroll up.
- Search for chats, and for messages inside a chat.
- A photo and video viewer inside the window, and a preview before sending
  a file.
- Stickers: favorites from the phone, recent ones, and new ones made from
  images.

**Desktop**
- Notifications with the window closed, which can hide the message text;
  a status icon with the unread count.
- Preferences, light and dark styles, a choice of chat background, and
  translations (Spanish so far).
- Keyboard use and screen reader support, following the GNOME guidelines.
- A daemon that starts at login and stops with Quit, for low memory use.

**Packaging**
- RPM for Fedora, built offline from vendored dependencies.

## Next

- **COPR repository**, so Fedora users can install and update with `dnf`.
- **Hardening:** a stricter systemd unit, and checking which process
  connects to the daemon's socket.
- **Installation guide.**

## Later

No fixed order.

- Edited and deleted messages.
- Forwarding messages.
- Polls, contacts and locations, shown and sent.
- Waveforms for voice notes, and thumbnails for videos you send.
- Notifications for reactions.
- Archiving, muting, pinning and deleting chats.
- Several accounts at once.
- Linking by phone number or passkey instead of a QR code.
- An application lock.
- Calls, if the protocol ever allows them.
