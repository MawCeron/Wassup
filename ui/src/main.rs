mod ipc;
mod window;

use adw::prelude::*;

const APP_ID: &str = "io.github.MawCeron.Wassup";

fn main() -> gtk::glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    // A second launch only re-activates the running instance.
    app.connect_activate(|app| match app.active_window() {
        Some(window) => window.present(),
        None => window::Window::build(app).present(),
    });
    app.run()
}
