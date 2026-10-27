# Packaging

Wassup ships as an RPM for Fedora, built from `packaging/wassup.spec`.

## Building the RPM

```sh
sudo dnf install rpm-build rpmdevtools golang cargo rust gtk4-devel \
    libadwaita-devel libwebp-devel glib2-devel protobuf-compiler gettext \
    desktop-file-utils appstream systemd-rpm-macros
rpmdev-setuptree
packaging/tarballs.sh          # sources from HEAD into ~/rpmbuild/SOURCES
rpmbuild -ba packaging/wassup.spec
```

`packaging/tarballs.sh` makes two sources from a commit: the code, and a
tarball with every Go module and Rust crate it uses (`go mod vendor`,
`cargo vendor`). The build then needs no network, as COPR and mock
require. `REF=<commit>` packages another commit than HEAD.

## What the package installs

| Path | What |
|---|---|
| `/usr/bin/wassupd`, `/usr/bin/wassup` | The daemon and the UI |
| `/usr/lib/systemd/user/wassupd.{service,socket}` | The daemon as a user service, started at login and by the UI after quitting |
| `/usr/lib/systemd/user-preset/80-wassup.preset` | Enables both for every user, so notifications work without setup |
| `/usr/share/dbus-1/services/io.github.MawCeron.Wassup.service` | Starts the UI for notification clicks and the status icon |
| `/usr/share/applications/io.github.MawCeron.Wassup.desktop` | The launcher, translated from `po/` |
| `/usr/share/metainfo/io.github.MawCeron.Wassup.metainfo.xml` | The description GNOME Software shows |
| `/usr/share/icons/hicolor/…` | The app icon and its symbolic version; interface icons are inside the UI binary |
| `/usr/share/locale/*/LC_MESSAGES/wassup.mo` | Translations |

## Sandboxing

`wassupd.service` runs the daemon with systemd's sandboxing, as far as a
user service allows: the file system is read-only except for Wassup's
data, cache and config directories and the runtime directory; it gets no
new privileges, devices, kernel tunables or namespaces of its own; system
calls are limited to `@system-service`, sockets to Unix and Internet ones,
and memory cannot be writable and executable at once. Files it creates are
private to the user. `systemd-analyze --user security wassupd` rates it
3.8 (OK), down from 9.4.

The directories are the XDG defaults under the home directory. Someone who
moves them with `XDG_DATA_HOME` and the like needs a drop-in adding the new
paths to `ReadWritePaths=`.

The daemon also checks, with `SO_PEERCRED`, that whoever connects to its
socket runs as the same user, including when systemd hands it the socket.

## Dependencies

At run time Wassup needs glycin's loaders for images and stickers,
GStreamer (base and good plugins) for video, audio and voice notes, and
util-linux for `setpriv`, which keeps a recording from outliving the
window. H.264 video needs `gstreamer1-plugin-openh264`, which the package
recommends; the status icon on GNOME needs the AppIndicator extension,
which it suggests.

## Licenses

The `License:` tag lists Wassup's license and those of every bundled Go
module and Rust crate, as Fedora asks of vendored code. When dependencies
change, list them again: for Rust, the `license` field of each crate in
`cargo metadata`; for Go, the license file of each module that
`go mod vendor` brings in.
