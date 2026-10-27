//! The preview shown before sending a file.

use super::*;
use gettextrs::gettext;
use std::io::Read;

/// Largest size of the image in the preview, in pixels.
const PREVIEW_WIDTH: i32 = 480;
const PREVIEW_HEIGHT: i32 = 360;

impl Window {
    /// Shows a chosen file over the conversation, with a caption field.
    /// Nothing is sent until the user presses Send; Escape discards it.
    pub(super) fn preview_file(self: &Rc<Self>, path: &str, as_document: bool) {
        // As a document, a photo or video goes as a file, like any other.
        let photo = !as_document && is_photo(path);
        let video = !as_document && !photo && is_video(path);
        let preview: gtk::Widget = if photo {
            let picture = gtk::Picture::builder()
                .content_fit(gtk::ContentFit::Contain)
                .height_request(PREVIEW_HEIGHT)
                .css_classes(["media"])
                .build();
            picture.update_property(&[gtk::accessible::Property::Label(&gettext("Photo"))]);
            let weak = picture.downgrade();
            let scale = self.window.scale_factor().max(1);
            load_texture(path, PREVIEW_WIDTH, PREVIEW_HEIGHT, scale, move |texture| {
                if let Some(picture) = weak.upgrade() {
                    picture.set_paintable(Some(&texture));
                }
            });
            picture.upcast()
        } else {
            document_card(path, video).upcast()
        };
        // Whatever was typed before choosing the file becomes its caption.
        let buffer = self.composer.buffer();
        let (start, end) = buffer.bounds();
        let caption = gtk::Entry::builder()
            .placeholder_text(gettext("Add a caption"))
            .text(buffer.text(&start, &end, false).trim())
            .hexpand(true)
            .build();
        let send = gtk::Button::builder()
            .icon_name("wassup-send-symbolic")
            .tooltip_text(gettext("Send"))
            .css_classes(["circular", "suggested-action"])
            .build();
        let bottom = gtk::Box::builder()
            .spacing(6)
            .margin_top(6)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        bottom.append(&caption);
        bottom.append(&send);
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .margin_start(12)
            .margin_end(12)
            .valign(gtk::Align::Center)
            .vexpand(true)
            .build();
        content.append(&preview);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&content));
        view.add_bottom_bar(&bottom);
        let dialog = adw::Dialog::builder()
            .title(if photo {
                gettext("Send Photo")
            } else if video {
                gettext("Send Video")
            } else {
                gettext("Send Document")
            })
            .content_width(PREVIEW_WIDTH + 24)
            .child(&view)
            .build();

        let on_send = Rc::new(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            #[weak]
            dialog,
            #[weak]
            caption,
            #[strong(rename_to = path)]
            path.to_owned(),
            move || {
                if ui.send_file(&path, caption.text().trim(), as_document) {
                    ui.composer.buffer().set_text("");
                }
                dialog.close();
            }
        ));
        send.connect_clicked({
            let on_send = on_send.clone();
            move |_| on_send()
        });
        caption.connect_activate(move |_| on_send());
        dialog.present(Some(&self.window));
        caption.grab_focus();
    }
}

/// A file's icon, name and size, as for a document in a message.
fn document_card(path: &str, video: bool) -> gtk::Box {
    let file = std::path::Path::new(path);
    let name = gtk::Label::builder()
        .label(
            file.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default(),
        )
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .css_classes(["heading"])
        .build();
    let size = gtk::Label::builder()
        .label(glib::format_size(file.metadata().map_or(0, |m| m.len())))
        .xalign(0.0)
        .css_classes(["caption", "dim-label"])
        .build();
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.append(&name);
    labels.append(&size);
    let card = gtk::Box::builder()
        .spacing(10)
        .margin_top(24)
        .margin_bottom(24)
        .halign(gtk::Align::Center)
        .css_classes(["document"])
        .build();
    card.append(
        &gtk::Image::builder()
            .icon_name(if video {
                "wassup-media-symbolic"
            } else {
                "wassup-document-symbolic"
            })
            .pixel_size(32)
            .build(),
    );
    card.append(&labels);
    card
}

/// Whether the daemon will send the file as a photo: JPEG and PNG, told
/// by their content as the daemon does, not by the name.
fn is_photo(path: &str) -> bool {
    let mut head = [0; 8];
    let read = std::fs::File::open(path).and_then(|mut f| f.read(&mut head));
    read.is_ok_and(|n| {
        head[..n].starts_with(b"\xff\xd8\xff") || head[..n].starts_with(b"\x89PNG\r\n\x1a\n")
    })
}

/// Whether the daemon will send the file as a video: an MP4 file, by the
/// same rule as Go's http.DetectContentType, which the daemon uses.
fn is_video(path: &str) -> bool {
    let mut head = Vec::with_capacity(512);
    let read = std::fs::File::open(path).and_then(|f| f.take(512).read_to_end(&mut head));
    if read.is_err() || head.len() < 12 || &head[4..8] != b"ftyp" {
        return false;
    }
    let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    if size > head.len() || !size.is_multiple_of(4) {
        return false;
    }
    // Major brand, then compatible brands; the minor version at 12 is skipped.
    (8..size)
        .step_by(4)
        .filter(|&at| at != 12)
        .any(|at| head[at..].starts_with(b"mp4"))
}

#[cfg(test)]
mod tests {
    use super::{is_photo, is_video};

    #[test]
    fn photos_are_told_by_content() {
        let dir = std::env::temp_dir().join(format!("wassup-preview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, data: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, data).unwrap();
            path.to_string_lossy().into_owned()
        };
        assert!(is_photo(&write("a.bin", b"\xff\xd8\xff\xe0rest")));
        assert!(is_photo(&write("b", b"\x89PNG\r\n\x1a\nrest")));
        assert!(!is_photo(&write("c.jpg", b"%PDF-1.7")));
        assert!(!is_photo(&dir.join("missing").to_string_lossy()));
        let mp4 = write("d.mov", b"\0\0\0\x18ftypisom\0\0\x02\0mp41isommore");
        assert!(is_video(&mp4) && !is_photo(&mp4));
        assert!(!is_video(&write(
            "e.mp4",
            b"\0\0\0\x14ftypqt  \0\0\0\0qt  "
        )));
        assert!(!is_video(&write("f.mp4", b"short")));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
