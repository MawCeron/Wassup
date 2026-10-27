package main

import (
	"os"
	"path/filepath"
	"strings"

	"github.com/leonelquinteros/gotext"
)

// tr translates text shown to the user. Strings are extracted from calls
// to it into po/wassup.pot.
func tr(s string) string { return get(s) }

// get is gotext.Get, which only formats when given arguments. Calling it
// through a variable keeps vet from taking every message for a format.
var get = gotext.Get

// initLocale loads the translations for the user's language, from the same
// .mo files as the UI.
func initLocale() {
	lang := os.Getenv("LANGUAGE") // A list such as "es_MX:es", as gettext reads it.
	for _, env := range []string{"LC_ALL", "LC_MESSAGES", "LANG"} {
		if lang == "" {
			lang = os.Getenv(env)
		}
	}
	gotext.Configure(localeDir(), lang, "wassup")
}

// localeDir is the first locale directory under the XDG data directories
// with Wassup translations: ~/.local/share/locale during development, the
// system one when installed.
func localeDir() string {
	home, _ := os.UserHomeDir()
	dirs := []string{os.Getenv("XDG_DATA_HOME")}
	if dirs[0] == "" {
		dirs[0] = filepath.Join(home, ".local", "share")
	}
	system := os.Getenv("XDG_DATA_DIRS")
	if system == "" {
		system = "/usr/local/share:/usr/share"
	}
	dirs = append(dirs, strings.Split(system, ":")...)
	for _, dir := range dirs {
		locale := filepath.Join(dir, "locale")
		if found, _ := filepath.Glob(filepath.Join(locale, "*", "LC_MESSAGES", "wassup.mo")); len(found) > 0 {
			return locale
		}
	}
	return "/usr/share/locale"
}
