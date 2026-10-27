//! The sticker picker and sending stickers.

use std::collections::HashMap;

use super::*;
use gettextrs::gettext;

impl Window {
    pub(super) fn show_stickers(self: &Rc<Self>, stickers: Vec<ipc::Sticker>) {
        for (heading, grid) in &self.sticker_sections {
            while let Some(child) = grid.first_child() {
                grid.remove(&child);
            }
            heading.set_visible(false);
        }
        self.sticker_pictures.borrow_mut().clear();
        THUMB_PATHS.with(|p| p.borrow_mut().clear());
        self.sticker_empty.set_visible(stickers.is_empty());
        let account_id = self
            .account
            .borrow()
            .as_ref()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        for sticker in stickers {
            // Favorites first, recent stickers second.
            let section = usize::from(sticker.favorite_id.is_empty());
            let picture = gtk::Picture::builder()
                .content_fit(gtk::ContentFit::Contain)
                .width_request(STICKER_THUMB)
                .height_request(STICKER_THUMB)
                .css_classes(["sticker-image"])
                .overflow(gtk::Overflow::Hidden)
                .build();
            let button = gtk::Button::builder()
                .child(&picture)
                .css_classes(["flat"])
                .build();
            button.update_property(&[gtk::accessible::Property::Label(&gettext("Send Sticker"))]);
            if sticker.path.is_empty() {
                self.sticker_pictures
                    .borrow_mut()
                    .insert(sticker.message_id.clone(), picture.downgrade());
                if self
                    .media_requested
                    .borrow_mut()
                    .insert(sticker.message_id.clone())
                {
                    self.send(request::Kind::DownloadMedia(ipc::DownloadMedia {
                        account_id: account_id.clone(),
                        chat_jid: sticker.chat_jid.clone(),
                        message_id: sticker.message_id.clone(),
                    }));
                }
            } else {
                show_thumb(&picture, &sticker.path);
            }
            // An animated sticker moves while the pointer is over it.
            let hover = gtk::EventControllerMotion::new();
            hover.connect_enter(glib::clone!(
                #[weak]
                picture,
                move |_, _, _| play_thumb(&picture)
            ));
            hover.connect_leave(glib::clone!(
                #[weak]
                picture,
                move |_| stop_thumb(&picture)
            ));
            button.add_controller(hover);
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = ui)]
                self,
                move |_| ui.send_sticker(&sticker)
            ));
            let (heading, grid) = &self.sticker_sections[section];
            heading.set_visible(true);
            grid.append(&button);
        }
    }

    pub(super) fn choose_sticker_image(self: &Rc<Self>) {
        self.sticker_popover.popdown();
        let images = gtk::FileFilter::new();
        images.set_name(Some(&gettext("Images")));
        images.add_mime_type("image/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&images);
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Create a Sticker"))
            .modal(true)
            .filters(&filters)
            .build();
        let ui = Rc::downgrade(self);
        dialog.open(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                if let (Some(ui), Some(path)) = (ui.upgrade(), result.ok().and_then(|f| f.path())) {
                    glib::spawn_future_local(async move { ui.send_new_sticker(&path).await });
                }
            },
        );
    }

    /// Makes a sticker from an image and sends it to the open chat.
    pub(super) async fn send_new_sticker(&self, image: &std::path::Path) {
        let webp = match crate::sticker::from_file(image).await {
            Ok(webp) => webp,
            // TRANSLATORS: {} is the reason.
            Err(e) => return self.toast(&gettext("Couldn't make a sticker: {}").replace("{}", &e)),
        };
        let Some((account_id, chat_jid)) = self.open_ids() else {
            return;
        };
        // The daemon reads the file from disk; media never goes over the socket.
        let dir = outgoing_dir();
        let file = dir.join(format!("sticker-{}.webp", glib::monotonic_time()));
        let written = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .and_then(|_| std::fs::write(&file, webp));
        if let Err(e) = written {
            return self
                .toast(&gettext("Couldn't make a sticker: {}").replace("{}", &e.to_string()));
        }
        let request = request::Kind::SendMedia(ipc::SendMedia {
            account_id,
            chat_jid,
            path: file.to_string_lossy().into_owned(),
            caption: String::new(),
            as_sticker: true,
            as_document: false,
            voice_note: false,
        });
        match self.send(request) {
            Some(id) => {
                let toast = adw::Toast::builder()
                    .title(gettext("Sending sticker…"))
                    .timeout(0)
                    .build();
                self.toasts.add_toast(toast.clone());
                self.uploads.borrow_mut().insert(id, toast);
                self.sticker_files.borrow_mut().insert(id, file);
            }
            None => {
                std::fs::remove_file(file).ok();
            }
        }
    }

    pub(super) fn send_sticker(&self, sticker: &ipc::Sticker) {
        self.sticker_popover.popdown();
        let Some((account_id, chat_jid)) = self.open_ids() else {
            return;
        };
        self.send(request::Kind::SendSticker(ipc::SendSticker {
            account_id,
            chat_jid,
            source_chat_jid: sticker.chat_jid.clone(),
            source_message_id: sticker.message_id.clone(),
            favorite_id: sticker.favorite_id.clone(),
        }));
    }
}

/// Where stickers made here wait for the daemon to send them.
pub(super) fn outgoing_dir() -> std::path::PathBuf {
    glib::user_cache_dir().join("wassup").join("outgoing")
}

/// Size of a sticker in the picker, in pixels.
pub(super) const STICKER_THUMB: i32 = 64;

thread_local! {
    /// The file shown in each thumbnail of the picker, by the picture's address.
    static THUMB_PATHS: RefCell<HashMap<usize, String>> = RefCell::default();
    /// The thumbnail the pointer is over, and the image to put back when it leaves.
    static HOVERED: RefCell<Option<(usize, Option<gdk::Paintable>)>> = const { RefCell::new(None) };
}

fn play_thumb(picture: &gtk::Picture) {
    let key = picture.as_ptr() as usize;
    let Some(path) = THUMB_PATHS.with(|p| p.borrow().get(&key).cloned()) else {
        return;
    };
    HOVERED.with(|h| *h.borrow_mut() = Some((key, picture.paintable())));
    animation::play(picture, &path, move |_| {
        HOVERED.with(|h| {
            h.borrow()
                .as_ref()
                .is_some_and(|(hovered, _)| *hovered == key)
        })
    });
}

fn stop_thumb(picture: &gtk::Picture) {
    let key = picture.as_ptr() as usize;
    let still = HOVERED.with(|h| match h.borrow_mut().take() {
        Some((hovered, still)) if hovered == key => Some(still),
        other => {
            *h.borrow_mut() = other;
            None
        }
    });
    if let Some(still) = still {
        picture.set_paintable(still.as_ref());
    }
}

pub(super) fn show_thumb(picture: &gtk::Picture, path: &str) {
    THUMB_PATHS.with(|p| {
        p.borrow_mut()
            .insert(picture.as_ptr() as usize, path.to_owned())
    });
    let scale = picture.scale_factor().max(1);
    let picture = picture.downgrade();
    load_texture(path, STICKER_THUMB, STICKER_THUMB, scale, move |texture| {
        if let Some(picture) = picture.upgrade() {
            picture.set_paintable(Some(&texture));
        }
    });
}
