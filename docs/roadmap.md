# Roadmap

What Wassup does today and what comes next. Bugs and requests are tracked
in [GitHub issues](https://github.com/MawCeron/Wassup/issues).

## Done

**Messaging**
- Text, photos, videos, documents, stickers, GIFs, audio and voice notes,
  sent and received; media downloads on demand into a size-limited cache.
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
- Stickers: favorites from the phone, adding and removing a favorite from a
  chat, recent ones, animated ones, and new ones made from images.
- Mentions: the name of a mentioned person in the text, a notification for
  a mention of the user in a muted chat, and a list of the members of a
  group when typing `@`.

**Desktop**
- Notifications with the window closed, which can hide the message text;
  a status icon with the unread count.
- Preferences, light and dark styles, a choice of chat background, and
  translations (Spanish so far).
- Keyboard use and screen reader support, following the GNOME guidelines.
- An About dialog with the licenses of the code and the font Wassup bundles.
- A daemon that starts at login and stops with Quit, for low memory use.

**Packaging**
- RPM for Fedora, built offline from vendored dependencies.
- A sandboxed systemd unit, and a daemon that accepts connections only from
  its own user.
- CI on every push, and GitHub releases built from tags; the first one is
  0.1.0.

## Planned

Each version has a theme, in the order planned. There are no dates, and
what a version holds can change as work goes on.

### 0.2.0: a chat as faithful as the phone's

- Edited and deleted messages.
- Notifications for reactions.
- Forwarding messages.

### 0.3.0: more kinds of message

- Polls, contacts and locations, shown and sent.
- Waveforms for voice notes, and thumbnails for videos you send.

### 0.4.0: managing chats

- Showing which chats are muted, so it is clear why one does not notify.
- Archiving, muting, pinning and deleting chats.

### 0.5.0: accounts and access

- Several accounts at once.
- Linking by phone number or passkey instead of a QR code.
- An application lock.

### Distribution

Not tied to a version.

- **COPR repository**, so Fedora users can install and update with `dnf`.
- **Installation guide.**

## Not planned

- Calls, if the protocol ever allows them.
