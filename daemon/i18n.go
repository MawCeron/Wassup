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

// trn is tr for a message that depends on a count, picking the plural form
// the language needs. The count goes in with the message's {}.
func trn(one, many string, n int) string { return getN(one, many, n) }

// get and getN are gotext.Get and GetN, which only format when given
// arguments. Calling them through variables keeps vet from taking every
// message for a format.
var (
	get  = gotext.Get
	getN = gotext.GetN
)

// initLocale loads the translations for language, or for the system's
// language if it is empty, from the same .mo files as the UI.
func initLocale(language string) {
	lang := language
	if lang == "" {
		lang = os.Getenv("LANGUAGE") // A list such as "es_MX:es", as gettext reads it.
	}
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
