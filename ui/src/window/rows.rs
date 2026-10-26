//! The list rows of chats and messages, and the pictures in them.

use super::*;

pub(super) fn chat_row_factory(owner: Owner) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let avatar = adw::Avatar::builder().size(40).show_initials(true).build();
        let title = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["heading"])
            .build();
        let time = gtk::Label::builder()
            .css_classes(["dim-label", "caption"])
            .build();
        let unread = gtk::Label::builder()
            .css_classes(["caption-heading", "accent"])
            .build();
        let row = gtk::Box::builder()
            .spacing(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        row.append(&avatar);
        row.append(&title);
        row.append(&time);
        row.append(&unread);
        item.downcast_ref::<gtk::ListItem>()
            .unwrap()
            .set_child(Some(&row));
    });
    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let chat = item.item().and_downcast::<glib::BoxedAnyObject>().unwrap();
        let chat = chat.borrow::<ipc::Chat>();
        let row = item.child().unwrap();
        let avatar = row.first_child().and_downcast::<adw::Avatar>().unwrap();
        let title = avatar.next_sibling().and_downcast::<gtk::Label>().unwrap();
        avatar_placeholder(&avatar, &chat);
        if chat.avatar_path.is_empty() {
            if let Some(ui) = owner.get().and_then(std::rc::Weak::upgrade) {
                ui.request_avatar(&chat);
            }
        } else {
            let (list_item, jid) = (item.downgrade(), chat.jid.clone());
            show_avatar(&avatar, &chat.avatar_path, move || {
                list_item
                    .upgrade()
                    .and_then(|i| i.item())
                    .and_downcast::<glib::BoxedAnyObject>()
                    .is_some_and(|c| c.borrow::<ipc::Chat>().jid == jid)
            });
        }
        let time = title.next_sibling().and_downcast::<gtk::Label>().unwrap();
        let unread = time.next_sibling().and_downcast::<gtk::Label>().unwrap();
        title.set_label(&chat_title(&chat));
        time.set_label(&format_time(chat.last_message_at_ms));
        unread.set_label(&chat.unread_count.to_string());
        unread.set_visible(chat.unread_count > 0);
    });
    factory
}

pub(super) fn show_status(label: &gtk::Label, status: ipc::MessageStatus) {
    let (marks, tooltip, read) = match status {
        ipc::MessageStatus::Sent => ("✓", "Sent", false),
        ipc::MessageStatus::Delivered => ("✓✓", "Delivered", false),
        ipc::MessageStatus::Read => ("✓✓", "Read", true),
        ipc::MessageStatus::Unspecified => ("", "", false),
    };
    label.set_label(marks);
    label.set_tooltip_text(Some(tooltip));
    label.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    label.set_css_classes(if read {
        &["caption", "accent"]
    } else {
        &["caption", "dim-label"]
    });
}

pub(super) fn message_row_factory(rows: BoundRows, owner: Owner) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let opener = owner.clone();
    factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        item.set_activatable(false);
        // A focused row would scroll the list to itself.
        item.set_focusable(false);
        item.set_selectable(false);
        let sender = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["caption-heading", "accent"])
            .build();
        let picture = gtk::Picture::builder()
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Contain)
            .halign(gtk::Align::Start)
            .css_classes(["media"])
            .cursor(&gdk::Cursor::from_name("pointer", None).unwrap())
            .build();
        // Clicking opens the file with the desktop's default application.
        let on_click = {
            let (list_item, owner) = (item.downgrade(), opener.clone());
            move || {
                let msg = list_item
                    .upgrade()
                    .and_then(|i| i.item())
                    .and_downcast::<glib::BoxedAnyObject>();
                if let (Some(msg), Some(ui)) = (msg, owner.get().and_then(std::rc::Weak::upgrade)) {
                    ui.open_media(&msg.borrow::<ipc::Message>());
                }
            }
        };
        let open = gtk::GestureClick::new();
        open.connect_released({
            let on_click = on_click.clone();
            move |_, _, _, _| on_click()
        });
        picture.add_controller(open);
        let doc_name = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["heading"])
            .build();
        let doc_size = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["caption", "dim-label"])
            .build();
        let doc_labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
        doc_labels.append(&doc_name);
        doc_labels.append(&doc_size);
        let document = gtk::Box::builder()
            .spacing(10)
            .css_classes(["document"])
            .cursor(&gdk::Cursor::from_name("pointer", None).unwrap())
            .build();
        document.append(
            &gtk::Image::builder()
                .icon_name("x-office-document-symbolic")
                .pixel_size(32)
                .build(),
        );
        document.append(&doc_labels);
        let open_doc = gtk::GestureClick::new();
        open_doc.connect_released(move |_, _, _, _| on_click());
        document.add_controller(open_doc);
        let text = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .build();
        let time = gtk::Label::builder()
            .css_classes(["caption", "dim-label"])
            .build();
        let status = gtk::Label::builder().css_classes(["caption"]).build();
        let footer = gtk::Box::builder()
            .spacing(4)
            .halign(gtk::Align::End)
            .build();
        footer.append(&time);
        footer.append(&status);
        let bubble = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .css_classes(["bubble"])
            .build();
        bubble.append(&sender);
        let quote_name = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption-heading", "accent"])
            .build();
        let quote_text = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption"])
            .build();
        let quote = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .css_classes(["quote"])
            .build();
        quote.append(&quote_name);
        quote.append(&quote_text);
        bubble.append(&quote);
        bubble.append(&picture);
        bubble.append(&document);
        let voice_icon = gtk::Image::builder()
            .icon_name("audio-input-microphone-symbolic")
            .tooltip_text("Voice message")
            .build();
        let play = gtk::Button::builder()
            .icon_name("media-playback-start-symbolic")
            .tooltip_text("Play")
            .css_classes(["circular"])
            .build();
        let progress = gtk::Scale::builder()
            .orientation(gtk::Orientation::Horizontal)
            .hexpand(true)
            .draw_value(false)
            .build();
        let length = gtk::Label::builder()
            .css_classes(["caption", "dim-label", "numeric"])
            .build();
        let audio = gtk::Box::builder()
            .spacing(8)
            .width_request(240)
            .css_classes(["document"])
            .build();
        audio.append(&voice_icon);
        audio.append(&play);
        audio.append(&progress);
        audio.append(&length);
        let with_message = {
            let (list_item, owner) = (item.downgrade(), opener.clone());
            move |f: &dyn Fn(&Rc<Window>, &ipc::Message)| {
                let msg = list_item
                    .upgrade()
                    .and_then(|i| i.item())
                    .and_downcast::<glib::BoxedAnyObject>();
                if let (Some(msg), Some(ui)) = (msg, owner.get().and_then(std::rc::Weak::upgrade)) {
                    f(&ui, &msg.borrow::<ipc::Message>());
                }
            }
        };
        play.connect_clicked({
            let with_message = with_message.clone();
            move |_| with_message(&|ui, msg| ui.toggle_audio(msg))
        });
        // Only user drags seek; progress updates set the value directly.
        progress.connect_change_value(move |_, _, value| {
            with_message(&|ui, msg| ui.seek_audio(&msg.id, value));
            glib::Propagation::Proceed
        });
        bubble.append(&audio);
        bubble.append(&text);
        bubble.append(&footer);
        let row = gtk::Box::builder()
            .margin_start(12)
            .margin_end(12)
            .margin_top(3)
            .margin_bottom(3)
            .build();
        // A wrapping label asks for its whole text on one line; the clamp
        // caps the bubble so long messages wrap instead of filling the view.
        let clamp = adw::Clamp::builder()
            .maximum_size(BUBBLE_MAX_WIDTH)
            .tightening_threshold(BUBBLE_MAX_WIDTH)
            .hexpand(true)
            .child(&bubble)
            .build();
        row.append(&clamp);
        item.set_child(Some(&row));
    });
    let bound = rows.clone();
    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let msg = item.item().and_downcast::<glib::BoxedAnyObject>().unwrap();
        let msg = msg.borrow::<ipc::Message>();
        let clamp = item.child().and_then(|row| row.first_child()).unwrap();
        let bubble = clamp.first_child().unwrap();
        let sender = bubble.first_child().and_downcast::<gtk::Label>().unwrap();
        let quote = sender.next_sibling().unwrap();
        let quote_name = quote.first_child().and_downcast::<gtk::Label>().unwrap();
        let quote_text = quote_name
            .next_sibling()
            .and_downcast::<gtk::Label>()
            .unwrap();
        let picture = quote.next_sibling().and_downcast::<gtk::Picture>().unwrap();
        let document = picture.next_sibling().unwrap();
        let doc_labels = document.last_child().unwrap();
        let doc_name = doc_labels
            .first_child()
            .and_downcast::<gtk::Label>()
            .unwrap();
        let doc_size = doc_name
            .next_sibling()
            .and_downcast::<gtk::Label>()
            .unwrap();
        let audio = document.next_sibling().and_downcast::<gtk::Box>().unwrap();
        let text = audio.next_sibling().and_downcast::<gtk::Label>().unwrap();
        let footer = text.next_sibling().unwrap();
        let time = footer.first_child().and_downcast::<gtk::Label>().unwrap();
        let status = time.next_sibling().and_downcast::<gtk::Label>().unwrap();

        let in_group = msg.chat_jid.ends_with("@g.us");
        sender.set_visible(in_group && !msg.from_me);
        if msg.sender_name.is_empty() {
            sender.set_label(&user_label(&msg.sender_jid));
        } else {
            sender.set_label(&msg.sender_name);
        }
        quote.set_visible(msg.quote.is_some());
        if let Some(q) = &msg.quote {
            quote_name.set_label(&if q.from_me {
                "You".to_owned()
            } else if q.sender_name.is_empty() {
                user_label(&q.sender_jid)
            } else {
                q.sender_name.clone()
            });
            quote_text.set_label(&q.text);
        }
        text.set_label(&msg.text);
        text.set_visible(!msg.text.is_empty());
        time.set_label(&time_of_day(msg.timestamp_ms));
        show_status(&status, msg.status());
        bound.borrow_mut().insert(
            msg.id.clone(),
            RowWidgets {
                status: status.downgrade(),
                picture: picture.downgrade(),
                audio: audio.downgrade(),
            },
        );
        let kind = msg.media.as_ref().map(|m| m.kind());
        let is_document = kind == Some(ipc::MediaKind::Document);
        let is_audio = kind == Some(ipc::MediaKind::Audio);
        let is_picture = matches!(kind, Some(ipc::MediaKind::Image | ipc::MediaKind::Sticker));
        picture.set_visible(is_picture);
        document.set_visible(is_document);
        audio.set_visible(is_audio);
        if let Some(media) = msg.media.as_ref().filter(|_| is_audio) {
            audio.first_child().unwrap().set_visible(media.voice_note);
            // The playing message keeps its progress when scrolled back to.
            let playing = owner.get().and_then(std::rc::Weak::upgrade).and_then(|ui| {
                ui.player
                    .borrow()
                    .as_ref()
                    .filter(|p| p.msg_id == msg.id)
                    .map(|p| p.media.clone())
            });
            show_audio(&audio, media.duration_seconds, playing.as_ref());
        }
        if let Some(media) = msg.media.as_ref().filter(|_| is_document) {
            doc_name.set_label(&media.file_name);
            doc_size.set_label(&glib::format_size(media.size));
        }
        picture.set_paintable(None::<&gdk::Paintable>);
        let sticker = msg
            .media
            .as_ref()
            .is_some_and(|m| m.kind() == ipc::MediaKind::Sticker);
        // Stickers are transparent: no bubble, and no placeholder behind them.
        if sticker {
            bubble.add_css_class("sticker");
            picture.remove_css_class("media");
        } else {
            bubble.remove_css_class("sticker");
            picture.add_css_class("media");
        }
        if let Some(media) = msg.media.as_ref().filter(|_| is_picture) {
            // Reserve the final size now so the row does not grow later.
            let (width, height) = media_size(media);
            picture.set_size_request(width, height);
            if media.path.is_empty() {
                if let Some(ui) = owner.get().and_then(std::rc::Weak::upgrade) {
                    ui.request_media(&msg);
                }
            } else {
                show_media(&picture, &media.path, width, height, &msg.id, &bound);
            }
        }
        clamp.set_halign(if msg.from_me {
            gtk::Align::End
        } else {
            gtk::Align::Start
        });
        if msg.from_me {
            bubble.add_css_class("mine");
        } else {
            bubble.remove_css_class("mine");
        }
    });
    factory.connect_unbind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        if let Some(msg) = item.item().and_downcast::<glib::BoxedAnyObject>() {
            rows.borrow_mut().remove(&msg.borrow::<ipc::Message>().id);
        }
    });
    factory
}

/// Largest an image or sticker is shown in a conversation, in pixels.
pub(super) const IMAGE_MAX: i32 = 280;
pub(super) const STICKER_MAX: i32 = 160;

/// On-screen size of media, scaled down to fit and keeping its proportions.
pub(super) fn media_size(media: &ipc::Media) -> (i32, i32) {
    let max = if media.kind() == ipc::MediaKind::Sticker {
        STICKER_MAX
    } else {
        IMAGE_MAX
    };
    let (w, h) = (media.width as f64, media.height as f64);
    if w <= 0.0 || h <= 0.0 {
        return (max, max * 3 / 4);
    }
    let scale = (max as f64 / w).min(max as f64 / h).min(1.0);
    (
        (w * scale).round().max(1.0) as i32,
        (h * scale).round().max(1.0) as i32,
    )
}

thread_local! {
    /// Recently shown images, so scrolling back does not decode them again.
    static TEXTURES: RefCell<std::collections::VecDeque<(String, gdk::Texture)>> = RefCell::default();
}
pub(super) const TEXTURE_CACHE: usize = 32;

/// Decodes an image file already scaled to fit width x height logical
/// pixels: a full 1600 px photo would take about 8 MB of memory, a 280 px
/// one about 0.3 MB. Decoding runs in glycin's sandboxed loaders, off the
/// main loop. Recently decoded files come from a small cache.
pub(super) fn load_texture(
    path: &str,
    width: i32,
    height: i32,
    scale: i32,
    done: impl FnOnce(gdk::Texture) + 'static,
) {
    let cached = TEXTURES.with(|c| {
        c.borrow()
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, t)| t.clone())
    });
    if let Some(texture) = cached {
        return done(texture);
    }
    let path = path.to_owned();
    glib::spawn_future_local(async move {
        let file = gio::File::for_path(&path);
        let decoded = match file.read_future(glib::Priority::DEFAULT).await {
            Ok(stream) => {
                gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale_future(
                    &stream,
                    width * scale,
                    height * scale,
                    true,
                )
                .await
            }
            Err(e) => Err(e),
        };
        let pixbuf = match decoded {
            Ok(pixbuf) => pixbuf,
            Err(e) => return eprintln!("wassup: couldn't load {path}: {e}"),
        };
        let format = if pixbuf.has_alpha() {
            gdk::MemoryFormat::R8g8b8a8
        } else {
            gdk::MemoryFormat::R8g8b8
        };
        let texture: gdk::Texture = gdk::MemoryTexture::new(
            pixbuf.width(),
            pixbuf.height(),
            format,
            &pixbuf.read_pixel_bytes(),
            pixbuf.rowstride() as usize,
        )
        .upcast();
        TEXTURES.with(|c| {
            let mut c = c.borrow_mut();
            c.push_front((path, texture.clone()));
            c.truncate(TEXTURE_CACHE);
        });
        done(texture);
    });
}

/// Shows a cached file in a message row's picture.
pub(super) fn show_media(
    picture: &gtk::Picture,
    path: &str,
    width: i32,
    height: i32,
    msg_id: &str,
    rows: &BoundRows,
) {
    let scale = picture.scale_factor().max(1);
    let (picture, msg_id, rows) = (picture.downgrade(), msg_id.to_owned(), rows.clone());
    load_texture(path, width, height, scale, move |texture| {
        // The row may show another message by now.
        let still_bound = rows.borrow().get(&msg_id).and_then(|r| r.picture.upgrade());
        if let (Some(picture), Some(bound)) = (picture.upgrade(), still_bound)
            && picture == bound
        {
            picture.set_paintable(Some(&texture));
        }
    });
}

/// What an avatar shows until, or instead of, a picture: initials of a
/// named contact, a group icon, or a person icon. A phone number has no
/// meaningful initials.
pub(super) fn avatar_placeholder(avatar: &adw::Avatar, chat: &ipc::Chat) {
    avatar.set_custom_image(None::<&gdk::Paintable>);
    avatar.set_text(Some(&chat.name));
    avatar.set_show_initials(!chat.is_group && !chat.name.is_empty());
    avatar.set_icon_name(Some(if chat.is_group {
        "system-users-symbolic"
    } else {
        "avatar-default-symbolic"
    }));
}

/// Puts a chat's cached picture in an avatar, unless by the time it is
/// decoded the avatar shows another chat (`still_for` says whether it does).
pub(super) fn show_avatar(
    avatar: &adw::Avatar,
    path: &str,
    still_for: impl Fn() -> bool + 'static,
) {
    let size = avatar.size();
    let scale = avatar.scale_factor().max(1);
    let avatar = avatar.downgrade();
    load_texture(path, size, size, scale, move |texture| {
        if let Some(avatar) = avatar.upgrade()
            && still_for()
        {
            avatar.set_custom_image(Some(&texture));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_fits_its_box_keeping_proportions() {
        let photo = |width, height| ipc::Media {
            kind: ipc::MediaKind::Image as i32,
            width,
            height,
            ..Default::default()
        };
        assert_eq!(media_size(&photo(1600, 1200)), (280, 210));
        assert_eq!(media_size(&photo(900, 1600)), (158, 280));
        assert_eq!(
            media_size(&photo(100, 50)),
            (100, 50),
            "small images are not enlarged"
        );
        assert_eq!(
            media_size(&photo(0, 0)),
            (280, 210),
            "unknown size gets a default box"
        );
        let sticker = ipc::Media {
            kind: ipc::MediaKind::Sticker as i32,
            width: 512,
            height: 512,
            ..Default::default()
        };
        assert_eq!(media_size(&sticker), (160, 160));
    }
}
