// Operate only on the document-portal entry for the newly created test fixture.
const {Gio, GLib} = imports.gi;
const ByteArray = imports.byteArray;
const [mode, fixture, output] = ARGV;
const appId = 'io.github.oddship.GnomeQuickshare';
if (!['absent', 'verify', 'cleanup'].includes(mode) || !GLib.path_is_absolute(fixture) || !output)
    throw new Error('Expected absent|verify|cleanup FIXTURE REPORT');
const bus = Gio.DBus.session;
function call(method, parameters = null) {
    return bus.call_sync('org.freedesktop.portal.Documents', '/org/freedesktop/portal/documents',
        'org.freedesktop.portal.Documents', method, parameters, null, Gio.DBusCallFlags.NONE, 2000, null).deepUnpack();
}
function path(bytes) { return ByteArray.toString(Uint8Array.from(bytes)).replace(/\0+$/, ''); }
function lookup() {
    // The portal expects a NUL-terminated filesystem byte string. GJS's
    // fromString() omits that terminator, so append it explicitly.
    try { return call('Lookup', new GLib.Variant('(ay)', [new Uint8Array([...ByteArray.fromString(fixture), 0])]))[0]; }
    catch (error) {
        if (Gio.DBusError.get_remote_error(error) === 'org.freedesktop.portal.Error.NotFound') return '';
        throw error;
    }
}
if (mode !== 'cleanup' && !GLib.file_test(fixture, GLib.FileTest.IS_REGULAR)) throw new Error('The host fixture does not exist');
const id = lookup();
let report = {mode, fixture, documentId: id};
if (mode === 'absent') {
    if (id !== '') throw new Error('Fixture already has a document grant');
} else if (id) {
    const [original, applications] = call('Info', new GLib.Variant('(s)', [id]));
    if (path(original) !== fixture) throw new Error('Refusing to touch a different document');
    if (mode === 'verify') {
        if (!(applications[appId] ?? []).includes('read')) throw new Error('The test app lacks document read permission');
        const mount = path(call('GetMountPoint')[0]);
        report = {...report, readGranted: true, documentPath: GLib.build_filenamev([mount, id, GLib.path_get_basename(fixture)])};
    } else {
        call('Delete', new GLib.Variant('(s)', [id]));
        if (lookup() !== '')
            throw new Error('Fixture document grant survived cleanup');
        report.deleted = true;
    }
} else if (mode === 'verify') throw new Error('The chooser did not create a document grant');
else report.deleted = false;
GLib.file_set_contents(output, JSON.stringify(report, null, 2));
