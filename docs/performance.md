# Performance

Memory is measured as PSS from `/proc/<pid>/smaps_rollup`, because RSS
counts shared libraries in full for every process that maps them. RSS is
listed for reference. Each figure is the median of three samples taken
at least five minutes after the account connected, while idle.

## Measurements

| Date | Version | Process | Accounts | PSS | RSS | Notes |
|---|---|---|---|---|---|---|
| 2026-10-22 | 12f86eb | daemon | 1 | 25.0 MB | 27.7 MB | One group chat in the store |
| 2026-10-22 | 12f86eb | UI | 1 | 85.2 MB | 178.2 MB | Debug build, default GSK renderer, one chat open |
| 2026-10-22 | f2d7bfc | daemon | 1 | 20.0 MB | 22.8 MB | systemd user service, 12 min up, 4 chats, 59 messages |
| 2026-10-22 | f2d7bfc | UI | 1 | 80.5 MB | 147.5 MB | Debug build, started by D-Bus activation 30 s before |
| 2026-10-23 | 21eedc9 | daemon | 1 | 25.9 MB | 28.5 MB | 14 min up, 8 chats, 235 messages, 48 cached media files (5.8 MB) |
| 2026-10-23 | 21eedc9 | UI | 1 | 138.3 MB | 169.7 MB | Debug build, 14 min of use with photos, stickers and voice notes |
| 2026-10-27 | a4d2cd5 | daemon | 1 | 40.8 MB | 43.5 MB | RPM, release build, 19 min up, 29 chats, 3485 messages |
| 2026-10-27 | a4d2cd5 | UI | 1 | 293.1 MB | 344.3 MB | RPM, release build, software rendering, Noto Color Emoji, 9 min up |
| 2026-10-27 | 6d55533 | daemon | 1 | 29.8 MB | | Release build, 1 h 43 min up, same account |
| 2026-10-27 | 6d55533 | UI | 1 | 34.1 MB | | Release build, software rendering, Twemoji, no chat open, 5 min up |

## Where the UI's memory goes

Of the UI's 138 MB above, about 70 MB are graphics drivers rather than
Wassup: on a machine with Intel and NVIDIA graphics, GTK's GPU rendering
loads the NVIDIA OpenGL libraries (about 40 MB) through glvnd, even when
the desktop runs on the Intel GPU, plus several Vulkan drivers including
Mesa's software one, which brings LLVM (about 27 MB). The heap was 30 MB.

The same chat, with images, in a test copy of the UI:

| Rendering | PSS |
|---|---|
| GTK default (GPU, all drivers) | 100 MB |
| Vulkan disabled, or only the Intel Vulkan driver | 100 MB |
| OpenGL through Mesa only (`__EGL_VENDOR_LIBRARY_FILENAMES` set to Mesa's) | 75 MB |
| No GPU (`GDK_DISABLE=gl,vulkan`, `GSK_RENDERER=cairo`) | 40 MB |

The renderer alone does not matter: GDK initializes OpenGL and Vulkan
when it opens the display, whichever renderer draws afterwards.

Daemon and UI together: about 100–110 MB PSS with GPU rendering; see
[Other clients](#other-clients) for how that compares.

### Rendering benchmark

A test copy of the UI scrolled a 200-message chat with 40 photos for
8 seconds at about 1500 px/s, on a HiDPI screen (scale 2) with Intel and
NVIDIA graphics. Two runs gave the same figures; the medians:

| Rendering | Frames | 95th percentile frame | CPU while scrolling | USS | RSS | PSS |
|---|---|---|---|---|---|---|
| GTK default (GPU) | 59.0 fps | 16.7 ms | 12–13 % | 75 MB | 175 MB | 110 MB |
| GPU through Mesa only | 59.0 fps | 16.7 ms | 12–13 % | 62 MB | 137 MB | 85 MB |
| Software (cairo) | 59.6 fps | 16.7 ms | 30 % | 38 MB | 99 MB | 55 MB |

All three keep 60 fps. Software rendering spends about 2.4 times the CPU
while scrolling, and none at rest. PSS depends on what other processes
share, so USS (memory only this process holds) is the fairer comparison.

## Emoji picker

Opening the emoji picker with Noto Color Emoji, the system's default emoji
font, makes the UI grow by hundreds of megabytes that are never given
back. The growth comes from the emoji font, not from Wassup: a minimal
GTK app that only creates a `GtkEmojiChooser` reproduces it.

### Where it comes from

Creating the picker alongside the composer (`a2b9abb`) took the UI from
27 MB to 234 MB at start. A bisection of 46 commits with release builds
and no daemon found that commit. Opening the picker once raised the UI to
616–638 MB, almost all of it heap.

GTK 4.22 builds a `GtkLabel` for each of about 1900 emoji when the picker
is created, and cairo caches the color glyphs it draws. GTK merge request
!10407 ("port to GtkGridView") attributes about 240 MiB of a growth of
over 400 MiB to cairo's scaled-font cache, and cairo merge request !671
describes the cache staying after the picker closes and caps it at
16 MiB. On 2026-10-27 the cairo request was still open, Fedora 44 had
cairo 1.18.6, and the `GtkGridView` picker was only in GTK's `main`
branch (4.25), not in 4.22 or 4.24.

### What does not free the memory

| Change in Wassup | UI after opening the picker once |
|---|---|
| Created on first use | 616–638 MB |
| Destroyed when it closes (closing with its button, the log confirmed it) | 616 MB |
| Destroyed when it closes, then `malloc_trim(0)` | 622 MB |

The memory is held by the process, not by the picker. Creating the
viewer's `gtk::Video` only when a video is shown did not change the
figures either (234.3 and 234.5 MB at start).

### Emoji font benchmark

A GTK 4.22.5 app with a window and a menu button holding a
`GtkEmojiChooser`, on Fedora 44, with the cairo renderer
(`GSK_RENDERER=cairo`, `GDK_DISABLE=gl,vulkan`). It reads its PSS at 4 s,
shows the picker, reads it at 12 s, closes it and reads it at 16 s.

```python
import sys, gi
gi.require_version('Gtk', '4.0')
from gi.repository import Gtk, GLib

def pss():
    for l in open('/proc/self/smaps_rollup'):
        if l.startswith('Pss:'):
            return int(l.split()[1]) / 1024

out = []
def log(label):
    out.append(f"{label} {pss():.1f}")

def on_activate(app):
    win = Gtk.ApplicationWindow(application=app, default_width=600, default_height=400)
    btn = Gtk.MenuButton(label="emoji")
    ch = Gtk.EmojiChooser()
    btn.set_popover(ch)
    win.set_child(btn)
    win.present()
    for ms, fn in [
        (4000, lambda: log("created")),
        (4500, btn.popup),
        (12000, lambda: log("shown mapped=%s" % ch.get_mapped())),
        (12500, btn.popdown),
        (16000, lambda: log("closed")),
        (16500, lambda: (print(" | ".join(out)), app.quit())),
    ]:
        GLib.timeout_add(ms, lambda f=fn: (f(), False)[1])

app = Gtk.Application(application_id="test.EmojiBench")
app.connect("activate", on_activate)
app.run([])
```

Each run used a `FONTCONFIG_FILE` that includes `/etc/fonts/fonts.conf`,
adds the directory of the font under test and rejects the other emoji
fonts:

| Emoji font | Renderer | Created | After showing |
|---|---|---|---|
| None (no picker) | cairo | 35.1 MB | 35.2 MB |
| None (no picker) | ngl | 130.8 MB | 130.8 MB |
| Noto Color Emoji (COLRv1) | cairo | 236 MB | 241–608 MB, varies by run |
| Noto Color Emoji (COLRv1) | ngl | 278 MB | 454 MB |
| Noto Emoji (monochrome) | cairo | 51 MB | 57.5 MB |
| Noto Emoji (monochrome) | ngl | 77 MB | 81 MB |
| OpenMoji Color 16 (COLRv1) | cairo | 110 MB | 112–238 MB, varies by run |
| Twemoji 14.0.2 (bitmap, CBDT) | cairo | 48 MB | 53.5–54.4 MB |
| twemoji-cbdt 17.0.3 (bitmap, CBDT) | cairo | 52 MB | 58 MB |

The fonts that grow, Noto Color Emoji and OpenMoji Color, are COLRv1
fonts whose glyphs are vectors; Fedora's Twemoji is a bitmap font
(`CBDT` and `CBLC` tables) and the monochrome Noto Emoji has plain
outlines. The popover was mapped in the Twemoji runs. The monochrome and Twemoji
figures repeated within a megabyte across runs; the Noto Color Emoji ones
did not, since they depend on how many glyphs cairo had drawn.

The picker drops the emoji whose ink is more than 1.5 times as wide as
that of 🙂, which Pango may draw with another font than the rest. With
Symbola, a monochrome symbol font, installed, it draws 🙂 narrower than
Twemoji draws flags and sequences, and the picker keeps fewer emoji:

| Emoji font | Emoji kept of 1910, Symbola installed | Symbola hidden |
|---|---|---|
| Noto Color Emoji | 1910 | |
| Twemoji 14.0.2 | 1373 | 1854 |
| twemoji-cbdt 17.0.3 | 1400 | 1910 |

Twemoji covers 1466 code points and Noto Color Emoji 1508, because the
Fedora package is Twemoji 14.0.2 and lacks the emoji added to Unicode
after version 14.

### What Wassup does

- The picker is created when its button is first pressed.
- Wassup carries a bitmap emoji font, Twemoji CBDT, and loads it in its own
  process. The UI then takes both Noto emoji fonts, other Twemoji fonts and
  Symbola out of its fontconfig, so the picker and the messages draw every
  emoji with it. Taking out only Noto Color Emoji makes Pango fall back to
  the monochrome Noto Emoji. Preferring Twemoji through fontconfig rules or
  through CSS `font-family` had no effect, because Pango chooses the font
  for emoji by itself.
- Without the font file, nothing changes.

With Fedora's Twemoji 14.0.2, a release build used 34.1 MB for the UI at
rest and 66.3 MB after a chat was open and the picker had been used. With
Twemoji CBDT, the font Wassup carries, the UI used 37.6 MB 20 seconds after
it started and 81.6 MB, peaking at 128 MB, after the picker had been opened
and closed, with 1910 emoji in the picker instead of 1373. With Noto Color
Emoji it was 293 MB at rest and over 600 MB after using the picker.

## Other clients

The same method (PSS, median of three samples, taken at least five
minutes after launch, idle) applied to the other WhatsApp clients
installed on the test machine, on 2026-10-26. Each ran alone, with the
window open and the account already linked, and PSS is summed over the
whole process tree.

| Client | Version | Kind | Processes | PSS | RSS | USS |
|---|---|---|---|---|---|---|
| Wassup, realistic history | 21eedc9 | Go daemon + GTK4, GPU rendering, debug build | 2 | 164 MB | 198 MB | |
| Wassup, new account | 7f38e22 | Go daemon + GTK4, software rendering, release build | 2 | 64 MB | 107 MB | 59 MB |
| Whats4Linux | AppImage | Go + WebKitGTK | 4 | 384 MB | 594 MB | 319 MB |
| whatRust 0.6.7 | Flatpak | Rust + WebKitGTK, WhatsApp Web | 4 | 1022 MB | 1241 MB | 925 MB |
| Karere 4.3.4 | Flatpak | Chromium, WhatsApp Web | 8 | 1356 MB | 2016 MB | 1162 MB |
| Wassup 0.1.0-1, same account as Karere below | a4d2cd5 | Go daemon + GTK4, software rendering, release RPM, Noto Color Emoji | 2 | 334 MB | 388 MB | 324 MB |
| Wassup, same account as Karere below | 6d55533 | The same with Twemoji, no chat open | 2 | 64 MB | | |
| Karere 4.3.4, same account as Wassup above | Flatpak | Chromium, WhatsApp Web | 9 | 1327 MB | 1990 MB | 1129 MB |

The first two Wassup rows differ in build, rendering and history, so
neither is the figure to quote alone.

- **Realistic history:** the daemon (25.9 MB) and the UI (138.3 MB) from
  2026-10-23, 14 minutes of use with 8 chats, 235 messages and 48 cached
  media files. It was a debug build with GTK's default GPU rendering,
  which was the default then.
- **New account:** the daemon (35.0 MB) and the UI (29.3 MB), release
  builds with software rendering, five minutes after linking, one chat
  open and one chat stored. The daemon is larger than in the first row,
  possibly because of the history WhatsApp sends on linking, which Wassup
  processes but does not store; this was not checked.

The last three rows, from 2026-10-27, are the release build with software
rendering on an account that holds 29 chats and 3485 messages, with Karere
open on the same account. The Wassup figures are taken at the same time
for the daemon and the UI; the UI of the last Wassup row was started with
no chat open. The second of those rows is the sum of 29.8 MB for the
daemon and 34.1 MB for the UI, five minutes after the UI started.

Caveats: the accounts are different and hold different amounts of chat
history, so the figures show the order of magnitude, not an exact ratio.
The WhatsApp Web clients load the whole web app in a browser engine,
which is where most of their memory goes.
