import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const APP = 'io.github.oddship.GnomeQuickshare';
const PATH = '/io/github/oddship/GnomeQuickshare';

export default class Quickshare extends Extension {
    enable() {
        this._button = new PanelMenu.Button(0, 'Quickshare');
        this._icon = new St.Icon({icon_name: 'send-to-symbolic', style_class: 'system-status-icon'});
        this._button.add_child(this._icon);
        this._visibility = new PopupMenu.PopupSwitchMenuItem('Visible for 10 minutes', false);
        this._button.menu.addMenuItem(this._visibility);
        this._visibility.connect('toggled', (_item, value) => {
            this._actions.activate_action('visible', new GLib.Variant('b', value));
        });
        this._button.menu.addAction('Open Quickshare', () => {
            Gio.DBus.session.call(APP, PATH, 'org.freedesktop.Application', 'Activate',
                new GLib.Variant('(a{sv})', [{}]), null, Gio.DBusCallFlags.NONE, -1, null, (connection, result) => {
                    try { connection.call_finish(result); } catch (error) { console.error(`Quickshare: ${error.message}`); }
                });
        });
        this._actions = Gio.DBusActionGroup.get(Gio.DBus.session, APP, PATH);
        this._signals = ['action-state-changed', 'action-enabled-changed', 'action-added', 'action-removed'].map(signal =>
            this._actions.connect(signal, () => this._sync()));
        this._sync();
        Main.panel.addToStatusArea(this.uuid, this._button);
    }
    _sync() {
        const running = this._actions.has_action('visible') && this._actions.get_action_enabled('visible');
        const visible = this._actions.get_action_state('visible')?.get_boolean() ?? false;
        this._visibility.setToggleState(visible);
        this._visibility.setSensitive(running);
        this._icon.icon_name = visible ? 'view-reveal-symbolic' : 'send-to-symbolic';
    }
    disable() {
        for (const id of this._signals ?? []) this._actions.disconnect(id);
        this._actions = null;
        this._signals = null;
        this._button?.destroy();
        this._button = null;
    }
}
