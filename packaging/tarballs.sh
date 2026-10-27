#!/bin/sh
# Builds the RPM sources from the current commit: the source tarball and
# one with the Go and Rust dependencies vendored, so the package builds
# without network, as COPR and mock require. See docs/packaging.md.
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
