//! Replying to messages and reacting to them.

use super::*;
use gettextrs::gettext;

/// The reactions offered in a message's menu, as in WhatsApp.
const QUICK_REACTIONS: [&str; 6] = ["👍", "❤️", "😂", "😮", "😢", "🙏"];

/// The bar above the composer while writing a reply.
pub(super) struct ReplyBar {
    pub(super) revealer: gtk::Revealer,
    name: gtk::Label,
    text: gtk::Label,
    close: gtk::Button,
    /// Id of the message being replied to.
    pub(super) target: RefCell<Option<String>>,
}

impl ReplyBar {
    pub(super) fn new() -> Self {
        let name = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption-heading", "accent"])
            .build();
        let text = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption"])
            .build();
        let labels = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .css_classes(["quote"])
            .build();
        labels.append(&name);
        labels.append(&text);
        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text(gettext("Cancel Reply"))
            .valign(gtk::Align::Center)
            .css_classes(["circular", "flat"])
            .build();
        let bar = gtk::Box::builder()
            .spacing(6)
            .margin_top(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        bar.append(&labels);
        bar.append(&close);
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .child(&bar)
            .build();
        Self {
            revealer,
            name,
            text,
            close,
            target: RefCell::default(),
        }
    }
}

impl Window {
    pub(super) fn connect_reply_bar(self: &Rc<Self>) {
        self.reply.close.connect_clicked(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_| {
                ui.cancel_reply();
                ui.composer.grab_focus();
            }
        ));
    }

    /// Shows the message being replied to above the composer.
    pub(super) fn start_reply(&self, msg: &ipc::Message) {
        self.reply.target.replace(Some(msg.id.clone()));
        self.reply.name.set_label(&sender_label(msg));
        self.reply.text.set_label(&summary(msg));
        self.reply.revealer.set_reveal_child(true);
        self.composer.grab_focus();
    }

    /// Drops the reply being written, if any; says whether there was one.
    pub(super) fn cancel_reply(&self) -> bool {
        self.reply.revealer.set_reveal_child(false);
        self.reply.target.take().is_some()
    }

    /// The menu of a message: quick reactions and Reply. `at` is where the
    /// click was, in the bubble's coordinates.
    pub(super) fn message_menu(
        self: &Rc<Self>,
        bubble: &gtk::Widget,
        msg: &ipc::Message,
        at: (f64, f64),
    ) {
        let reactions = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let popover = gtk::Popover::builder().has_arrow(true).build();
        for emoji in QUICK_REACTIONS {
            let mine = msg.reactions.iter().any(|r| r.mine && r.emoji == emoji);
            let button = gtk::Button::builder()
                .label(emoji)
                .css_classes(if mine {
                    ["flat", "circular", "reaction-mine"]
                } else {
                    ["flat", "circular", "reaction"]
                })
                .build();
            // TRANSLATORS: {} is an emoji.
            button.set_tooltip_text(Some(&gettext("React with {}").replace("{}", emoji)));
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = ui)]
                self,
                #[weak]
                popover,
                #[strong]
                msg,
                move |_| {
                    popover.popdown();
                    // Choosing one's own reaction again takes it back.
                    ui.react(&msg, if mine { "" } else { emoji });
                }
            ));
            reactions.append(&button);
        }
        let reply = gtk::Button::builder()
            .child(
                &adw::ButtonContent::builder()
                    .icon_name("mail-reply-sender-symbolic")
                    .label(gettext("Reply"))
                    .halign(gtk::Align::Start)
                    .build(),
            )
            .css_classes(["flat"])
            .build();
        reply.connect_clicked(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            #[weak]
            popover,
            #[strong]
            msg,
            move |_| {
                popover.popdown();
                ui.start_reply(&msg);
            }
        ));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        content.append(&reactions);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content.append(&reply);
        popover.set_child(Some(&content));
        popover.set_parent(bubble);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(at.0 as i32, at.1 as i32, 1, 1)));
        // A popover lives on its parent until removed.
        popover.connect_closed(|popover| {
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
    }

    fn react(&self, msg: &ipc::Message, emoji: &str) {
        let Some((account_id, chat_jid)) = self.open_ids() else {
            return;
        };
        self.send(request::Kind::React(ipc::React {
            account_id,
            chat_jid,
            message_id: msg.id.clone(),
            emoji: emoji.to_owned(),
        }));
    }

    /// Updates a message's reactions in the open chat.
    pub(super) fn reactions_changed(&self, changed: ipc::ReactionsChanged) {
        let open = self.is_current(&changed.account_id)
            && self.current_chat.borrow().as_deref() == Some(changed.chat_jid.as_str());
        if !open {
            return;
        }
        for pos in 0..self.messages.n_items() {
            let Some(item) = self
                .messages
                .item(pos)
                .and_downcast::<glib::BoxedAnyObject>()
            else {
                continue;
            };
            let mut msg = item.borrow_mut::<ipc::Message>();
            if msg.id == changed.message_id {
                msg.reactions = changed.reactions.clone();
                break;
            }
        }
        let label = self
            .rows
            .borrow()
            .get(&changed.message_id)
            .and_then(|r| r.reactions.upgrade());
        if let Some(label) = label {
            show_reactions(&label, &changed.reactions);
        }
    }
}

/// A message's reactions under its bubble: "❤️ 2 😂".
pub(super) fn show_reactions(label: &gtk::Label, reactions: &[ipc::Reaction]) {
    let text = reactions
        .iter()
        .map(|r| {
            if r.count > 1 {
                format!("{} {}", r.emoji, r.count)
            } else {
                r.emoji.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("  ");
    label.set_label(&text);
    label.set_visible(!reactions.is_empty());
    if reactions.iter().any(|r| r.mine) {
        label.add_css_class("mine");
    } else {
        label.remove_css_class("mine");
    }
}

/// Who sent a message, as a quote names them.
pub(super) fn sender_label(msg: &ipc::Message) -> String {
    if msg.from_me {
        gettext("You")
    } else if msg.sender_name.is_empty() {
        user_label(&msg.sender_jid)
    } else {
        msg.sender_name.clone()
    }
}

/// A message in a few words: its text, or what kind of media it is.
pub(super) fn summary(msg: &ipc::Message) -> String {
    match msg.media.as_ref() {
        _ if !msg.text.is_empty() => msg.text.clone(),
        Some(m) if m.kind() == ipc::MediaKind::Image => gettext("Photo"),
        Some(m) if m.kind() == ipc::MediaKind::Video => gettext("Video"),
        Some(m) if m.kind() == ipc::MediaKind::Sticker => gettext("Sticker"),
        Some(m) if m.kind() == ipc::MediaKind::Document => gettext("Document"),
        Some(m) if m.voice_note => gettext("Voice message"),
        Some(m) if m.kind() == ipc::MediaKind::Audio => gettext("Audio"),
        _ => String::new(),
    }
}
