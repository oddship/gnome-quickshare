use super::*;
use rqs_lib::{WifiPayload, sharing_nearby::wifi_credentials_metadata::SecurityType};

impl Window {
    pub(super) fn compose_wifi(&self) {
        let dialog = adw::MessageDialog::builder()
            .transient_for(&self.window)
            .heading(tr("Share Wi-Fi"))
            .body(tr("Enter a network's details, then choose a nearby device. The password is sent only after that device accepts."))
            .build();
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("prepare", &tr("Choose Device"))]);
        dialog.set_close_response("cancel");
        dialog.set_default_response(Some("prepare"));
        dialog.set_response_appearance("prepare", adw::ResponseAppearance::Suggested);
        let group = adw::PreferencesGroup::new();
        let name = adw::EntryRow::builder()
            .title(tr("Network Name (SSID)"))
            .activates_default(true)
            .build();
        let security = adw::ComboRow::builder()
            .title(tr("Security"))
            .model(&gtk::StringList::new(&[
                &tr("WPA/WPA2 Personal"),
                &tr("Open"),
                &tr("WEP"),
            ]))
            .build();
        let password = adw::PasswordEntryRow::builder()
            .title(tr("Password"))
            .activates_default(true)
            .build();
        let hidden = adw::SwitchRow::builder()
            .title(tr("Hidden Network"))
            .build();
        group.add(&name);
        group.add(&security);
        group.add(&password);
        group.add(&hidden);
        let hint = label(
            "Use a network name of 1–32 bytes and a password of 8–63 bytes (or 64 hexadecimal digits).",
            "body",
        );
        hint.set_xalign(0.);
        let form = vertical(12);
        form.append(&group);
        form.append(&hint);
        dialog.set_extra_child(Some(&form));
        if let Some(wifi) = self.wifi.borrow().as_ref() {
            name.set_text(&wifi.ssid);
            password.set_text(&wifi.password);
            security.set_selected(match wifi.security_type {
                SecurityType::Open => 1,
                SecurityType::Wep => 2,
                _ => 0,
            });
            hidden.set_active(wifi.hidden_ssid);
        }
        // Validate while editing; invalid credentials never enable sending.
        let weak_dialog = dialog.downgrade();
        let n = name.downgrade();
        let p = password.downgrade();
        let s = security.downgrade();
        let h = hidden.downgrade();
        let hint = hint.downgrade();
        let update: Rc<dyn Fn()> = Rc::new(move || {
            let (Some(n), Some(p), Some(s), Some(h), Some(hint)) = (
                n.upgrade(),
                p.upgrade(),
                s.upgrade(),
                h.upgrade(),
                hint.upgrade(),
            ) else {
                return;
            };
            let open = s.selected() == 1;
            p.set_sensitive(!open);
            hint.set_text(&tr(match s.selected() {
                1 => mark("Use a network name of 1–32 bytes. This network has no password."),
                2 => mark("Use a network name of 1–32 bytes and a WEP key of 5 or 13 ASCII characters, or 10 or 26 hexadecimal digits."),
                _ => mark("Use a network name of 1–32 bytes and a password of 8–63 bytes (or 64 hexadecimal digits)."),
            }));
            if let Some(dialog) = weak_dialog.upgrade() {
                dialog.set_response_enabled("prepare", payload(&n, &p, &s, &h).validate().is_ok());
            }
        });
        let changed = update.clone();
        name.connect_changed(move |_| changed());
        let changed = update.clone();
        password.connect_changed(move |_| changed());
        let changed = update.clone();
        security.connect_selected_notify(move |_| changed());
        update();
        let weak = self.weak();
        dialog.connect_response(None, move |_, response| {
            if response != "prepare" {
                return;
            }
            let wifi = payload(&name, &password, &security, &hidden);
            if wifi.validate().is_err() {
                return;
            }
            if let Some(ui) = weak.upgrade() {
                ui.wifi_preview.set_title(&wifi.ssid);
                ui.wifi_preview.set_title_lines(1);
                ui.wifi_preview.set_tooltip_text(Some(&wifi.ssid));
                ui.wifi_preview.set_subtitle(&tr("Wi-Fi network"));
                *ui.wifi.borrow_mut() = Some(wifi);
                ui.wifi_list.set_visible(true);
                ui.text_mode.set(true);
                ui.composer.set_visible(false);
                ui.refresh_files();
                ui.stack.set_visible_child_name("send");
                ui.window.present();
            }
        });
        dialog.present();
    }
}

fn payload(
    name: &adw::EntryRow,
    password: &adw::PasswordEntryRow,
    security: &adw::ComboRow,
    hidden: &adw::SwitchRow,
) -> WifiPayload {
    let security_type = match security.selected() {
        1 => SecurityType::Open,
        2 => SecurityType::Wep,
        _ => SecurityType::WpaPsk,
    };
    WifiPayload {
        ssid: name.text().to_string(),
        password: if security_type == SecurityType::Open {
            String::new()
        } else {
            password.text().to_string()
        },
        security_type,
        hidden_ssid: hidden.is_active(),
    }
}

impl Window {
    pub(super) fn received_wifi(&self, wifi: &WifiPayload, device: &str) {
        let dialog = adw::MessageDialog::builder()
            .transient_for(&self.window)
            .heading(trf("Wi-Fi Network from {device}", &[("device", device)]))
            .body(tr("Review the details before using this network."))
            .build();
        let group = adw::PreferencesGroup::new();
        let network = adw::ActionRow::builder()
            .use_markup(false)
            .title(tr("Network"))
            .subtitle(&wifi.ssid)
            .build();
        network.set_subtitle_lines(2);
        let copy_name = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text(tr("Copy Network Name"))
            .valign(gtk::Align::Center)
            .build();
        copy_name.add_css_class("flat");
        let ssid = wifi.ssid.clone();
        copy_name.connect_clicked(move |_| {
            if let Some(display) = gdk::Display::default() {
                display.clipboard().set_text(&ssid);
            }
        });
        network.add_suffix(&copy_name);
        group.add(&network);
        let security = adw::ActionRow::builder()
            .title(tr("Security"))
            .subtitle(tr(match wifi.security_type {
                SecurityType::Open => mark("Open"),
                SecurityType::Wep => mark("WEP"),
                SecurityType::WpaPsk => mark("WPA/WPA2 Personal"),
                _ => mark("Unknown"),
            }))
            .build();
        group.add(&security);
        if wifi.security_type != SecurityType::Open {
            let password_row = adw::ActionRow::builder()
                .use_markup(false)
                .title(tr("Password"))
                .subtitle("••••••••")
                .build();
            password_row.set_subtitle_lines(3);
            let reveal = gtk::ToggleButton::builder()
                .icon_name("view-reveal-symbolic")
                .tooltip_text(tr("Show Password"))
                .valign(gtk::Align::Center)
                .build();
            reveal.add_css_class("flat");
            let row = password_row.downgrade();
            let password = wifi.password.clone();
            reveal.connect_toggled(move |button| {
                if let Some(row) = row.upgrade() {
                    row.set_subtitle(if button.is_active() {
                        &password
                    } else {
                        "••••••••"
                    });
                    button.set_icon_name(if button.is_active() {
                        "view-conceal-symbolic"
                    } else {
                        "view-reveal-symbolic"
                    });
                    let name = tr(if button.is_active() {
                        mark("Hide Password")
                    } else {
                        mark("Show Password")
                    });
                    button.set_tooltip_text(Some(&name));
                    button.update_property(&[gtk::accessible::Property::Label(&name)]);
                }
            });
            password_row.add_suffix(&reveal);
            group.add(&password_row);
        } else {
            group.add(
                &adw::ActionRow::builder()
                    .title(tr("Password"))
                    .subtitle(tr("No password (open network)"))
                    .build(),
            );
        }
        group.add(
            &adw::ActionRow::builder()
                .title(tr("Hidden Network"))
                .subtitle(tr(if wifi.hidden_ssid {
                    mark("Yes")
                } else {
                    mark("No")
                }))
                .build(),
        );
        dialog.set_extra_child(Some(&group));
        dialog.add_response("close", &tr("Close"));
        if wifi.security_type != SecurityType::Open {
            dialog.add_response("copy-password", &tr("Copy Password"));
        }
        dialog.set_close_response("close");
        dialog.set_default_response(Some("close"));
        let password = wifi.password.clone();
        let weak = self.weak();
        dialog.connect_response(None, move |_, response| {
            if response == "copy-password"
                && let Some(display) = gdk::Display::default()
            {
                display.clipboard().set_text(&password);
                if let Some(ui) = weak.upgrade() {
                    ui.notify(&tr("Password copied"));
                }
            }
        });
        dialog.present();
    }
}
