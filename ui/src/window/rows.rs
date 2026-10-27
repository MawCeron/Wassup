//! The list rows of chats and messages, and the pictures in them.

use super::*;
use gettextrs::{gettext, ngettext};

pub(super) fn chat_row_factory(owner: Owner) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    // Two lines beside a tall avatar, as in WhatsApp Web: the name and the
    // time, then the last message and the unread count.
    factory.connect_setup(|_, item| {
        let avatar = adw::Avatar::builder().size(48).show_initials(true).build();
        let title = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["heading"])
            .build();
        let time = gtk::Label::builder()
            .css_classes(["dim-label", "caption"])
            .build();
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        top.append(&title);
        top.append(&time);
        let status = gtk::Image::new();
        let preview = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .single_line_mode(true)
            .css_classes(["dim-label"])
            .build();
        let unread = gtk::Label::builder()
            .valign(gtk::Align::Center)
            .css_classes(["unread-pill", "numeric"])
            .build();
        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        bottom.append(&status);
        bottom.append(&preview);
        bottom.append(&unread);
        let lines = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        lines.append(&top);
        lines.append(&bottom);
        let row = gtk::Box::builder()
            .spacing(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        row.append(&avatar);
        row.append(&lines);
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
        let lines = avatar.next_sibling().unwrap();
        let top = lines.first_child().unwrap();
        let title = top.first_child().and_downcast::<gtk::Label>().unwrap();
        let time = title.next_sibling().and_downcast::<gtk::Label>().unwrap();
        let bottom = top.next_sibling().unwrap();
        let status = bottom.first_child().and_downcast::<gtk::Image>().unwrap();
        let preview = status.next_sibling().and_downcast::<gtk::Label>().unwrap();
        let unread = preview.next_sibling().and_downcast::<gtk::Label>().unwrap();
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
        title.set_label(&chat_title(&chat));
        time.set_label(&format_time(chat.last_message_at_ms));
        // An unread chat's time takes the accent, as its count does.
        if chat.unread_count > 0 {
            time.set_css_classes(&["caption", "accent"]);
        } else {
            time.set_css_classes(&["caption", "dim-label"]);
        }
        // Own messages show their ticks; group messages who wrote them.
        if chat.last_from_me {
            show_status(&status, chat.last_status());
        } else {
            status.set_visible(false);
        }
        let text = if chat.last_sender_name.is_empty() {
            chat.last_message.clone()
        } else {
            format!("{}: {}", chat.last_sender_name, chat.last_message)
        };
        preview.set_label(&text);
        unread.set_label(&chat.unread_count.to_string());
        unread.set_visible(chat.unread_count > 0);
        // Read as one line, so the unread count is not a bare number.
        let mut label = chat_title(&chat);
        if chat.unread_count > 0 {
            label += ", ";
            // TRANSLATORS: {} is how many.
            label += &ngettext("{} unread message", "{} unread messages", chat.unread_count)
                .replace("{}", &chat.unread_count.to_string());
        }
        if !text.is_empty() {
            label += ", ";
            label += &text;
        }
        item.set_accessible_label(&label);
    });
    factory
}

pub(super) fn show_status(icon: &gtk::Image, status: ipc::MessageStatus) {
    let (name, tooltip, read) = match status {
        ipc::MessageStatus::Sent => (Some("wassup-tick-symbolic"), "Sent", false),
        ipc::MessageStatus::Delivered => (Some("wassup-double-tick-symbolic"), "Delivered", false),
        ipc::MessageStatus::Read => (Some("wassup-double-tick-symbolic"), "Read", true),
        ipc::MessageStatus::Unspecified => (None, "", false),
    };
    icon.set_icon_name(name);
    icon.set_visible(name.is_some()); // Received messages have none.
    icon.set_tooltip_text(Some(tooltip));
    icon.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    icon.set_css_classes(if read { &["accent"] } else { &["dim-label"] });
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
            .css_classes(["media"])
            // Clips the image to the rounded corners.
            .overflow(gtk::Overflow::Hidden)
            .build();
        // Videos show their thumbnail with a play badge and their length.
        let play_badge = gtk::Image::builder()
            .icon_name("wassup-play-symbolic")
            .pixel_size(24)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["osd", "video-badge"])
            .build();
        let video_length = gtk::Label::builder()
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .margin_end(6)
            .margin_bottom(6)
            .css_classes(["osd", "caption", "numeric", "video-length"])
            .build();
        // A picture grows to keep its image's proportions; the two clamps cap
        // it at its box, so a wide or tall sticker stays in its square.
        let clamp_width = adw::Clamp::builder().child(&picture).build();
        let clamp_height = adw::Clamp::builder()
            .orientation(gtk::Orientation::Vertical)
            .child(&clamp_width)
            .build();
        let media_overlay = gtk::Overlay::builder().child(&clamp_height).build();
        media_overlay.add_overlay(&play_badge);
        media_overlay.add_overlay(&video_length);
        // Buttons, so photos and documents also open from the keyboard.
        let media_button = gtk::Button::builder()
            .child(&media_overlay)
            .halign(gtk::Align::Start)
            .css_classes(["flat", "media-button"])
            .cursor(&gdk::Cursor::from_name("pointer", None).unwrap())
            .build();
        // Clicking opens the file, or for a sticker offers to favorite it.
        let on_click = {
            let (list_item, owner) = (item.downgrade(), opener.clone());
            move |anchor: &gtk::Widget| {
                let msg = list_item
                    .upgrade()
                    .and_then(|i| i.item())
                    .and_downcast::<glib::BoxedAnyObject>();
                if let (Some(msg), Some(ui)) = (msg, owner.get().and_then(std::rc::Weak::upgrade)) {
                    ui.open_media(&msg.borrow::<ipc::Message>(), anchor);
                }
            }
        };
        media_button.connect_clicked({
            let on_click = on_click.clone();
            move |button| on_click(button.upcast_ref())
        });
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
        let document_box = gtk::Box::builder()
            .spacing(10)
            .css_classes(["document"])
            .build();
        document_box.append(
            &gtk::Image::builder()
                .icon_name("wassup-document-symbolic")
                .pixel_size(32)
                .build(),
        );
        document_box.append(&doc_labels);
        let document = gtk::Button::builder()
            .child(&document_box)
            .halign(gtk::Align::Start)
            .css_classes(["flat", "media-button"])
            .cursor(&gdk::Cursor::from_name("pointer", None).unwrap())
            .build();
        document.connect_clicked(move |button| on_click(button.upcast_ref()));
        let text = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .build();
        let time = gtk::Label::builder()
            .css_classes(["caption", "dim-label"])
            .build();
        let status = gtk::Image::new();
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
        let quote_labels = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();
        quote_labels.append(&quote_name);
        quote_labels.append(&quote_text);
        // A quoted photo, video or document shows its thumbnail on the right.
        let quote_thumb = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .width_request(QUOTE_THUMB)
            .height_request(QUOTE_THUMB)
            .valign(gtk::Align::Center)
            .overflow(gtk::Overflow::Hidden)
            .css_classes(["link-thumb"])
            .build();
        let quote = gtk::Box::builder()
            .spacing(8)
            .css_classes(["quote"])
            .build();
        quote.append(&quote_labels);
        quote.append(&quote_thumb);
        bubble.append(&quote);
        // A link's preview, above the text as in WhatsApp; opens the link.
        let link_thumb = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .width_request(LINK_THUMB)
            .height_request(LINK_THUMB)
            .valign(gtk::Align::Start)
            .overflow(gtk::Overflow::Hidden)
            .css_classes(["link-thumb"])
            .build();
        let link_title = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["heading"])
            .build();
        let link_description = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption"])
            .build();
        let link_site = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption", "dim-label"])
            .build();
        let link_labels = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();
        link_labels.append(&link_title);
        link_labels.append(&link_description);
        link_labels.append(&link_site);
        let link_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        link_box.append(&link_thumb);
        link_box.append(&link_labels);
        let link_card = gtk::Button::builder()
            .child(&link_box)
            .css_classes(["flat", "link-card"])
            .build();
        link_card.connect_clicked({
            let (list_item, owner) = (item.downgrade(), opener.clone());
            move |_| {
                let msg = list_item
                    .upgrade()
                    .and_then(|i| i.item())
                    .and_downcast::<glib::BoxedAnyObject>();
                if let (Some(msg), Some(ui)) = (msg, owner.get().and_then(std::rc::Weak::upgrade))
                    && let Some(link) = &msg.borrow::<ipc::Message>().link
                {
                    ui.open_link(&link.url);
                }
            }
        });
        bubble.append(&link_card);
        bubble.append(&media_button);
        bubble.append(&document);
        let voice_icon = gtk::Image::builder()
            .icon_name("wassup-voice-note-symbolic")
            .tooltip_text(gettext("Voice Message"))
            .build();
        let play = gtk::Button::builder()
            .icon_name("wassup-play-symbolic")
            .tooltip_text(gettext("Play"))
            .css_classes(["circular"])
            .build();
        let progress = gtk::Scale::builder()
            .orientation(gtk::Orientation::Horizontal)
            .hexpand(true)
            .draw_value(false)
            .build();
        progress.update_property(&[gtk::accessible::Property::Label(&gettext(
            "Playback position",
        ))]);
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
        // Reactions sit on the bubble's bottom edge, outside it, as in
        // WhatsApp; the spacer under the bubble makes room for them.
        let reactions = gtk::Label::builder()
            .valign(gtk::Align::End)
            .margin_start(10)
            .margin_end(10)
            .css_classes(["reactions"])
            .visible(false)
            .build();
        let bubble_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bubble_column.append(&bubble);
        bubble_column.append(
            &gtk::Box::builder()
                .height_request(12)
                .visible(false)
                .build(),
        );
        let bubble_area = gtk::Overlay::builder().child(&bubble_column).build();
        bubble_area.add_overlay(&reactions);
        // Right click or a long press opens the message's menu.
        let open_menu = {
            let (list_item, owner, bubble) = (item.downgrade(), opener.clone(), bubble.downgrade());
            move |x: f64, y: f64| {
                let msg = list_item
                    .upgrade()
                    .and_then(|i| i.item())
                    .and_downcast::<glib::BoxedAnyObject>();
                if let (Some(msg), Some(ui), Some(bubble)) = (
                    msg,
                    owner.get().and_then(std::rc::Weak::upgrade),
                    bubble.upgrade(),
                ) {
                    let msg = msg.borrow::<ipc::Message>().clone();
                    ui.message_menu(bubble.upcast_ref(), &msg, (x, y));
                }
            }
        };
        let right_click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_SECONDARY)
            .build();
        right_click.connect_pressed({
            let open_menu = open_menu.clone();
            move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                open_menu(x, y);
            }
        });
        bubble.add_controller(right_click);
        let long_press = gtk::GestureLongPress::new();
        long_press.connect_pressed(move |_, x, y| open_menu(x, y));
        bubble.add_controller(long_press);
        // The day, above the first message of each day.
        let day = gtk::Label::builder()
            .halign(gtk::Align::Center)
            .margin_top(8)
            .margin_bottom(6)
            .css_classes(["day-separator"])
            .build();
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .margin_start(12)
            .margin_end(12)
            .margin_top(3)
            .margin_bottom(3)
            .build();
        row.append(&day);
        // A wrapping label asks for its whole text on one line; the clamp
        // caps the bubble so long messages wrap instead of filling the view.
        let clamp = adw::Clamp::builder()
            .maximum_size(BUBBLE_MAX_WIDTH)
            .tightening_threshold(BUBBLE_MAX_WIDTH)
            .hexpand(true)
            .child(&bubble_area)
            .build();
        row.append(&clamp);
        item.set_child(Some(&row));
    });
    let bound = rows.clone();
    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let msg = item.item().and_downcast::<glib::BoxedAnyObject>().unwrap();
        let msg = msg.borrow::<ipc::Message>();
        let day = item
            .child()
            .and_then(|row| row.first_child())
            .and_downcast::<gtk::Label>()
            .unwrap();
        let clamp = day.next_sibling().unwrap();
        let bubble_area = clamp.first_child().and_downcast::<gtk::Overlay>().unwrap();
        let bubble = bubble_area.child().and_then(|c| c.first_child()).unwrap();
        let reactions = bubble_area
            .last_child()
            .and_downcast::<gtk::Label>()
            .unwrap();
        let sender = bubble.first_child().and_downcast::<gtk::Label>().unwrap();
        let quote = sender.next_sibling().unwrap();
        let quote_name = quote
            .first_child()
            .and_then(|b| b.first_child())
            .and_downcast::<gtk::Label>()
            .unwrap();
        let quote_thumb = quote.last_child().and_downcast::<gtk::Picture>().unwrap();
        let quote_text = quote_name
            .next_sibling()
            .and_downcast::<gtk::Label>()
            .unwrap();
        let link_card = quote.next_sibling().unwrap();
        let link_box = link_card.first_child().unwrap();
        let link_thumb = link_box
            .first_child()
            .and_downcast::<gtk::Picture>()
            .unwrap();
        let link_title = link_thumb
            .next_sibling()
            .and_then(|b| b.first_child())
            .and_downcast::<gtk::Label>()
            .unwrap();
        let link_description = link_title
            .next_sibling()
            .and_downcast::<gtk::Label>()
            .unwrap();
        let link_site = link_description
            .next_sibling()
            .and_downcast::<gtk::Label>()
            .unwrap();
        let media_button = link_card
            .next_sibling()
            .and_downcast::<gtk::Button>()
            .unwrap();
        let clamps = media_button
            .child()
            .and_downcast::<gtk::Overlay>()
            .and_then(|o| o.child()) // The height clamp, then the width clamp.
            .unwrap();
        let picture = clamps
            .first_child()
            .and_then(|c| c.first_child())
            .and_downcast::<gtk::Picture>()
            .unwrap();
        let play_badge = clamps.next_sibling().unwrap();
        let video_length = play_badge
            .next_sibling()
            .and_downcast::<gtk::Label>()
            .unwrap();
        let document = media_button.next_sibling().unwrap();
        let doc_labels = document.first_child().and_then(|b| b.last_child()).unwrap();
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
        let status = time.next_sibling().and_downcast::<gtk::Image>().unwrap();
        show_reactions(&reactions, &msg.reactions);
        // A separator when this message starts a day: the first loaded one,
        // or one on another day than the message before it.
        let previous = owner.get().and_then(std::rc::Weak::upgrade).and_then(|ui| {
            item.position()
                .checked_sub(1)
                .and_then(|p| ui.messages.item(p))
                .and_downcast::<glib::BoxedAnyObject>()
                .map(|m| m.borrow::<ipc::Message>().timestamp_ms)
        });
        let starts_day = previous.is_none_or(|p| !same_day(p, msg.timestamp_ms));
        day.set_visible(starts_day);
        if starts_day {
            day.set_label(&day_label(msg.timestamp_ms));
        }

        let in_group = msg.chat_jid.ends_with("@g.us");
        sender.set_visible(in_group && !msg.from_me);
        if msg.sender_name.is_empty() {
            sender.set_label(&user_label(&msg.sender_jid));
        } else {
            sender.set_label(&msg.sender_name);
        }
        quote.set_visible(msg.quote.is_some());
        link_card.set_visible(msg.link.is_some());
        if let Some(link) = &msg.link {
            link_title.set_label(&link.title);
            link_title.set_visible(!link.title.is_empty());
            link_description.set_label(&link.description);
            link_description.set_visible(!link.description.is_empty());
            link_site.set_label(&link_host(&link.url));
            link_card.update_property(&[gtk::accessible::Property::Label(&link.title)]);
            link_thumb.set_paintable(None::<&gdk::Paintable>);
            link_thumb.set_visible(!link.thumbnail_path.is_empty());
            if !link.thumbnail_path.is_empty() {
                let weak = link_thumb.downgrade();
                let scale = link_thumb.scale_factor().max(1);
                load_texture(
                    &link.thumbnail_path,
                    LINK_THUMB,
                    LINK_THUMB,
                    scale,
                    move |t| {
                        if let Some(thumb) = weak.upgrade() {
                            thumb.set_paintable(Some(&t));
                        }
                    },
                );
            }
        }
        if let Some(q) = &msg.quote {
            quote_name.set_label(&if q.from_me {
                gettext("You")
            } else if q.sender_name.is_empty() {
                user_label(&q.sender_jid)
            } else {
                q.sender_name.clone()
            });
            quote_text.set_label(&q.text);
            quote_thumb.set_paintable(None::<&gdk::Paintable>);
            quote_thumb.set_visible(!q.thumbnail_path.is_empty());
            if !q.thumbnail_path.is_empty() {
                let weak = quote_thumb.downgrade();
                let scale = quote_thumb.scale_factor().max(1);
                load_texture(
                    &q.thumbnail_path,
                    QUOTE_THUMB,
                    QUOTE_THUMB,
                    scale,
                    move |t| {
                        if let Some(thumb) = weak.upgrade() {
                            thumb.set_paintable(Some(&t));
                        }
                    },
                );
            }
        }
        text.set_markup(&linkify(&msg.text));
        text.set_visible(!msg.text.is_empty());
        time.set_label(&time_of_day(msg.timestamp_ms));
        show_status(&status, msg.status());
        bound.borrow_mut().insert(
            msg.id.clone(),
            RowWidgets {
                status: status.downgrade(),
                picture: picture.downgrade(),
                audio: audio.downgrade(),
                reactions: reactions.downgrade(),
            },
        );
        let kind = msg.media.as_ref().map(|m| m.kind());
        let is_document = kind == Some(ipc::MediaKind::Document);
        let is_audio = kind == Some(ipc::MediaKind::Audio);
        let is_picture = matches!(kind, Some(ipc::MediaKind::Image | ipc::MediaKind::Sticker));
        let is_video = kind == Some(ipc::MediaKind::Video);
        media_button.set_visible(is_picture || is_video);
        let is_gif = msg.media.as_ref().is_some_and(|m| m.gif);
        play_badge.set_visible(is_video && !is_gif);
        video_length.set_visible(is_video);
        let media_label = match kind {
            Some(ipc::MediaKind::Sticker) => gettext("Sticker"),
            Some(ipc::MediaKind::Video) => gettext("Video"),
            _ => gettext("Photo"),
        };
        media_button.update_property(&[gtk::accessible::Property::Label(&media_label)]);
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
            picture.add_css_class("sticker-image");
        } else {
            bubble.remove_css_class("sticker");
            picture.remove_css_class("sticker-image");
            picture.add_css_class("media");
        }
        if let Some(media) = msg.media.as_ref().filter(|_| is_video) {
            // Only the thumbnail: a video downloads when played, a GIF at once.
            let (width, height) = media_size(media);
            fit_picture(&picture, width, height, false);
            // A GIF says what it is instead of its length.
            video_length.set_label(&if media.gif {
                "GIF".to_owned()
            } else {
                clock(media.duration_seconds as f64)
            });
            if !media.thumbnail_path.is_empty() {
                show_media(
                    &picture,
                    &media.thumbnail_path,
                    width,
                    height,
                    &msg.id,
                    &bound,
                );
            }
            if media.gif {
                // A GIF plays by itself, so it downloads without being asked.
                if media.path.is_empty() {
                    if let Some(ui) = owner.get().and_then(std::rc::Weak::upgrade) {
                        ui.request_media(&msg);
                    }
                } else {
                    gif::play(&picture, &media.path, still_shows(&bound, &msg.id));
                }
            }
        }
        if let Some(media) = msg.media.as_ref().filter(|_| is_picture) {
            // Reserve the final size now so the row does not grow later.
            let (width, height) = media_size(media);
            fit_picture(&picture, width, height, sticker);
            if media.path.is_empty() {
                if let Some(ui) = owner.get().and_then(std::rc::Weak::upgrade) {
                    ui.request_media(&msg);
                }
            } else {
                show_media(&picture, &media.path, width, height, &msg.id, &bound);
                if sticker {
                    animation::play(&picture, &media.path, still_shows(&bound, &msg.id));
                }
            }
        }
        clamp.set_halign(if msg.from_me {
            gtk::Align::End
        } else {
            gtk::Align::Start
        });
        reactions.set_halign(if msg.from_me {
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

/// Sizes a message's picture to width x height, at least and, through its
/// two clamps, at most, so its rounded corners are the image's. Stickers
/// sit centered in a fixed square around it, as in WhatsApp.
fn fit_picture(picture: &gtk::Picture, width: i32, height: i32, sticker: bool) {
    picture.set_size_request(width, height);
    if let Some(outer) = picture.parent().and_then(|c| c.parent()) {
        let side = if sticker { STICKER_MAX } else { -1 };
        outer.set_size_request(side, side);
    }
    let clamps = [picture.parent(), picture.parent().and_then(|c| c.parent())];
    for (clamp, max) in clamps.into_iter().zip([width, height]) {
        if let Some(clamp) = clamp.and_downcast::<adw::Clamp>() {
            clamp.set_maximum_size(max);
            clamp.set_tightening_threshold(max);
        }
    }
}

/// Whether a picture still shows the message `id`, for a playback that ends
/// with its row.
pub(super) fn still_shows(rows: &BoundRows, id: &str) -> impl Fn(&gtk::Picture) -> bool + 'static {
    let (rows, id) = (rows.clone(), id.to_owned());
    move |picture| {
        rows.borrow()
            .get(&id)
            .and_then(|r| r.picture.upgrade())
            .as_ref()
            == Some(picture)
    }
}

/// Whether two moments fall on the same local day.
fn same_day(a_ms: i64, b_ms: i64) -> bool {
    let day = |ms: i64| {
        glib::DateTime::from_unix_local(ms / 1000)
            .ok()
            .map(|d| d.ymd())
    };
    day(a_ms) == day(b_ms)
}

/// A day as the separator shows it: Today, Yesterday, the weekday within
/// the last week, or the date.
pub(super) fn day_label(ms: i64) -> String {
    let (Ok(at), Ok(now)) = (
        glib::DateTime::from_unix_local(ms / 1000),
        glib::DateTime::now_local(),
    ) else {
        return String::new();
    };
    let midnight = |d: &glib::DateTime| {
        let (y, m, day) = d.ymd();
        glib::DateTime::from_local(y, m, day, 0, 0, 0.0).ok()
    };
    let days_ago = match (midnight(&now), midnight(&at)) {
        (Some(today), Some(that)) => (today.difference(&that).as_seconds() + 3600) / 86_400,
        _ => i64::MAX,
    };
    let label = match days_ago {
        0 => return gettext("Today"),
        1 => return gettext("Yesterday"),
        2..=6 => at.format("%A"),
        _ => at.format("%x"),
    };
    // Weekdays are lowercase in some languages, such as Spanish.
    let label: String = label.map(Into::into).unwrap_or_default();
    let mut chars = label.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Side of a link preview's image, in pixels.
const LINK_THUMB: i32 = 64;
/// Side of a quoted photo's thumbnail, in pixels.
const QUOTE_THUMB: i32 = 40;

/// The site a link points to, as a preview shows it: "gnome.org".
pub(super) fn link_host(url: &str) -> String {
    glib::Uri::parse(url, glib::UriFlags::NONE)
        .ok()
        .and_then(|u| u.host())
        .map(|h| h.trim_start_matches("www.").to_owned())
        .unwrap_or_else(|| url.to_owned())
}

/// Message text as Pango markup, with web addresses made into links. The
/// rest is escaped: message text is never markup.
pub(super) fn linkify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        // Up to the next word that starts a web address.
        let start = rest
            .match_indices(|c: char| !c.is_whitespace())
            .map(|(i, _)| i)
            .find(|&i| {
                (i == 0 || rest[..i].ends_with(char::is_whitespace))
                    && ["https://", "http://", "www."].iter().any(|p| {
                        rest[i..]
                            .get(..p.len())
                            .is_some_and(|s| s.eq_ignore_ascii_case(p))
                    })
            });
        let Some(start) = start else {
            out += &glib::markup_escape_text(rest);
            break;
        };
        out += &glib::markup_escape_text(&rest[..start]);
        let word_end = rest[start..]
            .find(char::is_whitespace)
            .map_or(rest.len(), |e| start + e);
        // Punctuation right after an address is the sentence's, not the link's.
        let url = rest[start..word_end]
            .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '"', '\'']);
        let end = start + url.len();
        let href = if url.len() >= 4 && url[..4].eq_ignore_ascii_case("www.") {
            format!("https://{url}")
        } else {
            url.to_owned()
        };
        out += &format!(
            "<a href=\"{}\">{}</a>",
            glib::markup_escape_text(&href),
            glib::markup_escape_text(url)
        );
        rest = &rest[end..];
    }
    out
}

/// Largest an image or sticker is shown in a conversation, in pixels.
pub(super) const IMAGE_MAX: i32 = 280;
pub(super) const STICKER_MAX: i32 = 160;

/// On-screen size of media, scaled down to fit and keeping its proportions.
/// Stickers are always shown in the same square, whatever size the sender
/// announced: most do not announce one, and some announce a small one.
pub(super) fn media_size(media: &ipc::Media) -> (i32, i32) {
    if media.kind() == ipc::MediaKind::Sticker {
        return (STICKER_MAX, STICKER_MAX);
    }
    let (w, h) = (media.width as f64, media.height as f64);
    if w <= 0.0 || h <= 0.0 {
        return (IMAGE_MAX, IMAGE_MAX * 3 / 4);
    }
    let scale = (IMAGE_MAX as f64 / w).min(IMAGE_MAX as f64 / h).min(1.0);
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
        let Some(texture) = decode_texture(&path, width * scale, height * scale).await else {
            return;
        };
        TEXTURES.with(|c| {
            let mut c = c.borrow_mut();
            c.push_front((path, texture.clone()));
            c.truncate(TEXTURE_CACHE);
        });
        done(texture);
    });
}

/// Decodes an image file scaled to fit width x height device pixels,
/// without the cache: for one-off large images such as the photo viewer.
pub(super) async fn decode_texture(path: &str, width: i32, height: i32) -> Option<gdk::Texture> {
    let file = gio::File::for_path(path);
    let decoded = match file.read_future(glib::Priority::DEFAULT).await {
        Ok(stream) => {
            gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale_future(&stream, width, height, true).await
        }
        Err(e) => Err(e),
    };
    let pixbuf = match decoded {
        Ok(pixbuf) => pixbuf,
        Err(e) => {
            eprintln!("wassup: couldn't load {path}: {e}");
            return None;
        }
    };
    let format = if pixbuf.has_alpha() {
        gdk::MemoryFormat::R8g8b8a8
    } else {
        gdk::MemoryFormat::R8g8b8
    };
    Some(
        gdk::MemoryTexture::new(
            pixbuf.width(),
            pixbuf.height(),
            format,
            &pixbuf.read_pixel_bytes(),
            pixbuf.rowstride() as usize,
        )
        .upcast(),
    )
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
    fn days_are_named_relative_to_today() {
        let now = glib::DateTime::now_local().unwrap();
        let ms = |d: &glib::DateTime| d.to_unix() * 1000;
        assert_eq!(day_label(ms(&now)), "Today");
        assert_eq!(day_label(ms(&now.add_days(-1).unwrap())), "Yesterday");
        let label = day_label(ms(&now.add_days(-3).unwrap()));
        assert!(label.chars().next().unwrap().is_uppercase(), "{label}");
        assert!(same_day(ms(&now), ms(&now)));
        assert!(!same_day(ms(&now), ms(&now.add_days(-1).unwrap())));
    }

    #[test]
    fn links_are_made_clickable_and_text_escaped() {
        assert_eq!(
            linkify("see https://gnome.org/apps, ok"),
            "see <a href=\"https://gnome.org/apps\">https://gnome.org/apps</a>, ok"
        );
        assert_eq!(
            linkify("WWW.example.com."),
            "<a href=\"https://WWW.example.com\">WWW.example.com</a>."
        );
        assert_eq!(linkify("a <b> & c"), "a &lt;b&gt; &amp; c");
        assert_eq!(
            linkify("x https://a.b/?q=1&r=<2>"),
            "x <a href=\"https://a.b/?q=1&amp;r=&lt;2&gt;\">https://a.b/?q=1&amp;r=&lt;2&gt;</a>"
        );
        assert_eq!(linkify("nohttps://here"), "nohttps://here");
        assert_eq!(
            linkify("¿ves https://ñ.mx?"),
            "¿ves <a href=\"https://ñ.mx\">https://ñ.mx</a>?"
        );
    }

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
        // Stickers have one size, with the dimensions the sender announced or without them.
        for (width, height) in [(512, 512), (64, 64), (0, 0), (512, 300)] {
            let sticker = ipc::Media {
                kind: ipc::MediaKind::Sticker as i32,
                width,
                height,
                ..Default::default()
            };
            assert_eq!(media_size(&sticker), (160, 160), "{width}x{height}");
        }
    }
}
