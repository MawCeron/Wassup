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

Daemon and UI together: about 100–110 MB PSS. For reference, Whats4Linux
uses about 140 MB with one account.

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

