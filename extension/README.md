# Optional GNOME indicator

The extension adds an open action and a ten-minute visibility toggle. It uses the
app's standard GApplication action group on the session bus; it does not run a
second service or implement the protocol. Start Quickshare before toggling
visibility. The extension disables its toggle while the app is stopped.

Packaged separately as `nix build .#gnome-extension`. Install the resulting
`share/gnome-shell/extensions/gnome-quickshare@oddship.github.io` directory through
your normal GNOME extension setup, then enable the UUID. Shell versions 48–50
are declared; actual Shell integration still requires desktop testing.
