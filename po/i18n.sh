#!/bin/sh
# Translation upkeep. See docs/translating.md.
#   po/i18n.sh update           extract strings into wassup.pot and merge them into every .po
#   po/i18n.sh install [DIR]    compile the .po files into DIR (default ~/.local/share/locale)
set -eu
cd "$(dirname "$0")/.."

case "${1:-}" in
update)
	common="--package-name=Wassup --msgid-bugs-address=https://github.com/MawCeron/Wassup/issues --from-code=UTF-8 --add-comments=TRANSLATORS: --sort-by-file"
	xgettext $common -L Rust -k -kgettext -kngettext:1,2 -o po/wassup.pot ui/src/*.rs ui/src/window/*.rs
	xgettext $common -L Go -k -ktr -j -o po/wassup.pot $(ls daemon/*.go | grep -v _test.go)
	xgettext $common -L Desktop -j -o po/wassup.pot packaging/io.github.MawCeron.Wassup.desktop
	for lang in $(cat po/LINGUAS); do
		msgmerge -q --update --backup=none "po/$lang.po" po/wassup.pot
	done
	;;
install)
	dir="${2:-$HOME/.local/share/locale}"
	for lang in $(cat po/LINGUAS); do
		mkdir -p "$dir/$lang/LC_MESSAGES"
		msgfmt --check -o "$dir/$lang/LC_MESSAGES/wassup.mo" "po/$lang.po"
	done
	;;
*)
	sed -n '2,4p' "$0" >&2
	exit 1
	;;
esac
