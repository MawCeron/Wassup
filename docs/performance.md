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

The two Wassup rows differ in build, rendering and history, so neither
is the figure to quote alone.

- **Realistic history:** the daemon (25.9 MB) and the UI (138.3 MB) from
  2026-10-23, 14 minutes of use with 8 chats, 235 messages and 48 cached
  media files. It was a debug build with GTK's default GPU rendering,
  which was the default then.
- **New account:** the daemon (35.0 MB) and the UI (29.3 MB), release
  builds with software rendering, five minutes after linking, one chat
  open and one chat stored. The daemon is larger than in the first row,
  possibly because of the history WhatsApp sends on linking, which Wassup
  processes but does not store; this was not checked.

A release build with software rendering and a realistic history, the
configuration users will run, has not been measured yet.

Caveats: the accounts are different and hold different amounts of chat
history, so the figures show the order of magnitude, not an exact ratio.
The WhatsApp Web clients load the whole web app in a browser engine,
which is where most of their memory goes.
