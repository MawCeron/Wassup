//! The chat background: the doodles, a plain color, or the user's picture.

use super::*;
use gettextrs::gettext;

/// The background choices, as config.json stores them, in Preferences order.
pub(super) const BACKGROUNDS: [&str; 3] = ["", "plain", "picture"];

/// Largest side a background picture is kept at: enough for a full-screen
/// window, without holding a 12-megapixel photo in memory.
const PICTURE_MAX: i32 = 1920;

/// Where the user's background picture is kept.
fn picture_dir() -> std::path::PathBuf {
    glib::user_data_dir().join("wassup").join("background")
}

/// The user's background picture, if one was chosen.
fn picture() -> Option<std::path::PathBuf> {
    std::fs::read_dir(picture_dir())
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "jpg"))
}

impl Window {
    /// Shows the chosen background in the conversation, for the light or
    /// dark style in use.
    pub(super) fn update_chat_background(&self) {
        let dark = adw::StyleManager::default().is_dark();
        let choice = self.chat_background.borrow().clone();
        let class = match (choice.as_str(), picture()) {
            ("plain", _) => "chat-bg-plain",
            ("picture", Some(path)) => {
                // Dimmed a little, so bubbles stay readable on any picture.
                let veil = if dark { 0.35 } else { 0.15 };
                self.background_css.load_from_string(&format!(
                    ".chat-bg-picture {{ background: linear-gradient(alpha(var(--view-bg-color), {veil}), \
                     alpha(var(--view-bg-color), {veil})), url(\"{}\") center / cover no-repeat, var(--view-bg-color); }}",
                    gio::File::for_path(path).uri()
                ));
                "chat-bg-picture"
            }
            _ if dark => "chat-bg-dark",
            _ => "chat-bg-light",
        };
        self.message_scroll.set_css_classes(&["chat-bg", class]);
    }

    /// Lets the user pick a picture, kept scaled down as a JPEG. `done`
    /// says whether one was set.
    pub(super) fn choose_background(self: &Rc<Self>, done: impl FnOnce(bool) + 'static) {
        let images = gtk::FileFilter::new();
        images.set_name(Some(&gettext("Images")));
        images.add_mime_type("image/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&images);
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Choose a Background"))
            .modal(true)
            .filters(&filters)
            .build();
        let ui = Rc::downgrade(self);
        dialog.open(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                let (Some(ui), Ok(file)) = (ui.upgrade(), result) else {
                    return done(false);
                };
                glib::spawn_future_local(async move {
                    let saved = save_picture(&file).await;
                    if let Err(e) = &saved {
                        // TRANSLATORS: {} is the reason.
                        ui.toast(&gettext("Couldn't use that picture: {}").replace("{}", e));
                    }
                    done(saved.is_ok());
                });
            },
        );
    }
}

/// Decodes a picture scaled down and stores it as the background, under a
/// new name each time so GTK does not show a cached older one.
async fn save_picture(file: &gio::File) -> Result<(), String> {
    let stream = file
        .read_future(glib::Priority::DEFAULT)
        .await
        .map_err(|e| e.to_string())?;
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale_future(
        &stream,
        PICTURE_MAX,
        PICTURE_MAX,
        true,
    )
    .await
    .map_err(|e| e.to_string())?;
    let dir = picture_dir();
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|e| e.to_string())?;
    let old: Vec<_> = std::fs::read_dir(&dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    let path = dir.join(format!("background-{}.jpg", glib::monotonic_time()));
    pixbuf
        .savev(&path, "jpeg", &[("quality", "85")])
        .map_err(|e| e.to_string())?;
    for p in old {
        std::fs::remove_file(p).ok();
    }
    Ok(())
}
