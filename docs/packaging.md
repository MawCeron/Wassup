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
