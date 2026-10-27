# Translating

Wassup uses gettext with one domain, `wassup`, shared by the UI and the
daemon. Translations live in `po/`, one `.po` file per language, listed in
`po/LINGUAS`.

## Adding or updating a translation

```sh
po/i18n.sh update                 # refresh po/wassup.pot and every .po from the code
msginit -l fr -i po/wassup.pot -o po/fr.po   # start a new language
echo fr >> po/LINGUAS
po/i18n.sh install                # compile into ~/.local/share/locale to try it
```

Then restart the daemon and the UI. Both pick the first XDG data
directory that has Wassup translations, so `~/.local/share/locale` wins
over the system one during development.

Messages with `{}` get a value at runtime; keep the `{}` in the
translation. The comment above such a message says what it stands for.
`msgfmt --check` in `po/i18n.sh install` rejects a translation that drops
it.

## Marking strings in the code

- **UI (Rust):** wrap the text in `gettext("...")`. Put values in with
  `.replace("{}", value)`, and explain `{}` in a `// TRANSLATORS:` comment
  on the line before. xgettext does not look inside `glib::clone!`, so
  translatable text must sit outside that macro.
- **Daemon (Go):** wrap the text in `tr("...")`. Errors meant for the user
  are translated where the reply is sent, since package-level errors are
  created before the language is known.
- **Desktop file:** `Name`, `Comment` and `Keywords` are extracted from
  `packaging/io.github.MawCeron.Wassup.desktop`.

## Known limitation

Placeholders for content Wassup cannot show yet, such as `[Video]` or
`[Location]`, are stored with the message in the language in use when it
arrived. Changing the language later does not translate them.
