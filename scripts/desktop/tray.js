// Inspect only the test application's registered StatusNotifierItem.
const {Gio, GLib} = imports.gi;
const [mode, output, process, applicationBus, requestedLabel] = ARGV;
const wantedPid = Number(process);
if (!["present", "absent", "invoke"].includes(mode)) throw new Error("Choose present, absent or invoke");
const appId = 'io.github.oddship.GnomeQuickshare';
const bus = Gio.DBus.session;
function get(service, path, iface, name) {
    const result = bus.call_sync(service, path, 'org.freedesktop.DBus.Properties', 'Get', new GLib.Variant('(ss)', [iface, name]), null, Gio.DBusCallFlags.NONE, 1000, null);
    return result.deepUnpack()[0].deepUnpack();
}
function plain(value) {
    if (value instanceof GLib.Variant) return plain(value.deepUnpack());
    if (Array.isArray(value)) return value.map(plain);
    if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([k,v]) => [k,plain(v)]));
    return value;
}
function find() {
    const registered = get('org.kde.StatusNotifierWatcher', '/StatusNotifierWatcher', 'org.kde.StatusNotifierWatcher', 'RegisteredStatusNotifierItems');
    const matches = [];
    for (const item of registered) {
        const slash = item.indexOf('/');
        const service = slash < 0 ? item : item.slice(0, slash);
        const path = slash < 0 ? '/StatusNotifierItem' : item.slice(slash);
        if (!Gio.dbus_is_name(service)) continue;
        try {
            if (get(service, path, 'org.kde.StatusNotifierItem', 'Id') !== appId) continue;
            const icon = get(service, path, 'org.kde.StatusNotifierItem', 'IconName');
            const menu = get(service, path, 'org.kde.StatusNotifierItem', 'Menu');
            const layout = bus.call_sync(service, menu, 'com.canonical.dbusmenu', 'GetLayout', new GLib.Variant('(iias)', [0, 1, ['label']]), null, Gio.DBusCallFlags.NONE, 1000, null);
            matches.push({id: appId, service, path, icon, menu, layout: plain(layout)});
        } catch (_) { /* Other apps and disappearing items are not evidence. */ }
    }
    return matches;
}
function processId(service) {
    return bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
        'org.freedesktop.DBus', 'GetConnectionUnixProcessID', new GLib.Variant('(s)', [service]),
        null, Gio.DBusCallFlags.NONE, 1000, null).deepUnpack()[0];
}
function invoke(items) {
    if (!Number.isInteger(wantedPid) || wantedPid <= 0 || !applicationBus?.startsWith(':'))
        throw new Error('Menu invocation requires the verified app PID and unique bus name');
    if (processId(applicationBus) !== wantedPid) throw new Error('Application process identity changed');
    if (!['Open Quickshare', 'Visible for 10 minutes', 'Hide this computer', 'Quit'].includes(requestedLabel))
        throw new Error('Only the test application menu actions may be invoked');
    const owned = items.filter(item => processId(item.service) === wantedPid);
    if (owned.length !== 1) throw new Error('Expected exactly one tray item owned by the test process');
    const item = owned[0];
    const children = item.layout[1][2];
    const entries = children.filter(entry => entry[1].label === requestedLabel);
    if (entries.length !== 1) throw new Error('Expected exactly one matching tray menu entry');
    bus.call_sync(item.service, item.menu, 'com.canonical.dbusmenu', 'Event',
        new GLib.Variant('(isvu)', [entries[0][0], 'clicked', new GLib.Variant('i', 0), 0]),
        null, Gio.DBusCallFlags.NONE, 1000, null);
    return {process: wantedPid, applicationBus, service: item.service, label: requestedLabel, eventSent: true};
}
const loop = new GLib.MainLoop(null, false);
let attempts = 0;
let failure = null;
GLib.timeout_add(GLib.PRIORITY_DEFAULT, 200, () => {
    try {
        const items = find();
        if (mode === 'invoke' && items.length > 0) {
            GLib.file_set_contents(output, JSON.stringify(invoke(items), null, 2));
            loop.quit(); return GLib.SOURCE_REMOVE;
        }
        if ((mode === 'present' && items.length > 0) || (mode === 'absent' && items.length === 0)) {
            if (mode === 'present') {
                if (!items.every(item => item.icon === `${appId}-symbolic`)) throw new Error('Tray icon identity is incorrect');
                const text = JSON.stringify(items);
                for (const label of ['Open Quickshare', 'Visible for 10 minutes', 'Hide this computer', 'Quit'])
                    if (!text.includes(label)) throw new Error(`Tray menu lacks ${label}`);
            }
            GLib.file_set_contents(output, JSON.stringify({mode, items}, null, 2));
            loop.quit(); return GLib.SOURCE_REMOVE;
        }
        if (++attempts >= 50) throw new Error('Tray registration state did not settle');
    } catch (error) { failure = error; loop.quit(); return GLib.SOURCE_REMOVE; }
    return GLib.SOURCE_CONTINUE;
});
loop.run();
if (failure) throw failure;
