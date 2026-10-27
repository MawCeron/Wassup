#!/bin/sh
# Builds the RPM sources from the current commit: the source tarball and
# one with the Go and Rust dependencies vendored, so the package builds
# without network, as COPR and mock require, and the emoji font with its
# license, downloaded at a pinned version. See docs/packaging.md.
#   packaging/tarballs.sh [OUTDIR]   (default ~/rpmbuild/SOURCES)
# REF=<commit> packages another commit than HEAD.
set -eu
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' ui/Cargo.toml | head -1)
out=${1:-$HOME/rpmbuild/SOURCES}
ref=${REF:-HEAD}
mkdir -p "$out"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

git archive --prefix="wassup-$version/" -o "$out/wassup-$version.tar.gz" "$ref"

# Vendored from the same commit, never from uncommitted changes.
git archive --prefix=src/ "$ref" | tar -x -C "$work"
(cd "$work/src/daemon" && go mod vendor)
mkdir -p "$work/src/ui/.cargo"
(cd "$work/src/ui" && cargo vendor --locked vendor > .cargo/config.toml)
tar -C "$work/src" -cJf "$out/wassup-$version-vendor.tar.xz" \
	daemon/vendor ui/vendor ui/.cargo/config.toml
echo "$out/wassup-$version.tar.gz"
echo "$out/wassup-$version-vendor.tar.xz"

# The emoji font, built by https://github.com/MawCeron/twemoji-cbdt.
# Keep the version in wassup.spec in step with this one.
font_version=17.0.3.1
font_url=https://github.com/MawCeron/twemoji-cbdt/releases/download/v$font_version
fetch() { # file, sha256 and name in the sources
	curl -fsSL "$font_url/$1" -o "$out/$3"
	echo "$2  $out/$3" | sha256sum -c --quiet -
	echo "$out/$3"
}
fetch TwemojiCBDT.ttf b25db3224c9257ffaff51dd2ca04bbe5855183d8b901aa77a6d13747f9b6d56d "TwemojiCBDT-$font_version.ttf"
fetch NOTICE 51a4f56a01dfee3a9cd50cb9647b1dd757c9b22e99addfa8a845d039d315c4bb twemoji-cbdt-NOTICE
fetch LICENSE-GRAPHICS 8ae9438818c26e4873b91d8c6ad620526c011e27e125677f13031eda903f007c twemoji-cbdt-LICENSE-GRAPHICS
