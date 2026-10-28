// Run with gjs in a real GNOME session. Only inspect the requested process.
const {Atspi, GLib} = imports.gi;
const [mode, process, output, expectedCode] = ARGV;
const wantedPid = Number(process);
let observedPids = [];
if (!Number.isInteger(wantedPid) || wantedPid <= 0 || !output)
    throw new Error('Usage: gjs accessibility.js snapshot|accept PID OUTPUT [CODE]');

function inspect(accessible, budget, depth = 0) {
    if (--budget.remaining < 0 || depth > 40)
        throw new Error('Accessibility tree exceeded the inspection bound');
    const item = {name: accessible.get_name(), role: accessible.get_role_name(), children: []};
    // Keep references private; JSON evidence contains only this application's UI.
    Object.defineProperty(item, 'accessible', {value: accessible});
    for (let i = 0; i < accessible.get_child_count(); i++) {
        const child = accessible.get_child_at_index(i);
        if (child) item.children.push(inspect(child, budget, depth + 1));
    }
    return item;
}
function flatten(tree) {
    return [tree, ...tree.children.flatMap(flatten)];
}
function ownApplication() {
    const desktop = Atspi.get_desktop(0);
    observedPids = [];
    for (let i = 0; i < desktop.get_child_count(); i++) {
        const application = desktop.get_child_at_index(i);
        if (application) observedPids.push(application.get_process_id());
        if (application && application.get_process_id() === wantedPid)
            return application;
    }
    return null;
}

const loop = new GLib.MainLoop(null, false);
let attempts = 0;
let complete = false;
let failure = null;
GLib.timeout_add(GLib.PRIORITY_DEFAULT, 200, () => {
    try {
        const application = ownApplication();
        if (application) {
            const tree = inspect(application, {remaining: 1000});
            GLib.file_set_contents(output + '.pending', JSON.stringify({process: wantedPid, tree}, null, 2));
            if (mode === 'snapshot') {
                GLib.file_set_contents(output, JSON.stringify({process: wantedPid, tree}, null, 2));
                complete = true;
            } else if (mode === 'selected-file') {
                if (!/^quickshare-portal-fixture-[A-Za-z0-9]+\.txt$/.test(expectedCode ?? '')) throw new Error('Only the known chooser fixture may be checked');
                if (flatten(tree).some(node => node.name === expectedCode)) {
                    GLib.file_set_contents(output, JSON.stringify({process: wantedPid, selectedFixture: expectedCode, tree}, null, 2));
                    complete = true;
                }
            } else if (mode === 'window-visible') {
                if (!['true', 'false'].includes(expectedCode)) throw new Error('Specify true or false for window visibility');
                const desired = expectedCode === 'true';
                const showing = tree.children.some(window => window.name === 'Quickshare' && window.accessible.get_state_set().contains(Atspi.StateType.SHOWING));
                if (showing === desired) {
                    GLib.file_set_contents(output, JSON.stringify({process: wantedPid, desired, showing, tree}, null, 2));
                    complete = true;
                }
            } else if (mode === 'accept') {
                if (!/^\d{4}$/.test(expectedCode ?? '')) throw new Error('A matching four-digit test code is required');
                // Require the expected code and test peer within the same window.
                // Never activate a response in an unrelated incoming dialog.
                for (const window of tree.children) {
                    const nodes = flatten(window);
                    const matchesCode = nodes.some(node => node.name === `Matching code ${expectedCode}`);
                    const matchesPeer = nodes.some(node => node.name === 'Accept from Quickshare test peer?');
                    const button = nodes.find(node => node.name === 'Accept' && ['button', 'push button'].includes(node.role));
                    if (matchesCode && matchesPeer && button) {
                        if (!button.accessible.do_action(0)) throw new Error('Accept action was refused');
                        GLib.file_set_contents(output, JSON.stringify({process: wantedPid, matchingCodeExposed: true, peerChecked: true, acceptAction: true, tree}, null, 2));
                        complete = true;
                        break;
                    }
                }
            } else if (mode === 'set-background') {
                if (!['true', 'false'].includes(expectedCode)) throw new Error('Specify true or false for the background switch');
                const desired = expectedCode === 'true';
                const control = flatten(tree).find(node => node.name === 'Keep Running When Closed' && node.role === 'switch' && node.accessible.get_n_actions() > 0);
                if (control) {
                    const before = control.accessible.get_state_set().contains(Atspi.StateType.CHECKED);
                    if (before !== desired && !control.accessible.do_action(0)) throw new Error('Background switch action was refused');
                    GLib.file_set_contents(output, JSON.stringify({process: wantedPid, desired, before, actionInvoked: before !== desired, tree}, null, 2));
                    complete = true;
                }
            } else if (mode === 'close-preferences') {
                const preferences = tree.children.find(window => window.name === 'Preferences');
                const close = preferences && flatten(preferences).find(node => node.name === 'Close' && ['button', 'push button'].includes(node.role));
                if (close) {
                    if (!close.accessible.do_action(0)) throw new Error('Preferences close action was refused');
                    GLib.file_set_contents(output, JSON.stringify({process: wantedPid, preferencesClosed: true}, null, 2));
                    complete = true;
                }
            } else throw new Error(`Unknown operation: ${mode}`);
        }
        if (complete) { loop.quit(); return GLib.SOURCE_REMOVE; }
        if (++attempts >= 50) throw new Error('The requested application UI was not accessible in time');
    } catch (error) {
        failure = error;
        loop.quit();
        return GLib.SOURCE_REMOVE;
    }
    return GLib.SOURCE_CONTINUE;
});
loop.run();
if (failure) {
    GLib.file_set_contents(output + '.failure', JSON.stringify({process: wantedPid, observedPids, error: String(failure)}, null, 2));
    throw failure;
}
if (!complete) throw new Error('Accessibility check did not complete');
