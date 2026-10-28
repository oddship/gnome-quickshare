mod backend;
mod i18n;
mod model;
mod tray;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub const APP_ID: &str = "io.github.oddship.GnomeQuickshare";
fn main() -> glib::ExitCode {
    i18n::init();
    if std::env::args().any(|a| a == "--version") {
        println!("gnome-quickshare {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    if std::env::args().any(|a| a == "--help") {
        println!(
            "{}",
            i18n::tr(
                "GNOME Quickshare\n\nUsage: gnome-quickshare [FILES…]\n\n  --background   Start without a window\n  --version      Show version\n  --help         Show this help\n\nFiles can also be dropped into the window or selected in Files."
            )
        );
        return glib::ExitCode::SUCCESS;
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.add_main_option(
        "background",
        glib::Char::from(b'b'),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        &i18n::tr("Start without a window"),
        None,
    );
    let state: Rc<RefCell<Option<Rc<window::Window>>>> = Rc::default();
    let start_hidden = Rc::new(Cell::new(
        std::env::args().any(|a| a == "--background" || a == "-b"),
    ));
    let activate_hidden = start_hidden.clone();
    let state_activate = state.clone();
    app.connect_activate(move |app| {
        let ui = ensure_window(app, &state_activate);
        if !activate_hidden.replace(false) {
            ui.window.present();
        }
    });
    let state_open = state.clone();
    app.connect_open(move |app, files, _| {
        start_hidden.set(false);
        let ui = ensure_window(app, &state_open);
        ui.add_files(files.iter().filter_map(|f| f.path()).collect());
        ui.window.present();
    });
    let exit = app.run();
    if let Some(ui) = state.borrow().as_ref() {
        ui.shutdown();
    }
    exit
}
fn ensure_window(
    app: &adw::Application,
    slot: &Rc<RefCell<Option<Rc<window::Window>>>>,
) -> Rc<window::Window> {
    if let Some(ui) = slot.borrow().as_ref() {
        return ui.clone();
    }
    let ui = window::Window::new(app);
    *slot.borrow_mut() = Some(ui.clone());
    ui
}
