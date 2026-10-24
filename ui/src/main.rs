mod ipc;
mod sticker;
mod window;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "io.github.MawCeron.Wassup";

/// The single window, created on first use and forgotten when closed.
type Slot = Rc<RefCell<Option<Rc<window::Window>>>>;

fn window(app: &adw::Application, slot: &Slot) -> Rc<window::Window> {
    if let Some(w) = slot.borrow().clone() {
        return w;
    }
    let w = window::Window::build(app);
    let weak = Rc::downgrade(slot);
    w.connect_destroy(move || {
        if let Some(slot) = weak.upgrade() {
            slot.take();
        }
    });
    slot.replace(Some(w.clone()));
    w
}

fn main() -> glib::ExitCode {
    // Every launch is forwarded to the running instance, if there is one.
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.add_main_option(
        "toggle",
        glib::Char::from(b't'),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Show the window, or close it if it has focus",
        None,
    );
    let slot: Slot = Rc::default();

    app.connect_activate({
        let slot = slot.clone();
        move |app| window(app, &slot).present()
    });
    app.connect_command_line({
        let slot = slot.clone();
        move |app, cmd| {
            let open = slot.borrow().clone();
            match open {
                Some(w) if cmd.options_dict().contains("toggle") => w.toggle(),
                _ => window(app, &slot).present(),
            }
            glib::ExitCode::SUCCESS
        }
    });

    // Used by the daemon when a notification is clicked.
    let open_chat =
        gio::SimpleAction::new("open-chat", Some(glib::VariantTy::new("(ss)").unwrap()));
    open_chat.connect_activate({
        let app = app.clone();
        move |_, param| {
            let Some((account, chat)) = param.and_then(|p| p.get::<(String, String)>()) else {
                return;
            };
            window(&app, &slot).show_chat(&account, &chat);
        }
    });
    app.add_action(&open_chat);
    app.run()
}
