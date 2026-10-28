"""Optional Nautilus 4 extension. Installed by the Nix package; restart Files to load."""
from gi.repository import GObject, Gio, Nautilus


class QuickshareExtension(GObject.GObject, Nautilus.MenuProvider):
    def get_file_items(self, files):
        if not files or any(f.is_directory() or f.get_uri_scheme() != "file" for f in files):
            return []
        item = Nautilus.MenuItem(
            name="Quickshare::send", label="Send with Quickshare…",
            tip="Share these files with a nearby device", icon="send-to-symbolic",
        )
        item.connect("activate", self._send, [f.get_location() for f in files])
        return [item]

    @staticmethod
    def _send(_item, files):
        app = Gio.DesktopAppInfo.new("io.github.oddship.GnomeQuickshare.desktop")
        if app:
            app.launch(files, None)
