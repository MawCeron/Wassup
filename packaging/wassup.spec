# Go binaries carry no debug data Fedora's tools can split out.
%global debug_package %{nil}

Name:           wassup
Version:        0.1.0
Release:        1%{?dist}
Summary:        Unofficial WhatsApp client for GNOME

# Wassup is GPL-3.0-or-later. Bundled Go modules: GPL-3.0-only (libsignal),
# MPL-2.0 (whatsmeow), BSD-3-Clause, BSD-2-Clause, MIT, Apache-2.0, ISC,
# and SQLite's blessing. Bundled Rust crates: MIT, Apache-2.0,
# (MIT OR Apache-2.0), (Unlicense OR MIT) and Unicode-3.0.
License:        GPL-3.0-or-later AND GPL-3.0-only AND MPL-2.0 AND BSD-3-Clause AND BSD-2-Clause AND MIT AND Apache-2.0 AND ISC AND blessing AND (MIT OR Apache-2.0) AND (Unlicense OR MIT) AND Unicode-3.0
URL:            https://github.com/MawCeron/Wassup
Source0:        %{name}-%{version}.tar.gz
Source1:        %{name}-%{version}-vendor.tar.xz

ExclusiveArch:  x86_64 aarch64

BuildRequires:  golang >= 1.26.9
BuildRequires:  cargo
BuildRequires:  rust >= 1.92
BuildRequires:  pkgconfig(gtk4) >= 4.22
BuildRequires:  pkgconfig(libadwaita-1) >= 1.9
BuildRequires:  pkgconfig(libwebp)
BuildRequires:  glib2-devel
BuildRequires:  protobuf-compiler
BuildRequires:  gettext
BuildRequires:  desktop-file-utils
BuildRequires:  appstream
BuildRequires:  systemd-rpm-macros

# Images and stickers are decoded by glycin; video, audio and voice
# notes go through GStreamer.
Requires:       glycin-loaders
Requires:       gstreamer1
Requires:       gstreamer1-plugins-base
Requires:       gstreamer1-plugins-good
# setpriv keeps a recording from outliving the window.
Requires:       util-linux
Recommends:     gstreamer1-plugin-openh264
# GNOME shows the status icon only with this extension.
Suggests:       gnome-shell-extension-appindicator

%description
Wassup is a WhatsApp client for GNOME that speaks the protocol natively,
without a web view, and stays light on memory. A background service keeps
notifications coming while the window is closed.

Wassup is not affiliated with, endorsed by or connected to WhatsApp or Meta.

%prep
%autosetup -n %{name}-%{version}
tar -xJf %{SOURCE1}

%build
(cd daemon && GOFLAGS=-mod=vendor go build -trimpath -ldflags "-B gobuildid" -o wassupd .)
(cd ui && cargo build --release --offline --locked)

%install
install -Dm0755 daemon/wassupd %{buildroot}%{_bindir}/wassupd
install -Dm0755 ui/target/release/wassup %{buildroot}%{_bindir}/wassup
install -Dm0644 packaging/wassupd.service %{buildroot}%{_userunitdir}/wassupd.service
install -Dm0644 packaging/wassupd.socket %{buildroot}%{_userunitdir}/wassupd.socket
install -Dm0644 packaging/80-wassup.preset %{buildroot}%{_userpresetdir}/80-wassup.preset
install -Dm0644 packaging/io.github.MawCeron.Wassup.service \
    %{buildroot}%{_datadir}/dbus-1/services/io.github.MawCeron.Wassup.service
msgfmt --desktop -d po --template packaging/io.github.MawCeron.Wassup.desktop \
    -o io.github.MawCeron.Wassup.desktop
desktop-file-install --dir=%{buildroot}%{_datadir}/applications io.github.MawCeron.Wassup.desktop
install -Dm0644 data/io.github.MawCeron.Wassup.metainfo.xml \
    %{buildroot}%{_metainfodir}/io.github.MawCeron.Wassup.metainfo.xml
install -Dm0644 data/icons/hicolor/scalable/apps/io.github.MawCeron.Wassup.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/io.github.MawCeron.Wassup.svg
install -Dm0644 data/icons/hicolor/symbolic/apps/io.github.MawCeron.Wassup-symbolic.svg \
    %{buildroot}%{_datadir}/icons/hicolor/symbolic/apps/io.github.MawCeron.Wassup-symbolic.svg
po/i18n.sh install %{buildroot}%{_datadir}/locale
%find_lang %{name}

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/io.github.MawCeron.Wassup.desktop
appstreamcli validate --no-net %{buildroot}%{_metainfodir}/io.github.MawCeron.Wassup.metainfo.xml
(cd daemon && GOFLAGS=-mod=vendor go test ./...)
(cd ui && cargo test --release --offline --locked)

%post
%systemd_user_post wassupd.socket wassupd.service

%preun
%systemd_user_preun wassupd.socket wassupd.service

%files -f %{name}.lang
%license LICENSE
%doc README.md
%{_bindir}/wassup
%{_bindir}/wassupd
%{_userunitdir}/wassupd.service
%{_userunitdir}/wassupd.socket
%{_userpresetdir}/80-wassup.preset
%{_datadir}/dbus-1/services/io.github.MawCeron.Wassup.service
%{_datadir}/applications/io.github.MawCeron.Wassup.desktop
%{_metainfodir}/io.github.MawCeron.Wassup.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/io.github.MawCeron.Wassup.svg
%{_datadir}/icons/hicolor/symbolic/apps/io.github.MawCeron.Wassup-symbolic.svg

%changelog
* Tue Oct 27 2026 Mauricio Cerón Medina <ceron.maw@gmail.com> - 0.1.0-1
- First release
