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

Daemon and UI together: about 100–110 MB PSS. For reference, Whats4Linux
uses about 140 MB with one account.
