mod emoji_font;
mod ipc;
mod sticker;
mod window;

use gettextrs::gettext;
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

/// The text after a key of config.json. The daemon validates the file
/// strictly and refuses to start on a malformed one, so a plain scan for
/// the key is enough here.
fn config_value<'a>(config: &'a str, key: &str) -> Option<&'a str> {
    let (_, rest) = config.split_once(&format!("\"{key}\""))?;
    Some(rest.trim_start().strip_prefix(':')?.trim_start())
}

/// Whether config.json asks for GPU rendering.
fn wants_gpu(config: &str) -> bool {
    config_value(config, "gpu_rendering").is_some_and(|v| v.starts_with("true"))
}

/// The language chosen in config.json, if not the system's.
fn language(config: &str) -> Option<&str> {
    let (code, _) = config_value(config, "language")?
        .strip_prefix('"')?
        .split_once('"')?;
    (!code.is_empty()).then_some(code)
}

/// Loads the translations for the chosen language, or the user's. They
/// live in the first XDG data directory that has Wassup's, as for the
/// daemon: ~/.local/share/locale during development, the system one when
/// installed.
fn init_locale(config: &str) {
    use gettextrs::{
        LocaleCategory, bind_textdomain_codeset, bindtextdomain, setlocale, textdomain,
    };
    // SAFETY: called first thing in main, before any other thread exists.
    unsafe {
        // gettext, GTK and libadwaita all read LANGUAGE first.
        if let Some(code) = language(config) {
            std::env::set_var("LANGUAGE", code);
        }
        setlocale(LocaleCategory::LcAll, "");
    }
    let dir = std::iter::once(glib::user_data_dir())
        .chain(glib::system_data_dirs())
        .map(|d| d.join("locale"))
        .find(|d| {
            std::fs::read_dir(d).is_ok_and(|langs| {
                langs
                    .flatten()
                    .any(|l| l.path().join("LC_MESSAGES/wassup.mo").exists())
            })
        });
    if let Some(dir) = dir {
        let _ = bindtextdomain("wassup", dir);
    }
    let _ = bind_textdomain_codeset("wassup", "UTF-8");
    let _ = textdomain("wassup");
}

/// Draws in software unless the user enabled the GPU. GPU rendering loads
/// the graphics drivers into the process, which on some machines costs
/// over 50 MB, while software rendering keeps 60 fps for a chat window.
/// See docs/performance.md.
fn choose_renderer(config: &str) {
    let gpu = wants_gpu(config);
    let user_chose =
        std::env::var_os("GSK_RENDERER").is_some() || std::env::var_os("GDK_DISABLE").is_some();
    if !gpu && !user_chose {
        // SAFETY: called first thing in main, before any other thread exists.
        unsafe {
            std::env::set_var("GDK_DISABLE", "gl,vulkan");
            std::env::set_var("GSK_RENDERER", "cairo");
        }
    }
}

fn main() -> glib::ExitCode {
    let config = std::fs::read_to_string(glib::user_config_dir().join("wassup/config.json"))
        .unwrap_or_default();
    choose_renderer(&config);
    emoji_font::use_bundled_font();
    init_locale(&config);
    gio::resources_register_include!("wassup.gresource").expect("bundled resources");
    // Every launch is forwarded to the running instance, if there is one.
    let app = adw::Application::builder()
        .application_id(APP_ID)
        // Where GTK finds the bundled icons, whatever the app id.
        .resource_base_path("/io/github/MawCeron/Wassup")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.add_main_option(
        "toggle",
        glib::Char::from(b't'),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        &gettext("Show the window, or close it if it has focus"),
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

#[cfg(test)]
mod tests {
    use super::{language, wants_gpu};

    #[test]
    fn reads_the_gpu_option() {
        assert!(wants_gpu(r#"{"gpu_rendering": true}"#));
        assert!(wants_gpu(
            "{\n  \"media_cache_mb\": 200,\n  \"gpu_rendering\" :true\n}"
        ));
        assert!(!wants_gpu(r#"{"gpu_rendering": false}"#));
        assert!(!wants_gpu(r#"{"media_cache_mb": 200}"#));
        assert!(!wants_gpu(""));
    }

    #[test]
    fn reads_the_language() {
        assert_eq!(language(r#"{"language": "pt_BR"}"#), Some("pt_BR"));
        assert_eq!(
            language(r#"{"language":"es", "gpu_rendering": true}"#),
            Some("es")
        );
        assert_eq!(language(r#"{"language": ""}"#), None);
        assert_eq!(language(r#"{"gpu_rendering": true}"#), None);
    }
}
