// Remove only the newly created test fixture from the host's recent-file list.
imports.gi.versions.Gtk = '4.0';
const {Gtk, Gio, GLib} = imports.gi;
const [fixture, output] = ARGV;
if (!GLib.path_is_absolute(fixture) || !/^quickshare-portal-fixture-[A-Za-z0-9]+\.txt$/.test(GLib.path_get_basename(fixture)))
    throw new Error('Only the chooser fixture may be removed');
Gtk.init();
const manager = Gtk.RecentManager.get_default();
const uri = Gio.File.new_for_path(fixture).get_uri();
const existed = manager.has_item(uri);
if (existed) manager.remove_item(uri);
const loop = new GLib.MainLoop(null, false);
GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1000, () => { loop.quit(); return GLib.SOURCE_REMOVE; });
loop.run();
if (manager.has_item(uri)) throw new Error('Fixture survived recent-file cleanup');
GLib.file_set_contents(output, JSON.stringify({fixtureEntryExisted: existed, fixtureEntryAbsent: true}, null, 2));
