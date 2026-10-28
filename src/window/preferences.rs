use super::*;

impl Window {
    pub(super) fn set_tray(&self, enabled: bool) {
        use ksni::TrayMethods;
        let generation = self.tray_generation.get() + 1;
        self.tray_generation.set(generation);
        let existing = self.tray_handle.borrow_mut().take();
        let weak = self.weak();
        glib::spawn_future_local(async move {
            if let Some(handle) = existing {
                handle.shutdown().await;
            }
            if !enabled {
                return;
            }
            let (tx, rx) = async_channel::unbounded();
            let result = crate::tray::Tray { tx }
                .assume_sni_available(true)
                .disable_dbus_name(std::path::Path::new("/.flatpak-info").exists())
                .spawn()
                .await;
            match result {
                Ok(handle) => {
                    let Some(ui) = weak.upgrade() else {
                        handle.shutdown().await;
                        return;
                    };
                    if ui.tray_generation.get() != generation {
                        handle.shutdown().await;
                        return;
                    }
                    *ui.tray_handle.borrow_mut() = Some(handle);
                    drop(ui);
                    while let Ok(command) = rx.recv().await {
                        let Some(ui) = weak.upgrade() else {
                            break;
                        };
                        match command {
                            crate::tray::Action::Open => ui.window.present(),
                            crate::tray::Action::Visible(value) => {
                                if ui.ready.get() {
                                    ui.visible.set_active(value);
                                }
                            }
                            crate::tray::Action::Quit => {
                                ui.backend.send(Command::Stop);
                                ui.app.quit();
                            }
                        }
                    }
                }
                Err(error) => {
                    if let Some(ui) = weak.upgrade() {
                        ui.notify(&trf(
                            "Could not show tray icon: {error}",
                            &[("error", &error.to_string())],
                        ));
                    }
                }
            }
        });
    }
    pub(super) fn preferences(&self) {
        if let Some(dialog) = self.preferences_window.upgrade() {
            dialog.present();
            return;
        }
        let dialog = adw::PreferencesWindow::builder()
            .transient_for(&self.window)
            .modal(true)
            .title(tr("Preferences"))
            .search_enabled(false)
            .default_width(520)
            .default_height(680)
            .build();
        self.preferences_window.set(Some(&dialog));
        let page = adw::PreferencesPage::new();
        page.set_focusable(true);
        let settings = self.settings.borrow().clone();
        let group = adw::PreferencesGroup::builder()
            .title(tr("This Computer"))
            .description(tr("Changes are saved as you make them."))
            .build();
        let name = adw::EntryRow::builder()
            .title(tr("Device Name"))
            .show_apply_button(true)
            .text(&settings.name)
            .build();
        group.add(&name);
        let folder = adw::ActionRow::builder()
            .use_markup(false)
            .title(tr("Receive Folder"))
            .subtitle(model::display_path(&settings.downloads))
            .subtitle_lines(1)
            .build();
        folder.set_tooltip_text(Some(&settings.downloads.to_string_lossy()));
        let pick = gtk::Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text(tr("Choose Receive Folder"))
            .valign(gtk::Align::Center)
            .build();
        pick.add_css_class("flat");
        folder.add_suffix(&pick);
        folder.set_activatable_widget(Some(&pick));
        group.add(&folder);
        page.add(&group);
        let background_group = adw::PreferencesGroup::builder()
            .title(tr("Background"))
            .build();
        let background = adw::SwitchRow::builder()
            .title(tr("Keep Running When Closed"))
            .subtitle(tr("Continue receiving while the window is closed"))
            .active(settings.background)
            .build();
        background_group.add(&background);
        let autostart = adw::SwitchRow::builder()
            .title(tr("Start at Login"))
            .subtitle(tr("Start in the background with visibility off"))
            .active(settings.autostart)
            .build();
        background_group.add(&autostart);
        let tray = adw::SwitchRow::builder()
            .title(tr("Show Tray Icon"))
            .subtitle(tr("Requires a desktop with tray support"))
            .active(settings.tray)
            .build();
        background_group.add(&tray);
        page.add(&background_group);
        let connection = adw::PreferencesGroup::builder().title(tr("Connection")).description(tr("Use the same Wi-Fi network and enable Bluetooth. A fixed TCP port can help with firewall rules; allow mDNS on UDP 5353.")).build();
        let automatic = adw::SwitchRow::builder()
            .title(tr("Automatic Port"))
            .active(settings.port.is_none())
            .build();
        connection.add(&automatic);
        let port = adw::EntryRow::builder()
            .title(tr("TCP Port"))
            .input_purpose(gtk::InputPurpose::Digits)
            .show_apply_button(true)
            .text(
                settings
                    .port
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "53317".into()),
            )
            .visible(settings.port.is_some())
            .build();
        connection.add(&port);
        connection.add(
            &adw::ActionRow::builder()
                .title(tr("Current TCP Port"))
                .subtitle(self.port.get().to_string())
                .build(),
        );
        page.add(&connection);
        let privacy = adw::PreferencesGroup::builder().title(tr("Privacy")).description(tr("History stores device names, sizes, times, and outcomes. It excludes filenames and shared content.")).build();
        let clear_history = gtk::Button::with_label(&tr("Clear Transfer History"));
        clear_history.add_css_class("destructive-action");
        privacy.add(&clear_history);
        page.add(&privacy);
        let weak = self.weak();
        clear_history.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.clear_history();
            }
        });
        dialog.add(&page);
        let controls = Rc::new(PreferenceControls {
            name: name.downgrade(),
            folder: folder.downgrade(),
            background: background.downgrade(),
            autostart: autostart.downgrade(),
            tray: tray.downgrade(),
            automatic: automatic.downgrade(),
            port: port.downgrade(),
        });
        let saving = Rc::new(Cell::new(false));
        for (row, kind) in [(background, 0), (autostart, 1), (tray, 2), (automatic, 3)] {
            let weak = self.weak();
            let parent = dialog.downgrade();
            let page = page.downgrade();
            let controls = controls.clone();
            let saving = saving.clone();
            row.connect_active_notify(move |row| {
                if saving.get() {
                    return;
                }
                if let (Some(ui), Some(parent), Some(page)) =
                    (weak.upgrade(), parent.upgrade(), page.upgrade())
                {
                    let mut candidate = ui.settings.borrow().clone();
                    match kind {
                        0 => candidate.background = row.is_active(),
                        1 => candidate.autostart = row.is_active(),
                        2 => candidate.tray = row.is_active(),
                        _ => candidate.port = if row.is_active() { None } else { Some(53317) },
                    }
                    ui.commit_preferences(candidate, &parent, &page, &saving, &controls);
                }
            });
        }
        for (row, is_name) in [(name, true), (port, false)] {
            let weak = self.weak();
            let parent = dialog.downgrade();
            let page = page.downgrade();
            let controls = controls.clone();
            let saving = saving.clone();
            row.connect_apply(move |row| {
                if saving.get() {
                    return;
                }
                if let (Some(ui), Some(parent), Some(page)) =
                    (weak.upgrade(), parent.upgrade(), page.upgrade())
                {
                    let mut candidate = ui.settings.borrow().clone();
                    if is_name {
                        candidate.name = row.text().trim().to_string();
                    } else {
                        match row.text().parse::<u32>() {
                            Ok(port) => candidate.port = Some(port),
                            Err(_) => {
                                row.add_css_class("error");
                                parent.add_toast(adw::Toast::new(&tr(
                                    "Enter a port between 1024 and 65535.",
                                )));
                                return;
                            }
                        }
                    }
                    if let Err(error) = candidate.validate() {
                        row.add_css_class("error");
                        parent.add_toast(adw::Toast::new(&error));
                        return;
                    }
                    row.remove_css_class("error");
                    ui.commit_preferences(candidate, &parent, &page, &saving, &controls);
                }
            });
        }
        let weak = self.weak();
        let parent = dialog.downgrade();
        let page_weak = page.downgrade();
        let initial_name = controls.name.clone();
        pick.connect_clicked(move |_| {
            if saving.get() {
                return;
            }
            if let (Some(ui), Some(parent), Some(page)) =
                (weak.upgrade(), parent.upgrade(), page_weak.upgrade())
            {
                let chooser = gtk::FileDialog::builder()
                    .title(tr("Receive Files In…"))
                    .accept_label(tr("Choose Folder"))
                    .build();
                chooser.set_initial_folder(Some(&gio::File::for_path(
                    &ui.settings.borrow().downloads,
                )));
                let weak = ui.weak();
                let parent_weak = parent.downgrade();
                let saving = saving.clone();
                let controls = controls.clone();
                chooser.select_folder(Some(&parent), gio::Cancellable::NONE, move |result| {
                    if let (Some(ui), Some(parent), Ok(file)) =
                        (weak.upgrade(), parent_weak.upgrade(), result)
                        && let Some(path) = file.path()
                    {
                        let mut candidate = ui.settings.borrow().clone();
                        candidate.downloads = path;
                        ui.commit_preferences(candidate, &parent, &page, &saving, &controls);
                    }
                });
            }
        });
        dialog.present();
        let page = page.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(page) = page.upgrade() {
                page.grab_focus();
            }
            if let Some(name) = initial_name.upgrade() {
                name.select_region(-1, -1);
            }
        });
    }
    fn commit_preferences(
        &self,
        candidate: Settings,
        parent: &adw::PreferencesWindow,
        page: &adw::PreferencesPage,
        saving: &Rc<Cell<bool>>,
        controls: &Rc<PreferenceControls>,
    ) {
        if saving.replace(true) {
            return;
        }
        page.set_sensitive(false);
        let weak = self.weak();
        let parent = parent.downgrade();
        let page = page.downgrade();
        let controls = controls.clone();
        let saving = saving.clone();
        glib::spawn_future_local(async move {
            if let Some(ui) = weak.upgrade() {
                let result = ui.save_preference_settings(candidate).await;
                if let Err(error) = result
                    && let Some(parent) = parent.upgrade()
                {
                    parent.add_toast(adw::Toast::new(&error));
                }
                controls.sync(&ui.settings.borrow());
            }
            saving.set(false);
            if let Some(page) = page.upgrade() {
                page.set_sensitive(true);
            }
        });
    }
    async fn save_preference_settings(&self, mut candidate: Settings) -> Result<(), String> {
        candidate.validate()?;
        let previous = self.settings.borrow().clone();
        if std::path::Path::new("/.flatpak-info").exists()
            && (candidate.background != previous.background
                || candidate.autostart != previous.autostart)
        {
            let response = ashpd::desktop::background::Background::request()
                .reason(tr("Receive shares while Quickshare is in the background").as_str())
                .auto_start(candidate.autostart)
                .command(["gnome-quickshare", "--background"])
                .dbus_activatable(false)
                .send()
                .await
                .and_then(|r| r.response())
                .map_err(|error| {
                    trf(
                        "Background permission was not granted: {error}",
                        &[("error", &error.to_string())],
                    )
                })?;
            candidate.background &= response.run_in_background();
            candidate.autostart &= response.auto_start();
        }
        if candidate.autostart != previous.autostart
            && !std::path::Path::new("/.flatpak-info").exists()
        {
            candidate.install_autostart()?;
        }
        candidate.save()?;
        let connection_changed = candidate.name != previous.name
            || candidate.downloads != previous.downloads
            || candidate.port != previous.port;
        let tray_changed = candidate.tray != previous.tray;
        let tray = candidate.tray;
        *self.settings.borrow_mut() = candidate;
        if tray_changed {
            self.set_tray(tray);
        }
        if let Some(action) = self
            .app
            .lookup_action("device-name")
            .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
        {
            action.set_state(&self.settings.borrow().name.to_variant());
        }
        if connection_changed {
            self.request_restart();
        }
        Ok(())
    }
    pub(super) fn clear_history(&self) {
        if let Err(error) = model::store_history(&[]) {
            self.notify(&error);
            return;
        }
        self.history.borrow_mut().clear();
        while let Some(child) = self.history_list.first_child() {
            self.history_list.remove(&child);
        }
        let finished: Vec<_> = self
            .transfers
            .borrow()
            .iter()
            .filter(|(_, transfer)| transfer.terminal.get())
            .map(|(id, _)| id.clone())
            .collect();
        for id in finished {
            if let Some(item) = self.transfers.borrow_mut().remove(&id) {
                self.transfer_list.remove(&item.container);
            }
        }
        self.activity_empty
            .set_visible(self.transfers.borrow().is_empty() && self.queue.borrow().is_empty());
        self.notify(&tr("Transfer history cleared."));
    }
    pub(super) fn help(&self) {
        let dialog = adw::MessageDialog::builder().transient_for(&self.window).heading(tr("A little closer, in three steps.")).body(tr("Connect both devices to the same Wi-Fi network and turn on Bluetooth.\n\nTo send: open Quick Share → Receive on your Android device. Choose files or text here, then select the nearby device.\n\nTo receive: turn on visibility in the Receive tab, share from your other device, and compare the matching code before accepting.\n\nCan’t connect? Disable VPNs or guest isolation. Allow UDP 5353 and the TCP port shown in Preferences through your firewall. A static port makes firewall setup easier.\n\nSupported: Android Quick Share and compatible LAN implementations. Wi-Fi Direct, Google accounts, contacts-only sharing, and relay transfers are unavailable.")).build();
        dialog.add_response("close", &tr("Got It"));
        dialog.present();
    }
    pub(super) fn shortcuts(&self) {
        let dialog = adw::MessageDialog::builder().transient_for(&self.window).heading(tr("Keyboard Shortcuts")).body(tr("Ctrl+O     Choose files\nCtrl+,      Preferences\nCtrl+Q     Quit\nF1             Connection help\n\nUse Tab to move between controls and Enter to activate.")).build();
        dialog.add_response("close", &tr("Close"));
        dialog.present();
    }
}

struct PreferenceControls {
    name: glib::WeakRef<adw::EntryRow>,
    folder: glib::WeakRef<adw::ActionRow>,
    background: glib::WeakRef<adw::SwitchRow>,
    autostart: glib::WeakRef<adw::SwitchRow>,
    tray: glib::WeakRef<adw::SwitchRow>,
    automatic: glib::WeakRef<adw::SwitchRow>,
    port: glib::WeakRef<adw::EntryRow>,
}
impl PreferenceControls {
    fn sync(&self, settings: &Settings) {
        if let Some(row) = self.name.upgrade() {
            row.set_text(&settings.name);
        }
        if let Some(row) = self.folder.upgrade() {
            row.set_subtitle(&model::display_path(&settings.downloads));
            row.set_tooltip_text(Some(&settings.downloads.to_string_lossy()));
        }
        for (row, active) in [
            (&self.background, settings.background),
            (&self.autostart, settings.autostart),
            (&self.tray, settings.tray),
            (&self.automatic, settings.port.is_none()),
        ] {
            if let Some(row) = row.upgrade() {
                row.set_active(active);
            }
        }
        if let Some(row) = self.port.upgrade() {
            row.set_visible(settings.port.is_some());
            row.set_text(&settings.port.unwrap_or(53317).to_string());
        }
    }
}
