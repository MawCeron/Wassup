//! The photo viewer, over the whole window.

use super::*;
use gettextrs::gettext;

/// The viewer's widgets, built once and shown over the window on demand.
pub(super) struct Viewer {
    pub(super) root: gtk::Overlay,
    picture: gtk::Picture,
    spinner: adw::Spinner,
    title: gtk::Label,
    caption: gtk::Label,
    prev: gtk::Button,
    next: gtk::Button,
    close: gtk::Button,
    /// Id of the message on show, while the viewer is open.
    msg: RefCell<Option<String>>,
}

impl Viewer {
    pub(super) fn new() -> Self {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .hexpand(true)
            .vexpand(true)
            .build();
        picture.update_property(&[gtk::accessible::Property::Label(&gettext("Photo"))]);
        let spinner = adw::Spinner::builder()
            .width_request(48)
            .height_request(48)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        let button = |icon: &str, tooltip: String, halign, valign| {
            gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tooltip)
                .halign(halign)
                .valign(valign)
                .margin_start(12)
                .margin_end(12)
                .margin_top(12)
                .margin_bottom(12)
                .css_classes(["osd", "circular"])
                .build()
        };
        let prev = button(
            "go-previous-symbolic",
            gettext("Previous Photo"),
            gtk::Align::Start,
            gtk::Align::Center,
        );
        let next = button(
            "go-next-symbolic",
            gettext("Next Photo"),
            gtk::Align::End,
            gtk::Align::Center,
        );
        let close = button(
            "window-close-symbolic",
            gettext("Close"),
            gtk::Align::End,
            gtk::Align::Start,
        );
        let title = gtk::Label::builder()
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_start(18)
            .margin_top(20)
            .css_classes(["viewer-text", "viewer-caption", "heading"])
            .build();
        let caption = gtk::Label::builder()
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .margin_bottom(18)
            .margin_start(72)
            .margin_end(72)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .css_classes(["viewer-text", "viewer-caption"])
            .build();
        let root = gtk::Overlay::builder()
            .child(&picture)
            .visible(false)
            .css_classes(["viewer"])
            .build();
        for widget in [
            spinner.upcast_ref::<gtk::Widget>(),
            title.upcast_ref(),
            caption.upcast_ref(),
            prev.upcast_ref(),
            next.upcast_ref(),
            close.upcast_ref(),
        ] {
            root.add_overlay(widget);
        }
        Self {
            root,
            picture,
            spinner,
            title,
            caption,
            prev,
            next,
            close,
            msg: RefCell::default(),
        }
    }
}

impl Window {
    /// Wires the viewer's buttons and keys; called once the window exists.
    pub(super) fn connect_viewer(self: &Rc<Self>) {
        let v = &self.viewer;
        v.close.connect_clicked(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_| ui.close_viewer()
        ));
        v.prev.connect_clicked(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_| ui.step_viewer(-1)
        ));
        v.next.connect_clicked(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_| ui.step_viewer(1)
        ));
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                match key {
                    gdk::Key::Escape => ui.close_viewer(),
                    gdk::Key::Left => ui.step_viewer(-1),
                    gdk::Key::Right => ui.step_viewer(1),
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            }
        ));
        v.root.add_controller(keys);
    }

    /// Opens the viewer on a photo of the open chat.
    pub(super) fn show_photo(&self, msg: &ipc::Message) {
        let v = &self.viewer;
        let first = !v.root.is_visible();
        v.msg.replace(Some(msg.id.clone()));
        v.title.set_label(&format!(
            "{} · {}",
            if msg.from_me {
                gettext("You")
            } else if msg.sender_name.is_empty() {
                user_label(&msg.sender_jid)
            } else {
                msg.sender_name.clone()
            },
            time_of_day(msg.timestamp_ms)
        ));
        v.caption.set_label(&msg.text);
        v.caption.set_visible(!msg.text.is_empty());
        let photos = self.photo_ids();
        let pos = photos.iter().position(|id| *id == msg.id);
        v.prev.set_sensitive(pos.is_some_and(|p| p > 0));
        v.next
            .set_sensitive(pos.is_some_and(|p| p + 1 < photos.len()));
        v.picture.set_paintable(None::<&gdk::Paintable>);
        v.spinner.set_visible(true);
        match msg.media.as_ref().map(|m| m.path.as_str()) {
            Some("") => self.request_media(msg), // photo_ready shows it.
            Some(path) => self.load_photo(&msg.id, path),
            None => {}
        }
        if first {
            v.root.set_visible(true);
            // The window underneath must not take focus or clicks meanwhile.
            self.page.set_can_focus(false);
            self.page.set_can_target(false);
            v.close.grab_focus();
        }
    }

    /// Decodes the photo at the window's size, outside the thumbnail
    /// cache: it is large, and freed when the viewer moves on.
    fn load_photo(&self, msg_id: &str, path: &str) {
        let scale = self.window.scale_factor().max(1);
        let (width, height) = (self.window.width() * scale, self.window.height() * scale);
        let (viewer, id, path) = (
            Rc::downgrade(&self.viewer),
            msg_id.to_owned(),
            path.to_owned(),
        );
        glib::spawn_future_local(async move {
            let texture = decode_texture(&path, width, height).await;
            let Some(v) = viewer.upgrade() else { return };
            if v.msg.borrow().as_deref() != Some(id.as_str()) {
                return; // Moved on meanwhile.
            }
            v.spinner.set_visible(false);
            v.picture.set_paintable(texture.as_ref());
        });
    }

    /// Called when a download ends, in case the viewer waits for it.
    pub(super) fn photo_ready(&self, ready: &ipc::MediaReady) {
        if self.viewer.msg.borrow().as_deref() != Some(ready.message_id.as_str()) {
            return;
        }
        if ready.error.is_empty() {
            self.load_photo(&ready.message_id, &ready.path);
        } else {
            self.close_viewer();
            self.toast(&gettext("Couldn't download the file"));
        }
    }

    pub(super) fn step_viewer(&self, delta: isize) {
        let photos = self.photo_ids();
        let current = self.viewer.msg.borrow().clone();
        let Some(pos) = photos.iter().position(|id| Some(id) == current.as_ref()) else {
            return;
        };
        let Some(id) = pos.checked_add_signed(delta).and_then(|p| photos.get(p)) else {
            return;
        };
        if let Some(msg) = self.message(id) {
            self.show_photo(&msg);
        }
    }

    pub(super) fn close_viewer(&self) {
        let v = &self.viewer;
        if !v.root.is_visible() {
            return;
        }
        v.root.set_visible(false);
        v.msg.replace(None);
        v.picture.set_paintable(None::<&gdk::Paintable>); // Frees the large texture.
        self.page.set_can_focus(true);
        self.page.set_can_target(true);
        self.composer.grab_focus();
    }

    /// Ids of the open chat's loaded photos, oldest first.
    fn photo_ids(&self) -> Vec<String> {
        (0..self.messages.n_items())
            .filter_map(|i| self.messages.item(i).and_downcast::<glib::BoxedAnyObject>())
            .filter_map(|item| {
                let msg = item.borrow::<ipc::Message>();
                let photo = msg
                    .media
                    .as_ref()
                    .is_some_and(|m| m.kind() == ipc::MediaKind::Image);
                photo.then(|| msg.id.clone())
            })
            .collect()
    }

    pub(super) fn message(&self, id: &str) -> Option<ipc::Message> {
        (0..self.messages.n_items())
            .filter_map(|i| self.messages.item(i).and_downcast::<glib::BoxedAnyObject>())
            .find(|item| item.borrow::<ipc::Message>().id == id)
            .map(|item| item.borrow::<ipc::Message>().clone())
    }
}
