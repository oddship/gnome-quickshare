use crate::i18n::{mark, ntr, tr, trf};
mod preferences;
mod transfers;
mod wifi;

use crate::{
    APP_ID,
    backend::{Backend, Command, Event},
    model::{self, Settings},
};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use rqs_lib::{
    EndpointInfo, OutboundPayload, SendInfo, TextPayloadType, TransferState,
    channel::{MessageClient, TransferAction, TransferKind},
    hdl::info::TransferPayload,
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::PathBuf,
    rc::{Rc, Weak},
};

struct TransferWidgets {
    container: gtk::Box,
    row: adw::ActionRow,
    progress: gtk::ProgressBar,
    cancel: gtk::Button,
    open: gtk::Button,
    indicator: gtk::Image,
    metadata: RefCell<Option<rqs_lib::hdl::info::TransferMetadata>>,
    consent: RefCell<Option<adw::MessageDialog>>,
    terminal: Cell<bool>,
    recorded: Cell<bool>,
    started: Cell<Option<std::time::Instant>>,
}
struct Pending {
    number: u64,
    info: SendInfo,
    widget: gtk::Box,
}

pub struct Window {
    pub window: adw::ApplicationWindow,
    app: adw::Application,
    settings: RefCell<Settings>,
    backend: Backend,
    toast: adw::ToastOverlay,
    stack: adw::ViewStack,
    files: RefCell<Vec<PathBuf>>,
    file_list: gtk::ListBox,
    selection_label: gtk::Label,
    selection_header: gtk::Box,
    composer: gtk::Box,
    text_view: gtk::TextView,
    text_mode: Cell<bool>,
    wifi: RefCell<Option<rqs_lib::WifiPayload>>,
    wifi_list: gtk::ListBox,
    wifi_preview: adw::ActionRow,
    text: gtk::TextBuffer,
    devices: RefCell<HashMap<String, EndpointInfo>>,
    device_list: gtk::ListBox,
    device_empty: gtk::Box,
    device_count: gtk::Label,
    discovery_spinner: gtk::Spinner,
    status: adw::Banner,
    visible: gtk::Switch,
    visibility_description: adw::ActionRow,
    visibility_deadline: Cell<Option<std::time::Instant>>,
    name_row: adw::ActionRow,
    downloads_row: adw::ActionRow,
    preferences_window: glib::WeakRef<adw::PreferencesWindow>,
    transfer_list: gtk::Box,
    history_list: gtk::Box,
    activity_empty: adw::StatusPage,
    transfers: RefCell<HashMap<String, Rc<TransferWidgets>>>,
    queue: RefCell<VecDeque<Pending>>,
    next_number: Cell<u64>,
    history: RefCell<Vec<model::HistoryEntry>>,
    tray_handle: RefCell<Option<ksni::Handle<crate::tray::Tray>>>,
    tray_generation: Cell<u64>,
    restart_pending: Cell<bool>,
    ready: Cell<bool>,
    port: Cell<u32>,
    timed_visibility: RefCell<Option<glib::SourceId>>,
    weak: RefCell<Weak<Self>>,
}
fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(&tr(text)));
    label.add_css_class(class);
    label.set_wrap(true);
    if class == "page-title" {
        label.set_justify(gtk::Justification::Center);
    }
    label
}
fn margins(widget: &impl IsA<gtk::Widget>, amount: i32) {
    widget.set_margin_top(amount);
    widget.set_margin_bottom(amount);
    widget.set_margin_start(amount);
    widget.set_margin_end(amount);
}
fn vertical(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}
fn icon(name: &str, size: i32) -> gtk::Image {
    let icon = gtk::Image::from_icon_name(name);
    icon.set_pixel_size(size);
    icon
}
fn section(title: &str) -> gtk::Label {
    let l = label(title, "section-heading");
    l.set_xalign(0.);
    l
}

impl Window {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let settings = Settings::load();
        let (backend, events) = Backend::start(settings.clone());
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Quickshare")
            .default_width(820)
            .default_height(820)
            .width_request(360)
            .build();
        gtk::Window::set_default_icon_name(APP_ID);
        // Follow accessibility changes without requiring GTK 4.16 CSS media queries.
        let style = adw::StyleManager::default();
        if style.is_high_contrast() {
            window.add_css_class("high-contrast");
        }
        let weak_window = window.downgrade();
        style.connect_high_contrast_notify(move |style| {
            if let Some(window) = weak_window.upgrade() {
                if style.is_high_contrast() {
                    window.add_css_class("high-contrast");
                } else {
                    window.remove_css_class("high-contrast");
                }
            }
        });
        let provider = gtk::CssProvider::new();
        provider.load_from_string(include_str!("../data/style.css"));
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let toast = adw::ToastOverlay::new();
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        let stack = adw::ViewStack::new();
        let title = adw::WindowTitle::new("Quickshare", "");
        title.set_visible(false);
        let menu = gio::Menu::new();
        menu.append(Some(&tr("Share Wi-Fi…")), Some("app.compose-wifi"));
        menu.append(Some(&tr("Preferences")), Some("app.preferences"));
        menu.append(Some(&tr("Restart Discovery")), Some("app.restart"));
        menu.append(Some(&tr("How to connect")), Some("app.help"));
        menu.append(Some(&tr("Keyboard Shortcuts")), Some("app.shortcuts"));
        menu.append(Some(&tr("About Quickshare")), Some("app.about"));
        menu.append(Some(&tr("Quit")), Some("app.quit"));
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .tooltip_text(tr("Main Menu"))
            .build();
        header.pack_end(&menu_button);
        let folder_button = gtk::Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text(tr("Open Received Files"))
            .build();
        header.pack_start(&folder_button);
        toolbar.add_top_bar(&header);
        let switcher = adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .halign(gtk::Align::Center)
            .build();
        let titles = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        titles.append(&title);
        titles.append(&switcher);
        header.set_title_widget(Some(&titles));
        let bottom_switcher = adw::ViewSwitcherBar::builder()
            .stack(&stack)
            .reveal(false)
            .build();
        toolbar.add_bottom_bar(&bottom_switcher);
        let narrow =
            adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 550sp").unwrap());
        narrow.add_setter(&switcher, "visible", Some(&false.to_value()));
        narrow.add_setter(&title, "visible", Some(&true.to_value()));
        narrow.add_setter(&bottom_switcher, "reveal", Some(&true.to_value()));
        window.add_breakpoint(narrow.clone());
        let status = adw::Banner::builder()
            .title(tr("Starting Quick Share…"))
            .button_label(tr("Connection Help"))
            .revealed(true)
            .build();
        toolbar.add_top_bar(&status);
        toolbar.set_content(Some(&stack));
        toast.set_child(Some(&toolbar));
        window.set_content(Some(&toast));

        let send = vertical(18);
        margins(&send, 24);
        let send_title = label("Send to a Nearby Device", "page-title");
        send.append(&send_title);
        narrow.add_setter(&send_title, "visible", Some(&false.to_value()));
        let intro = label("Choose what to share, then select a device below.", "body");
        intro.set_justify(gtk::Justification::Center);
        send.append(&intro);
        narrow.add_setter(&intro, "visible", Some(&false.to_value()));
        let drop_area = vertical(10);
        drop_area.add_css_class("drop-area");
        let drop_icon = icon("folder-new-symbolic", 30);
        drop_icon.add_css_class("dim-label");
        drop_area.append(&drop_icon);
        narrow.add_setter(&drop_icon, "visible", Some(&false.to_value()));
        drop_area.append(&label("Drop files here", "title-3"));
        let drop_caption = label("Or choose what to share", "dim-label");
        drop_area.append(&drop_caption);
        narrow.add_setter(&drop_caption, "visible", Some(&false.to_value()));
        narrow.add_setter(
            &drop_area,
            "css-classes",
            Some(&glib::StrV::from(["drop-area", "compact-drop"]).to_value()),
        );
        let choose = gtk::Button::with_label(&tr("Choose Files…"));
        choose.add_css_class("suggested-action");
        choose.add_css_class("pill");
        choose.set_halign(gtk::Align::Center);
        let share_text = gtk::Button::with_label(&tr("Share Text…"));
        share_text.add_css_class("pill");
        let share_actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        share_actions.set_halign(gtk::Align::Center);
        share_actions.append(&choose);
        share_actions.append(&share_text);
        narrow.add_setter(
            &share_actions,
            "orientation",
            Some(&gtk::Orientation::Vertical.to_value()),
        );
        drop_area.append(&share_actions);
        send.append(&drop_area);
        let selection_label = label("", "body");
        selection_label.set_xalign(0.);
        let selection_header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        selection_header.append(&selection_label);
        selection_label.set_hexpand(true);
        let clear = gtk::Button::with_label(&tr("Clear"));
        clear.add_css_class("flat");
        selection_header.append(&clear);
        selection_header.set_visible(false);
        send.append(&selection_header);
        let file_list = gtk::ListBox::new();
        file_list.add_css_class("boxed-list");
        file_list.set_selection_mode(gtk::SelectionMode::None);
        file_list.set_visible(false);
        send.append(&file_list);
        let text_box = vertical(10);
        text_box.set_visible(false);
        text_box.append(&section("Text or Link"));
        let text_view = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(12)
            .bottom_margin(12)
            .left_margin(12)
            .right_margin(12)
            .height_request(90)
            .accepts_tab(false)
            .build();
        text_view.add_css_class("card");
        let text = text_view.buffer();
        text_box.append(&text_view);
        text_box.append(&label(
            "Select a nearby device below to send this message.",
            "body",
        ));
        send.append(&text_box);
        let wifi_list = gtk::ListBox::new();
        wifi_list.add_css_class("boxed-list");
        wifi_list.set_selection_mode(gtk::SelectionMode::None);
        wifi_list.set_visible(false);
        let wifi_preview = adw::ActionRow::builder().use_markup(false).build();
        wifi_preview.add_prefix(&icon("network-wireless-symbolic", 24));
        let edit_wifi = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text(tr("Edit Wi-Fi Details"))
            .valign(gtk::Align::Center)
            .build();
        edit_wifi.add_css_class("flat");
        wifi_preview.add_suffix(&edit_wifi);
        let clear_wifi = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text(tr("Clear Wi-Fi Details"))
            .valign(gtk::Align::Center)
            .build();
        clear_wifi.add_css_class("flat");
        clear_wifi.add_css_class("circular");
        wifi_preview.add_suffix(&clear_wifi);
        wifi_list.append(&wifi_preview);
        send.append(&wifi_list);
        let nearby = adw::PreferencesGroup::builder()
            .title(tr("Nearby Devices"))
            .build();
        let discovery_state = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let discovery_spinner = gtk::Spinner::builder()
            .width_request(16)
            .height_request(16)
            .build();
        let device_count = label("Starting…", "body");
        discovery_state.append(&discovery_spinner);
        discovery_state.append(&device_count);
        nearby.set_header_suffix(Some(&discovery_state));
        let device_list = gtk::ListBox::new();
        device_list.add_css_class("boxed-list");
        device_list.set_selection_mode(gtk::SelectionMode::None);
        device_list.set_visible(false);
        nearby.add(&device_list);
        let device_empty = vertical(8);
        margins(&device_empty, 14);
        device_empty.append(&icon("network-wireless-symbolic", 24));
        let hint = label("Open Quick Share on your other device", "title-4");
        hint.set_justify(gtk::Justification::Center);
        device_empty.append(&hint);
        let hint = label(
            "Choose Receive and connect to the same Wi-Fi network.
Keep Bluetooth enabled on both devices.",
            "body",
        );
        hint.set_justify(gtk::Justification::Center);
        device_empty.append(&hint);
        nearby.add(&device_empty);
        send.append(&nearby);
        let privacy = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        privacy.set_halign(gtk::Align::Center);
        privacy.append(&icon("channel-secure-symbolic", 14));
        privacy.append(&label("Direct transfers. No cloud. No account.", "caption"));
        send.append(&privacy);
        Self::add_page(&stack, &send, "send", mark("Send"), "send-to-symbolic");

        let receive = vertical(20);
        margins(&receive, 24);
        receive.append(&label("Receive from Nearby Devices", "page-title"));
        let receive_intro = label("Turn on visibility, then share to this computer.", "body");
        receive_intro.set_justify(gtk::Justification::Center);
        receive.append(&receive_intro);
        let visibility_list = gtk::ListBox::new();
        visibility_list.add_css_class("boxed-list");
        visibility_list.set_selection_mode(gtk::SelectionMode::None);
        let visible_row = adw::ActionRow::builder()
            .use_markup(false)
            .title(tr("Visible to nearby devices"))
            .subtitle(tr("Starting Quick Share…"))
            .build();
        visible_row.add_prefix(&icon("view-reveal-symbolic", 22));
        let visible = gtk::Switch::builder()
            .valign(gtk::Align::Center)
            .sensitive(false)
            .build();
        visible_row.add_suffix(&visible);
        visible_row.set_activatable_widget(Some(&visible));
        visibility_list.append(&visible_row);
        receive.append(&visibility_list);
        let visibility_description = visible_row.clone();
        let name_group = adw::PreferencesGroup::new();
        name_group.set_title(&tr("This Computer"));
        let name_row = adw::ActionRow::builder()
            .use_markup(false)
            .title(&settings.name)
            .subtitle(tr("Your name on nearby devices"))
            .build();
        name_row.add_prefix(&icon("computer-symbolic", 24));
        let edit_name = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text(tr("Edit Device Name"))
            .valign(gtk::Align::Center)
            .build();
        edit_name.add_css_class("flat");
        name_row.add_suffix(&edit_name);
        name_row.set_activatable_widget(Some(&edit_name));
        name_group.add(&name_row);
        let downloads_row = adw::ActionRow::builder()
            .use_markup(false)
            .title(tr("Received files"))
            .subtitle(model::display_path(&settings.downloads))
            .build();
        downloads_row.set_subtitle_lines(1);
        downloads_row.set_tooltip_text(Some(&settings.downloads.to_string_lossy()));
        downloads_row.add_prefix(&icon("folder-download-symbolic", 24));
        let open = gtk::Button::builder()
            .icon_name("folder-open-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text(tr("Open Received Files"))
            .build();
        open.add_css_class("flat");
        downloads_row.add_suffix(&open);
        downloads_row.set_activatable_widget(Some(&open));
        name_group.add(&downloads_row);
        receive.append(&name_group);
        let consent_info = vertical(8);
        consent_info.set_margin_top(8);
        consent_info.append(&icon("channel-secure-symbolic", 24));
        let trust = label(
            "Compare the matching code before accepting.
Every share needs your approval.",
            "body",
        );
        trust.set_justify(gtk::Justification::Center);
        consent_info.append(&trust);
        receive.append(&consent_info);
        Self::add_page(
            &stack,
            &receive,
            "receive",
            mark("Receive"),
            "folder-download-symbolic",
        );

        let activity = vertical(18);
        margins(&activity, 28);
        activity.append(&section("Your Transfers"));
        let activity_empty = adw::StatusPage::builder()
            .icon_name("document-open-recent-symbolic")
            .title(tr("No Transfers Yet"))
            .description(tr(
                "Your transfers appear here.\nFile contents and messages stay out of your history.",
            ))
            .build();
        activity.append(&activity_empty);
        let transfer_list = vertical(12);
        activity.append(&transfer_list);
        let history_list = vertical(12);
        activity.append(&history_list);
        Self::add_page(
            &stack,
            &activity,
            "activity",
            mark("Activity"),
            "document-open-recent-symbolic",
        );
        let ui = Rc::new(Self {
            window,
            app: app.clone(),
            settings: RefCell::new(settings),
            backend,
            toast,
            stack,
            files: RefCell::default(),
            file_list,
            selection_label,
            selection_header,
            composer: text_box,
            text_view,
            text_mode: Cell::new(false),
            wifi: RefCell::default(),
            wifi_list,
            wifi_preview,
            text,
            devices: RefCell::default(),
            device_list,
            device_empty,
            device_count,
            discovery_spinner,
            status,
            visible,
            visibility_description,
            visibility_deadline: Cell::new(None),
            name_row,
            downloads_row,
            preferences_window: glib::WeakRef::new(),
            transfer_list,
            history_list,
            activity_empty,
            transfers: RefCell::default(),
            queue: RefCell::default(),
            next_number: Cell::new(1),
            history: RefCell::new(model::load_history()),
            tray_handle: RefCell::default(),
            tray_generation: Cell::new(0),
            restart_pending: Cell::new(false),
            ready: Cell::new(false),
            port: Cell::new(0),
            timed_visibility: RefCell::default(),
            weak: RefCell::default(),
        });
        *ui.weak.borrow_mut() = Rc::downgrade(&ui);
        let weak = ui.weak();
        edit_wifi.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.compose_wifi();
            }
        });
        let weak = ui.weak();
        clear_wifi.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.wifi.borrow_mut().take();
                ui.wifi_list.set_visible(false);
                ui.text_mode.set(false);
                ui.refresh_files();
            }
        });
        let weak = Rc::downgrade(&ui);
        choose.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.choose_files();
            }
        });
        let weak = Rc::downgrade(&ui);
        clear.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.files.borrow_mut().clear();
                ui.text.set_text(&tr(""));
                ui.refresh_files();
            }
        });
        for button in [folder_button, open] {
            let weak = Rc::downgrade(&ui);
            button.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.open_downloads();
                }
            });
        }
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let weak = Rc::downgrade(&ui);
        drop.connect_drop(move |_, value, _, _| {
            if let (Some(ui), Ok(files)) = (weak.upgrade(), value.get::<gdk::FileList>()) {
                ui.add_files(files.files().iter().filter_map(|f| f.path()).collect());
                true
            } else {
                false
            }
        });
        ui.window.add_controller(drop);
        let weak = Rc::downgrade(&ui);
        ui.visible.connect_active_notify(move |switch| {
            if let Some(ui) = weak.upgrade() {
                ui.set_visible(switch.is_active());
            }
        });
        let weak = Rc::downgrade(&ui);
        ui.window.connect_close_request(move |window| {
            if let Some(ui) = weak.upgrade() {
                if ui.settings.borrow().background {
                    window.set_visible(false);
                    return glib::Propagation::Stop;
                }
                ui.backend.send(Command::Stop);
                ui.app.quit();
            }
            glib::Propagation::Proceed
        });
        let weak = ui.weak();
        share_text.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.compose_text();
            }
        });
        let weak = ui.weak();
        edit_name.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.preferences();
            }
        });
        let weak = ui.weak();
        ui.status.connect_button_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.help();
            }
        });
        ui.actions();
        ui.restore_history();
        if ui.settings.borrow().tray {
            ui.set_tray(true);
        }
        let weak = ui.weak();
        gio::NetworkMonitor::default().connect_network_changed(move |_, available| {
            if let Some(ui) = weak.upgrade() {
                if available {
                    ui.request_restart();
                } else {
                    ui.status
                        .set_title(&tr("Connect to a local network to share"));
                    ui.status.set_revealed(true);
                    ui.notify(&tr(
                        "Your network disconnected. Transfers may be interrupted.",
                    ));
                }
            }
        });
        let weak = Rc::downgrade(&ui);
        glib::spawn_future_local(async move {
            while let Ok(event) = events.recv().await {
                let Some(ui) = weak.upgrade() else {
                    break;
                };
                ui.event(event);
            }
        });
        ui
    }
    fn add_page(
        stack: &adw::ViewStack,
        child: &impl IsA<gtk::Widget>,
        name: &str,
        title: &str,
        icon: &str,
    ) {
        let clamp = adw::Clamp::builder()
            .maximum_size(660)
            .tightening_threshold(480)
            .child(child)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .build();
        stack.add_titled_with_icon(&scroll, Some(name), &tr(title), icon);
    }
    fn notify(&self, message: &str) {
        self.toast.add_toast(adw::Toast::new(&tr(message)));
    }
    fn weak(&self) -> Weak<Self> {
        self.weak.borrow().clone()
    }
    fn actions(&self) {
        for name in [
            "preferences",
            "help",
            "shortcuts",
            "about",
            "quit",
            "choose-files",
            "restart",
            "show-activity",
            "close-window",
            "show-send",
            "show-receive",
            "compose-text",
            "compose-wifi",
        ] {
            let action = gio::SimpleAction::new(name, None);
            let weak = self.weak();
            action.connect_activate(move |_, _| {
                if let Some(ui) = weak.upgrade() {
                    match name {
                        "preferences" => ui.preferences(),
                        "restart" => ui.request_restart(),
                        "show-send" => ui.stack.set_visible_child_name("send"),
                        "show-receive" => ui.stack.set_visible_child_name("receive"),
                        "compose-text" => ui.compose_text(),
                        "compose-wifi" => ui.compose_wifi(),
                        "show-activity" => {
                            ui.stack.set_visible_child_name("activity");
                            ui.window.present();
                        }
                        "close-window" => ui.window.close(),
                        "help" => ui.help(),
                        "shortcuts" => ui.shortcuts(),
                        "choose-files" => ui.choose_files(),
                        "about" => {
                            let about = adw::AboutWindow::builder()
                                .transient_for(&ui.window)
                                .application_name("GNOME Quickshare")
                                .application_icon(APP_ID)
                                .developer_name("Oddship")
                                .version(env!("CARGO_PKG_VERSION"))
                                .license_type(gtk::License::Gpl30)
                                .comments(tr(
                                    "Share a little closer.\nQuick Share for the GNOME desktop.",
                                ))
                                .build();
                            about.add_acknowledgement_section(
                                Some(&tr("Protocol research and implementation")),
                                &[
                                    "Packet — nozwock",
                                    "rquickshare — Martichou",
                                    "NearDrop — grishka",
                                ],
                            );
                            about.present();
                        }
                        "quit" => {
                            ui.backend.send(Command::Stop);
                            ui.app.quit();
                        }
                        _ => {}
                    }
                }
            });
            self.app.add_action(&action);
        }
        let visibility = gio::SimpleAction::new_stateful(
            "visible",
            Some(glib::VariantTy::BOOLEAN),
            &false.to_variant(),
        );
        visibility.set_enabled(false);
        let weak = self.weak();
        visibility.connect_activate(move |_, value| {
            if let (Some(ui), Some(value)) = (weak.upgrade(), value.and_then(|v| v.get::<bool>()))
                && ui.ready.get()
            {
                ui.visible.set_active(value);
            }
        });
        let weak = self.weak();
        visibility.connect_change_state(move |_, value| {
            if let (Some(ui), Some(value)) = (weak.upgrade(), value.and_then(|v| v.get::<bool>()))
                && ui.ready.get()
            {
                ui.visible.set_active(value);
            }
        });
        self.app.add_action(&visibility);
        let name = gio::SimpleAction::new_stateful(
            "device-name",
            None,
            &self.settings.borrow().name.to_variant(),
        );
        name.connect_change_state(|_, _| {});
        self.app.add_action(&name);
        let port = gio::SimpleAction::new_stateful("listen-port", None, &0_u32.to_variant());
        port.connect_change_state(|_, _| {});
        self.app.add_action(&port);
        self.app.set_accels_for_action("app.quit", &["<Control>q"]);
        self.app
            .set_accels_for_action("app.close-window", &["<Control>w"]);
        self.app
            .set_accels_for_action("app.preferences", &["<Control>comma"]);
        self.app
            .set_accels_for_action("app.choose-files", &["<Control>o"]);
        self.app.set_accels_for_action("app.help", &["F1"]);
        self.app
            .set_accels_for_action("app.compose-text", &["<Control>l"]);
        for (action, key) in [
            ("app.show-send", "<Control>1"),
            ("app.show-receive", "<Control>2"),
            ("app.show-activity", "<Control>3"),
        ] {
            self.app.set_accels_for_action(action, &[key]);
        }
    }
    pub fn shutdown(&self) {
        if self.backend.shutdown().is_err() {
            eprintln!("Quickshare backend thread panicked during shutdown");
        }
    }
    fn choose_files(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(tr("Choose files to share"))
            .accept_label(tr("Add Files"))
            .build();
        let weak = self.weak();
        dialog.open_multiple(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let (Some(ui), Ok(files)) = (weak.upgrade(), result) {
                ui.add_files(
                    (0..files.n_items())
                        .filter_map(|i| files.item(i)?.downcast::<gio::File>().ok()?.path())
                        .collect(),
                );
            }
        });
    }
    pub fn add_files(&self, paths: Vec<PathBuf>) {
        if let Err(e) = model::validate_files(&paths) {
            self.notify(&e);
            return;
        }
        let mut files = self.files.borrow_mut();
        for path in paths {
            if !files.contains(&path) {
                files.push(path);
            }
        }
        drop(files);
        self.wifi.borrow_mut().take();
        self.wifi_list.set_visible(false);
        self.text_mode.set(false);
        self.composer.set_visible(false);
        self.refresh_files();
        self.stack.set_visible_child_name("send");
    }
    fn refresh_files(&self) {
        while let Some(child) = self.file_list.first_child() {
            self.file_list.remove(&child);
        }
        let files = self.files.borrow();
        self.file_list
            .set_visible(!files.is_empty() && !self.text_mode.get());
        self.selection_header
            .set_visible(!files.is_empty() && !self.text_mode.get());
        let total = files
            .iter()
            .filter_map(|p| p.metadata().ok())
            .map(|m| m.len())
            .sum();
        self.selection_label.set_text(&if files.is_empty() {
            String::new()
        } else {
            format!(
                "{} · {}",
                model::file_count(files.len()),
                model::bytes(total)
            )
        });
        for path in files.iter() {
            let row = adw::ActionRow::builder()
                .use_markup(false)
                .title(path.file_name().unwrap_or_default().to_string_lossy())
                .subtitle(model::bytes(path.metadata().map(|m| m.len()).unwrap_or(0)))
                .build();
            row.set_title_lines(1);
            row.add_prefix(&icon("text-x-generic-symbolic", 24));
            let remove = gtk::Button::builder()
                .icon_name("window-close-symbolic")
                .tooltip_text(tr("Remove File"))
                .valign(gtk::Align::Center)
                .build();
            remove.add_css_class("flat");
            remove.add_css_class("circular");
            let weak = self.weak();
            let path = path.clone();
            remove.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.files.borrow_mut().retain(|p| p != &path);
                    ui.refresh_files();
                }
            });
            row.add_suffix(&remove);
            self.file_list.append(&row);
        }
    }
    fn set_visible(&self, value: bool) {
        if let Some(timer) = self.timed_visibility.borrow_mut().take() {
            timer.remove();
        }
        self.backend.send(Command::Visible(value));
        if let Some(action) = self
            .app
            .lookup_action("visible")
            .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
        {
            action.set_state(&value.to_variant());
        }
        self.visibility_deadline
            .set(value.then(|| std::time::Instant::now() + std::time::Duration::from_secs(600)));
        self.visibility_description.set_subtitle(&if value {
            ntr(
                "Visible for {count} more minute",
                "Visible for {count} more minutes",
                10,
            )
        } else {
            tr("Hidden. Turn on to receive.")
        });
        if value {
            let weak = self.weak();
            *self.timed_visibility.borrow_mut() =
                Some(glib::timeout_add_seconds_local(1, move || {
                    let Some(ui) = weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    let remaining = ui
                        .visibility_deadline
                        .get()
                        .map(|deadline| {
                            deadline
                                .saturating_duration_since(std::time::Instant::now())
                                .as_secs()
                        })
                        .unwrap_or(0);
                    if remaining == 0 {
                        ui.timed_visibility.borrow_mut().take();
                        ui.visible.set_active(false);
                        return glib::ControlFlow::Break;
                    }
                    ui.visibility_description.set_subtitle(&ntr(
                        "Visible for {count} more minute",
                        "Visible for {count} more minutes",
                        remaining.div_ceil(60) as usize,
                    ));
                    glib::ControlFlow::Continue
                }));
        }
    }
    fn compose_text(&self) {
        self.wifi.borrow_mut().take();
        self.wifi_list.set_visible(false);
        self.text_mode.set(true);
        self.composer.set_visible(true);
        self.refresh_files();
        self.stack.set_visible_child_name("send");
        self.text_view.grab_focus();
    }
    fn request_restart(&self) {
        if self.transfers.borrow().values().any(|t| !t.terminal.get()) {
            self.restart_pending.set(true);
            self.notify(&tr("Discovery will restart after the current transfer."));
            return;
        }
        self.ready.set(false);
        self.backend
            .send(Command::Restart(self.settings.borrow().clone()));
    }
    fn open_downloads(&self) {
        let file = gio::File::for_path(&self.settings.borrow().downloads);
        let launcher = gtk::FileLauncher::new(Some(&file));
        launcher.launch(Some(&self.window), gio::Cancellable::NONE, |_| {});
    }
    fn event(&self, event: Event) {
        match event {
            Event::Starting => {
                self.ready.set(false);
                if let Some(action) = self
                    .app
                    .lookup_action("visible")
                    .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
                {
                    action.set_enabled(false);
                }
                if let Some(action) = self
                    .app
                    .lookup_action("listen-port")
                    .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
                {
                    action.set_state(&0_u32.to_variant());
                }
                self.visible.set_sensitive(false);
                self.status.set_title(&tr("Starting Quick Share…"));
                self.status.set_revealed(true);
                self.visibility_description
                    .set_subtitle(&tr("Starting Quick Share…"));
                self.devices.borrow_mut().clear();
                self.refresh_devices();
            }
            Event::Ready(port) => {
                self.ready.set(true);
                self.port.set(port);
                if let Some(action) = self
                    .app
                    .lookup_action("listen-port")
                    .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
                {
                    action.set_state(&port.to_variant());
                }
                self.status.set_title(&trf(
                    "Quick Share is ready · Port {port}",
                    &[("port", &port.to_string())],
                ));
                self.visible.set_sensitive(true);
                self.status.set_revealed(false);
                self.name_row.set_title(&self.settings.borrow().name);
                self.downloads_row
                    .set_subtitle(&model::display_path(&self.settings.borrow().downloads));
                if !self.visible.is_active() {
                    self.visibility_description
                        .set_subtitle(&tr("Hidden. Turn on to receive."));
                }
                self.refresh_devices();
                if let Some(action) = self
                    .app
                    .lookup_action("visible")
                    .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
                {
                    action.set_enabled(true);
                }
                self.backend
                    .send(Command::Visible(self.visible.is_active()));
                self.dispatch_next();
            }
            Event::Error(message) => {
                self.status.set_title(&message);
                self.status.set_revealed(true);
                if self.ready.get() {
                    self.notify(&message);
                } else {
                    self.toast.add_toast(
                        adw::Toast::builder()
                            .title(&message)
                            .button_label(tr("Retry"))
                            .action_name("app.restart")
                            .timeout(0)
                            .build(),
                    );
                }
            }
            Event::Device(device) => {
                if device.present.unwrap_or(false) {
                    self.devices.borrow_mut().insert(device.id.clone(), device);
                } else {
                    self.devices.borrow_mut().remove(&device.id);
                }
                self.refresh_devices();
            }
            Event::Transfer(message) => {
                if let Some(client) = message.msg.as_client() {
                    self.transfer(&message.id, client);
                }
            }
        }
    }
    fn refresh_devices(&self) {
        while let Some(child) = self.device_list.first_child() {
            self.device_list.remove(&child);
        }
        let devices = self.devices.borrow();
        self.device_empty.set_visible(devices.is_empty());
        self.device_list.set_visible(!devices.is_empty());
        self.discovery_spinner.set_spinning(self.ready.get());
        self.discovery_spinner.set_visible(devices.is_empty());
        self.device_count.set_text(&if devices.is_empty() {
            if self.ready.get() {
                tr("Searching…")
            } else {
                tr("Starting…")
            }
        } else {
            ntr(
                "{count} nearby device",
                "{count} nearby devices",
                devices.len(),
            )
        });
        let mut sorted: Vec<_> = devices.values().collect();
        sorted.sort_by_key(|d| d.name.clone());
        for device in sorted {
            let row = adw::ActionRow::builder()
                .use_markup(false)
                .title(device.name.as_deref().unwrap_or(&tr("Nearby device")))
                .subtitle(tr("Ready to share"))
                .activatable(true)
                .build();
            row.set_title_lines(1);
            let image = icon("computer-symbolic", 24);
            image.add_css_class("device-avatar");
            row.add_prefix(&image);
            row.add_suffix(&icon("go-next-symbolic", 16));
            let weak = self.weak();
            let id = device.id.clone();
            row.connect_activated(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.send_to(&id);
                }
            });
            self.device_list.append(&row);
        }
    }
    fn send_to(&self, id: &str) {
        if !self.ready.get() {
            self.notify(&tr("Quick Share is still starting."));
            return;
        }
        let text = self
            .text
            .text(&self.text.start_iter(), &self.text.end_iter(), false)
            .to_string();
        let files = self.files.borrow();
        let payload = if let Some(wifi) = self.wifi.borrow().as_ref() {
            if wifi.validate().is_err() {
                return;
            }
            OutboundPayload::Wifi(wifi.clone())
        } else if !self.text_mode.get() && !files.is_empty() {
            match model::validate_files(&files) {
                Ok(paths) => OutboundPayload::Files(paths),
                Err(e) => {
                    self.notify(&e);
                    return;
                }
            }
        } else if self.text_mode.get() && !text.trim().is_empty() {
            if text.len() > 1024 * 1024 {
                self.notify(&tr("Keep messages smaller than 1 MB."));
                return;
            }
            let text_type = if text.starts_with("https://") || text.starts_with("http://") {
                TextPayloadType::Url
            } else {
                TextPayloadType::Text
            };
            OutboundPayload::Text { text, text_type }
        } else {
            self.notify(&tr("Choose files or write a message first."));
            return;
        };
        let Some(device) = self.devices.borrow().get(id).cloned() else {
            return;
        };
        let info = SendInfo {
            id: device.id.clone(),
            addr: device.id,
            name: device.name.unwrap_or_else(|| tr("Nearby device")),
            ob: payload,
        };
        let number = self.next_number.get();
        self.next_number.set(number + 1);
        let row = adw::ActionRow::builder()
            .use_markup(false)
            .title(trf("To {device}", &[("device", &info.name)]))
            .subtitle(tr("Queued · waiting for the current transfer"))
            .build();
        let cancel = gtk::Button::builder()
            .icon_name("process-stop-symbolic")
            .tooltip_text(tr("Remove from Queue"))
            .valign(gtk::Align::Center)
            .build();
        cancel.add_css_class("flat");
        row.add_suffix(&cancel);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        list.append(&row);
        let widget = vertical(0);
        widget.append(&list);
        self.transfer_list.prepend(&widget);
        self.activity_empty.set_visible(false);
        let weak = self.weak();
        cancel.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                let position = ui.queue.borrow().iter().position(|p| p.number == number);
                if let Some(position) = position {
                    let pending = ui.queue.borrow_mut().remove(position).unwrap();
                    ui.transfer_list.remove(&pending.widget);
                }
            }
        });
        self.queue.borrow_mut().push_back(Pending {
            number,
            info,
            widget,
        });
        self.stack.set_visible_child_name("activity");
        self.dispatch_next();
    }
}
