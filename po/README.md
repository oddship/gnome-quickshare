# Translations

The application uses GNU gettext through GLib. `gnome-quickshare.pot` is the
source template. No reviewed human-language catalogs are included yet; the
interface falls back to English. The synthetic `zz` catalog exists only in the
temporary layout test and is never installed in release packages.

Use the development shell (`nix develop path:.`) for gettext, Python, AppStream,
and the native GUI smoke tools. Update the template and merge existing catalogs:

```sh
scripts/i18n/update.sh
```

To add a language, create `po/LANGUAGE.po` with `msginit` from the template, edit
its headers/plural rules and translations, and add the language name on its own
line in `po/LINGUAS`. The installer compiles catalogs with `msgfmt --check
--check-format` and merges translated desktop/AppStream metadata. Do not commit
compiled `.mo` files.

`tr` translates a literal. `mark` marks a label stored for later display, keeping
history outcomes independent of the current language. `trf` uses named tokens
such as `{device}`: preserve every token's spelling, but reorder them as needed.
`ntr` selects the language's plural form before replacing `{count}`. Placeholders
in filenames, device names and received content are data, never recursively
expanded. GTK icon names, D-Bus/action names, file paths, accelerators and the
Quickshare/Quick Share brand names remain unchanged.

The `label` and `section` helpers translate their first argument; the extraction
script includes those keywords. Error text returned by the operating system or
the transport engine may be untranslated. Host Files/Shell extensions do not yet
use these application catalogs.

Installed binaries look for `share/locale` under their installation prefix.
Cargo builds fall back to `po/locale`; `GNOME_QUICKSHARE_LOCALEDIR` explicitly
selects another catalog directory for development. Language selection follows the
system locale and standard `LANGUAGE` environment variable.

Run the accented/expanded-layout smoke check after building:

```sh
scripts/i18n/smoke.sh "$PWD/target/debug/gnome-quickshare" /tmp/quickshare-i18n
```

The check needs a generated non-C UTF-8 locale (`en_US.UTF-8` by default; set
`QUICKSHARE_TEST_LOCALE` to another installed locale). It compiles a temporary
catalog, verifies localized CLI help, desktop metadata
and AppStream XML, then captures the actual GTK Send/Receive/Activity/Preferences
views at desktop and narrow widths. Set `QUICKSHARE_SMOKE_PROBE` to the native
probe binary to include the localized consent/transfer check. This is a layout
and lookup test, not evidence that a human translation is accurate.
