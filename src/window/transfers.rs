use super::*;

impl Window {
    pub(super) fn transfer(&self, id: &str, client: &MessageClient) {
        let inbound = client.kind == TransferKind::Inbound;
        if self
            .transfers
            .borrow()
            .get(id)
            .is_some_and(|t| t.terminal.get())
            && client.state.as_ref().is_some_and(|s| {
                !matches!(
                    s,
                    TransferState::Finished
                        | TransferState::Cancelled
                        | TransferState::Rejected
                        | TransferState::Disconnected
                )
            })
        {
            self.transfers.borrow_mut().remove(id);
        }
        let item = {
            let mut transfers = self.transfers.borrow_mut();
            transfers
                .entry(id.to_string())
                .or_insert_with(|| {
                    let row = adw::ActionRow::builder()
                        .use_markup(false)
                        .title(if inbound {
                            tr("Incoming transfer")
                        } else {
                            tr("Outgoing transfer")
                        })
                        .build();
                    row.set_title_lines(1);
                    row.set_subtitle_lines(2);
                    let indicator = icon(
                        if inbound {
                            "folder-download-symbolic"
                        } else {
                            "send-to-symbolic"
                        },
                        24,
                    );
                    row.add_prefix(&indicator);
                    let open = gtk::Button::with_label(&tr("Open Folder"));
                    open.add_css_class("flat");
                    open.set_valign(gtk::Align::Center);
                    open.set_visible(false);
                    open.set_tooltip_text(Some(&tr("Open Received Files")));
                    let weak = self.weak();
                    open.connect_clicked(move |_| {
                        if let Some(ui) = weak.upgrade() {
                            ui.open_downloads();
                        }
                    });
                    row.add_suffix(&open);
                    let cancel = gtk::Button::builder()
                        .icon_name("process-stop-symbolic")
                        .tooltip_text(tr("Cancel Transfer"))
                        .valign(gtk::Align::Center)
                        .build();
                    cancel.add_css_class("flat");
                    row.add_suffix(&cancel);
                    let weak = self.weak();
                    let transfer_id = id.to_string();
                    cancel.connect_clicked(move |_| {
                        if let Some(ui) = weak.upgrade() {
                            ui.backend.send(Command::Action(
                                transfer_id.clone(),
                                TransferAction::TransferCancel,
                            ));
                        }
                    });
                    let list = gtk::ListBox::new();
                    list.add_css_class("boxed-list");
                    list.set_selection_mode(gtk::SelectionMode::None);
                    list.append(&row);
                    let box_ = vertical(0);
                    box_.append(&list);
                    let progress = gtk::ProgressBar::new();
                    progress.add_css_class("transfer-progress");
                    let progress_row = gtk::ListBoxRow::builder()
                        .child(&progress)
                        .activatable(false)
                        .selectable(false)
                        .build();
                    list.append(&progress_row);
                    self.transfer_list.prepend(&box_);
                    self.activity_empty.set_visible(false);
                    Rc::new(TransferWidgets {
                        container: box_,
                        row,
                        progress,
                        cancel,
                        open,
                        indicator,
                        metadata: RefCell::default(),
                        consent: RefCell::default(),
                        terminal: Cell::new(false),
                        recorded: Cell::new(false),
                        started: Cell::new(None),
                    })
                })
                .clone()
        };
        if let Some(meta) = &client.metadata {
            *item.metadata.borrow_mut() = Some(meta.clone());
        }
        let metadata = item.metadata.borrow().clone();
        let Some(state) = &client.state else {
            return;
        };
        let terminal = matches!(
            state,
            TransferState::Finished
                | TransferState::Rejected
                | TransferState::Cancelled
                | TransferState::Disconnected
        );
        item.terminal.set(terminal);
        item.cancel.set_visible(!terminal);
        if let Some(progress_row) = item.progress.parent() {
            progress_row.set_visible(!terminal);
        }
        let state_label = match state {
            TransferState::WaitingForUserConsent => mark("Waiting for approval"),
            TransferState::ReceivingFiles => mark("Receiving"),
            TransferState::SendingFiles => mark("Sending"),
            TransferState::Finished => mark("Complete"),
            TransferState::Rejected => mark("Declined"),
            TransferState::Cancelled => mark("Cancelled"),
            TransferState::Disconnected => mark("Connection lost"),
            _ => mark("Connecting"),
        };
        if let Some(meta) = &metadata {
            let device = meta
                .source
                .as_ref()
                .map(|s| s.name.clone())
                .unwrap_or_else(|| tr("Nearby device"));
            item.row.set_title(&trf(
                if inbound {
                    mark("From {device}")
                } else {
                    mark("To {device}")
                },
                &[("device", &device)],
            ));
            item.progress
                .set_fraction(if *state == TransferState::Finished {
                    1.
                } else if meta.total_bytes > 0 {
                    (meta.ack_bytes as f64 / meta.total_bytes as f64).clamp(0., 1.)
                } else {
                    0.
                });
            if matches!(
                state,
                TransferState::SendingFiles | TransferState::ReceivingFiles
            ) {
                let started = item.started.get().unwrap_or_else(|| {
                    let now = std::time::Instant::now();
                    item.started.set(Some(now));
                    now
                });
                let seconds = started.elapsed().as_secs_f64();
                if seconds > 0.25 && meta.ack_bytes > 0 {
                    let rate = meta.ack_bytes as f64 / seconds;
                    let remaining = (meta.total_bytes.saturating_sub(meta.ack_bytes) as f64
                        / rate.max(1.))
                    .ceil() as u64;
                    let eta = if remaining < 60 {
                        ntr(
                            "{count} second left",
                            "{count} seconds left",
                            remaining as usize,
                        )
                    } else {
                        ntr(
                            "{count} minute left",
                            "{count} minutes left",
                            remaining.div_ceil(60) as usize,
                        )
                    };
                    item.progress.set_show_text(true);
                    item.progress.set_text(Some(&trf(
                        "{rate}/s · {eta}",
                        &[("rate", &model::bytes(rate as u64)), ("eta", &eta)],
                    )));
                }
            }
            item.row.set_subtitle(&trf(
                "{state} · {done} of {total}{code}",
                &[
                    ("state", &tr(state_label)),
                    ("done", &model::bytes(meta.ack_bytes)),
                    ("total", &model::bytes(meta.total_bytes)),
                    (
                        "code",
                        &meta
                            .pin_code
                            .as_ref()
                            .map(|p| trf(" · Code {code}", &[("code", p)]))
                            .unwrap_or_default(),
                    ),
                ],
            ));
            if terminal {
                let content = match &meta.payload {
                    Some(TransferPayload::Files(files)) if files.len() == 1 => {
                        std::path::Path::new(&files[0])
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    }
                    Some(TransferPayload::Files(files)) => model::file_count(files.len()),
                    Some(TransferPayload::Url(_)) => tr("Link"),
                    Some(TransferPayload::Wifi { .. }) => tr("Wi-Fi Details"),
                    _ => tr("Text"),
                };
                let time = glib::DateTime::now_local()
                    .ok()
                    .and_then(|now| now.format(&tr("%H:%M")).ok())
                    .map(|time| time.to_string())
                    .unwrap_or_default();
                item.row.set_title(&content);
                let omit_size = matches!(
                    meta.payload_kind,
                    rqs_lib::hdl::info::TransferPayloadKind::WiFi
                ) || (meta.total_bytes == 0
                    && !matches!(
                        meta.payload_kind,
                        rqs_lib::hdl::info::TransferPayloadKind::Files
                    ));
                item.row.set_subtitle(&trf(
                    if omit_size && inbound {
                        mark("{state} · From {device} · {time}")
                    } else if omit_size {
                        mark("{state} · To {device} · {time}")
                    } else if inbound {
                        mark("{state} · From {device} · {size} · {time}")
                    } else {
                        mark("{state} · To {device} · {size} · {time}")
                    },
                    &[
                        ("state", &tr(state_label)),
                        ("device", &device),
                        ("size", &model::bytes(meta.total_bytes)),
                        ("time", &time),
                    ],
                ));
                if *state == TransferState::Finished {
                    item.indicator.set_icon_name(Some("object-select-symbolic"));
                    item.indicator.add_css_class("success");
                    item.open.set_visible(
                        inbound && matches!(meta.payload, Some(TransferPayload::Files(_))),
                    );
                } else {
                    item.indicator
                        .set_icon_name(Some("dialog-warning-symbolic"));
                    item.indicator.add_css_class("warning");
                }
            }
            if inbound
                && *state == TransferState::WaitingForUserConsent
                && item.consent.borrow().is_none()
            {
                if let Some(TransferPayload::Files(files)) = &meta.payload
                    && !model::safe_offer(files)
                {
                    self.backend
                        .send(Command::Action(id.into(), TransferAction::ConsentDecline));
                    self.notify(&tr("Rejected a transfer with an unsafe filename."));
                    return;
                }
                if !self.visible.is_active() {
                    self.backend
                        .send(Command::Action(id.into(), TransferAction::ConsentDecline));
                    return;
                }
                self.ask_consent(id, meta, &item);
            }
            if *state == TransferState::Finished
                && inbound
                && !item.recorded.get()
                && let Some(payload) = &meta.payload
            {
                self.received_payload(payload, &device);
            }
        } else {
            item.row.set_subtitle(&tr(state_label));
        }
        if terminal {
            item.progress.set_show_text(false);
            if let Some(dialog) = item.consent.borrow_mut().take() {
                dialog.close();
            }
            if !item.recorded.replace(true) {
                if *state == TransferState::Finished {
                    item.progress.set_fraction(1.);
                    self.desktop_notification(
                        id,
                        &tr("Transfer complete"),
                        &tr("Your share has arrived."),
                    );
                }
                let entry = model::HistoryEntry {
                    device: metadata
                        .as_ref()
                        .and_then(|m| m.source.as_ref())
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| "Nearby device".into()),
                    incoming: inbound,
                    bytes: metadata.as_ref().map(|m| m.total_bytes).unwrap_or(0),
                    outcome: state_label.into(),
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                };
                let mut history = self.history.borrow_mut();
                history.push(entry);
                if history.len() > 100 {
                    let extra = history.len() - 100;
                    history.drain(..extra);
                }
                if let Err(error) = model::store_history(&history) {
                    self.notify(&trf(
                        "Could not save history: {error}",
                        &[("error", &error.to_string())],
                    ));
                }
                let weak = self.weak();
                glib::idle_add_local_once(move || {
                    if let Some(ui) = weak.upgrade() {
                        ui.dispatch_next();
                    }
                });
            }
        }
    }
    pub(super) fn dispatch_next(&self) {
        if self.transfers.borrow().values().any(|t| !t.terminal.get()) {
            return;
        }
        if self.restart_pending.replace(false) {
            self.request_restart();
            return;
        }
        if !self.ready.get() {
            return;
        }
        let Some(pending) = self.queue.borrow_mut().pop_front() else {
            return;
        };
        self.transfer_list.remove(&pending.widget);
        // Install a connecting row before handing the request to the worker.
        self.transfer(
            &pending.info.id,
            &MessageClient {
                kind: TransferKind::Outbound,
                state: Some(TransferState::Initial),
                metadata: None,
            },
        );
        self.backend.send(Command::Send(pending.info));
    }
    pub(super) fn restore_history(&self) {
        for entry in self.history.borrow().iter().rev() {
            let time = glib::DateTime::from_unix_local(entry.timestamp as i64)
                .ok()
                .and_then(|t| t.format(&tr("%e %b, %H:%M")).ok())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let row = adw::ActionRow::builder()
                .use_markup(false)
                .title(trf(
                    if entry.incoming {
                        mark("From {device}")
                    } else {
                        mark("To {device}")
                    },
                    &[("device", &entry.device)],
                ))
                .subtitle(format!(
                    "{} · {} · {}",
                    tr(&entry.outcome),
                    model::bytes(entry.bytes),
                    time
                ))
                .build();
            row.set_title_lines(1);
            row.set_subtitle_lines(2);
            row.add_prefix(&icon(
                if entry.incoming {
                    "folder-download-symbolic"
                } else {
                    "send-to-symbolic"
                },
                24,
            ));
            let list = gtk::ListBox::new();
            list.add_css_class("boxed-list");
            list.set_selection_mode(gtk::SelectionMode::None);
            list.append(&row);
            self.history_list.append(&list);
            self.activity_empty.set_visible(false);
        }
    }
    pub(super) fn ask_consent(
        &self,
        id: &str,
        meta: &rqs_lib::hdl::info::TransferMetadata,
        item: &Rc<TransferWidgets>,
    ) {
        let device = meta
            .source
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| tr("Nearby device"));
        let description = match &meta.payload {
            Some(TransferPayload::Files(files)) => format!(
                "{} · {}\n{}",
                model::file_count(files.len()),
                model::bytes(meta.total_bytes),
                {
                    let mut names = files.iter().take(3).cloned().collect::<Vec<_>>();
                    if files.len() > 3 {
                        names.push(ntr("and {count} more", "and {count} more", files.len() - 3));
                    }
                    names.join("\n")
                }
            ),
            _ => match meta.payload_kind {
                rqs_lib::hdl::info::TransferPayloadKind::WiFi => trf(
                    "Wi-Fi network “{ssid}”",
                    &[("ssid", meta.payload_preview.as_deref().unwrap_or(""))],
                ),
                rqs_lib::hdl::info::TransferPayloadKind::Url => tr("Link"),
                rqs_lib::hdl::info::TransferPayloadKind::Text => tr("Text message"),
                _ => tr("Files"),
            },
        };
        let dialog = adw::MessageDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .heading(trf("Accept from {device}?", &[("device", &device)]))
            .body(trf(
                "{description}\n\nAccept only if both codes match.",
                &[("description", &description)],
            ))
            .build();
        let pin = label(meta.pin_code.as_deref().unwrap_or("—"), "pin");
        pin.update_property(&[gtk::accessible::Property::Label(&trf(
            "Matching code {code}",
            &[(
                "code",
                meta.pin_code.as_deref().unwrap_or(&tr("unavailable")),
            )],
        ))]);
        dialog.set_extra_child(Some(&pin));
        dialog.add_responses(&[("decline", &tr("Decline")), ("accept", &tr("Accept"))]);
        dialog.set_response_appearance("accept", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("decline"));
        dialog.set_close_response("decline");
        if meta.pin_code.is_none() {
            dialog.set_response_enabled("accept", false);
        }
        let weak = self.weak();
        let transfer_id = id.to_string();
        dialog.connect_response(None, move |_, response| {
            if let Some(ui) = weak.upgrade() {
                ui.backend.send(Command::Action(
                    transfer_id.clone(),
                    if response == "accept" {
                        TransferAction::ConsentAccept
                    } else {
                        TransferAction::ConsentDecline
                    },
                ));
            }
        });
        *item.consent.borrow_mut() = Some(dialog.clone());
        dialog.present();
        self.desktop_notification(
            id,
            &tr("Incoming share"),
            &trf(
                "{device} wants to share with you. Open Quickshare to review.",
                &[("device", &device)],
            ),
        );
    }
    pub(super) fn received_payload(&self, payload: &TransferPayload, device: &str) {
        if let TransferPayload::Wifi {
            ssid,
            password,
            security_type,
            hidden_ssid,
        } = payload
        {
            self.received_wifi(
                &rqs_lib::WifiPayload {
                    ssid: ssid.clone(),
                    password: password.clone(),
                    security_type: *security_type,
                    hidden_ssid: *hidden_ssid,
                },
                device,
            );
            return;
        }
        let text = match payload {
            TransferPayload::Text(text) | TransferPayload::Url(text) => text.clone(),
            TransferPayload::Wifi { .. } => unreachable!(),
            TransferPayload::Files(_) => return,
        };
        let dialog = adw::MessageDialog::builder()
            .transient_for(&self.window)
            .heading(tr("Something for you"))
            .body(tr("Review received content before using it."))
            .build();
        let view = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .left_margin(12)
            .right_margin(12)
            .top_margin(12)
            .bottom_margin(12)
            .build();
        view.buffer().set_text(&text);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&view)
            .min_content_height(120)
            .max_content_height(320)
            .propagate_natural_height(true)
            .build();
        dialog.set_extra_child(Some(&scroll));
        dialog.add_responses(&[("close", &tr("Close")), ("copy", &tr("Copy"))]);
        dialog.set_close_response("close");
        dialog.connect_response(None, move |_, response| {
            if response == "copy"
                && let Some(display) = gdk::Display::default()
            {
                display.clipboard().set_text(&text);
            }
        });
        dialog.present();
    }
    pub(super) fn desktop_notification(&self, id: &str, title: &str, body: &str) {
        let n = gio::Notification::new(&tr(title));
        n.set_body(Some(&tr(body)));
        n.set_default_action("app.show-activity");
        self.app.send_notification(Some(id), &n);
    }
}
