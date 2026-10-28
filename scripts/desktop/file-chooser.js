// Select only the uniquely named fixture in this app's real portal chooser.
const {Atspi, GLib, Gio} = imports.gi;
const [process, output, fixture] = ARGV;
const brokerPid = Number(process);
if (!Number.isInteger(brokerPid) || brokerPid <= 0 || !output || !GLib.path_is_absolute(fixture))
    throw new Error('Expected portal PID, report and absolute fixture path');
const title = 'Choose files to share';
const wanted = GLib.path_get_basename(fixture);
if (!/^quickshare-portal-fixture-[A-Za-z0-9]+\.txt$/.test(wanted)) throw new Error('Only a unique test fixture may be selected');
const safeNames = new Set([title, 'Add Files', 'Cancel', 'Back', 'Forward', wanted]);
function owners() {
    const result = new Map([[brokerPid, 'org.freedesktop.impl.portal.desktop.gnome']]);
    for (const name of ['org.freedesktop.impl.portal.desktop.gtk', 'org.gnome.Nautilus']) {
        try {
            const reply = Gio.DBus.session.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                'org.freedesktop.DBus', 'GetConnectionUnixProcessID', new GLib.Variant('(s)', [name]),
                null, Gio.DBusCallFlags.NONE, 1000, null);
            result.set(reply.deepUnpack()[0], name);
        } catch (_) { /* Delegates can start only when requested. */ }
    }
    return result;
}
function inspect(node, budget, depth = 0) {
    if (--budget.remaining < 0 || depth > 35) throw new Error('Chooser inspection exceeded its bound');
    const name = node.get_name(), role = node.get_role_name(), actions = [];
    for (let i = 0; i < node.get_n_actions(); i++) {
        try { actions.push(node.get_action_name(i)); } catch (_) { actions.push('<unavailable>'); }
    }
    const item = {role, name: safeNames.has(name) ? name : '<redacted>', actions,
        showing: node.get_state_set().contains(Atspi.StateType.SHOWING), children: []};
    Object.defineProperty(item, 'accessible', {value: node});
    // File names are inspected only by the exact fixture-matching predicate below.
    if (['table', 'tree', 'list', 'list box', 'tree table'].includes(role)) return item;
    for (let i = 0; i < node.get_child_count(); i++) {
        const child = node.get_child_at_index(i);
        if (child) item.children.push(inspect(child, budget, depth + 1));
    }
    return item;
}
function flatten(tree) { return [tree, ...tree.children.flatMap(flatten)]; }
function containsFixture(node, budget = {remaining: 2000}) {
    if (--budget.remaining < 0) throw new Error('Fixture inspection exceeded its bound');
    if (!node.get_state_set().contains(Atspi.StateType.SHOWING)) return false;
    const name = node.get_name();
    if (name === wanted || name.startsWith(wanted + ', ') || name.startsWith(wanted + '\n') || name.startsWith(wanted + ' (')) return true;
    for (let i = 0; i < node.get_child_count(); i++) {
        const child = node.get_child_at_index(i);
        if (child && containsFixture(child, budget)) return true;
    }
    return false;
}
const loop = new GLib.MainLoop(null, false);
let attempts = 0, selectionRequested = false, accepted = false, owner = null, failure = null;
GLib.timeout_add(GLib.PRIORITY_DEFAULT, 200, () => {
    try {
        const processes = owners(), desktop = Atspi.get_desktop(0), matches = [];
        for (let i = 0; i < desktop.get_child_count(); i++) {
            const application = desktop.get_child_at_index(i);
            if (!application || !processes.has(application.get_process_id())) continue;
            for (let j = 0; j < application.get_child_count(); j++) {
                const window = application.get_child_at_index(j);
                if (window && window.get_name() === title) matches.push({window, pid: application.get_process_id()});
            }
        }
        if (matches.length > 1) throw new Error('Refusing ambiguous chooser windows');
        if (matches.length === 0 && accepted) {
            GLib.file_set_contents(output, JSON.stringify({...owner, title, fixture: wanted, fixtureWasShown: true,
                selectionRequested, addFilesInvoked: accepted, dialogClosed: true}, null, 2));
            loop.quit(); return GLib.SOURCE_REMOVE;
        }
        if (matches.length === 1) {
            owner = {process: matches[0].pid, service: processes.get(matches[0].pid)};
            const tree = inspect(matches[0].window, {remaining: 600});
            const nodes = flatten(tree);
            if (!selectionRequested) {
                const view = nodes.find(node => node.role === 'panel' && node.actions.includes('view.select-all') && containsFixture(node.accessible));
                // The fixture directory contains exactly one controlled file. Never
                // select anything until that unique file is visible in the view.
                if (view) {
                    if (!view.accessible.do_action(view.actions.indexOf('view.select-all'))) throw new Error('The chooser refused fixture selection');
                    selectionRequested = true;
                }
            } else if (!accepted) {
                const add = nodes.find(node => node.name === 'Add Files' && node.role === 'button' && node.showing);
                if (add && add.accessible.get_state_set().contains(Atspi.StateType.SENSITIVE)) {
                    if (!add.accessible.do_action(add.actions.indexOf('click'))) throw new Error('The chooser refused Add Files');
                    accepted = true;
                }
            }
            GLib.file_set_contents(output + '.pending', JSON.stringify({...owner, selectionRequested, accepted, tree}, null, 2));
        }
        if (++attempts >= 50) throw new Error('The chooser did not select and submit the controlled fixture');
    } catch (error) {
        // GNOME can replace chooser children between the tree query and an
        // individual property read. Retry only that stale-object condition;
        // selection failures and ambiguous windows must still fail closed.
        if (String(error.message).includes('Object does not exist at path') && ++attempts < 50)
            return GLib.SOURCE_CONTINUE;
        failure = error; loop.quit(); return GLib.SOURCE_REMOVE;
    }
    return GLib.SOURCE_CONTINUE;
});
loop.run();
if (failure) throw failure;
