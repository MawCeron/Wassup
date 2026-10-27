//! Searching messages in the open chat.

use super::*;
use gettextrs::gettext;

/// The search bar under the conversation header and its results.
pub(super) struct MessageSearch {
    pub(super) bar: gtk::SearchBar,
    pub(super) entry: gtk::SearchEntry,
    pub(super) button: gtk::ToggleButton,
    results: gtk::ListBox,
    /// Results page, or a message when nothing matches.
    pub(super) pages: gtk::Stack,
    /// Ids of the SearchMessages and from_id ListMessages requests in flight.
    pub(super) request: Cell<u64>,
    pub(super) jump_request: Cell<u64>,
    jump_target: RefCell<Option<String>>,
}

impl MessageSearch {
    pub(super) fn new() -> Self {
        let entry = gtk::SearchEntry::builder()
            .placeholder_text(gettext("Search messages"))
            .hexpand(true)
            .build();
        let bar = gtk::SearchBar::builder()
            .child(
                &adw::Clamp::builder()
                    .maximum_size(500)
                    .child(&entry)
                    .build(),
            )
            .build();
        bar.connect_entry(&entry);
        let button = gtk::ToggleButton::builder()
            .icon_name("edit-find-symbolic")
            .tooltip_text(gettext("Search Messages"))
            .build();
        button
            .bind_property("active", &bar, "search-mode-enabled")
            .bidirectional()
            .build();
        let results = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .valign(gtk::Align::Start)
            .build();
        let pages = gtk::Stack::new();
        pages.add_named(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(
                    &adw::Clamp::builder()
                        .maximum_size(700)
                        .child(&results)
                        .build(),
                )
                .build(),
            Some("results"),
        );
        pages.add_named(
            &adw::StatusPage::builder()
                .icon_name("edit-find-symbolic")
                .title(gettext("No Messages Found"))
                .build(),
            Some("none"),
        );
        Self {
            bar,
            entry,
            button,
            results,
            pages,
            request: Cell::new(0),
            jump_request: Cell::new(0),
            jump_target: RefCell::default(),
        }
    }
}

impl Window {
    pub(super) fn connect_message_search(self: &Rc<Self>) {
        let s = &self.search;
        s.entry.connect_search_changed(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |entry| ui.search_messages(&entry.text())
        ));
        s.bar.connect_search_mode_enabled_notify(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |bar| {
                if !bar.is_search_mode() {
                    ui.search.entry.set_text("");
                    ui.show_conversation();
                }
            }
        ));
        s.results.connect_row_activated(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_, row| ui.jump_to(&row.widget_name())
        ));
    }

    fn search_messages(&self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            self.search.request.set(0);
            return self.show_conversation();
        }
        let Some((account_id, chat_jid)) = self.open_ids() else {
            return;
        };
        let request = request::Kind::SearchMessages(ipc::SearchMessages {
            account_id,
            chat_jid,
            query: text.to_owned(),
        });
        if let Some(id) = self.send(request) {
            self.search.request.set(id);
        }
    }

    /// Lists the matches, newest first, in place of the conversation.
    pub(super) fn show_results(&self, found: Vec<ipc::Message>) {
        let s = &self.search;
        s.results.remove_all();
        for msg in &found {
            let sender = gtk::Label::builder()
                .label(if msg.from_me {
                    gettext("You")
                } else if msg.sender_name.is_empty() {
                    user_label(&msg.sender_jid)
                } else {
                    msg.sender_name.clone()
                })
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["caption-heading"])
                .build();
            let when = gtk::Label::builder()
                .label(format_time(msg.timestamp_ms))
                .css_classes(["caption", "dim-label"])
                .build();
            let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            top.append(&sender);
            top.append(&when);
            let text = gtk::Label::builder()
                .label(&msg.text)
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .lines(2)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            let column = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(4)
                .margin_top(8)
                .margin_bottom(8)
                .margin_start(12)
                .margin_end(12)
                .build();
            column.append(&top);
            column.append(&text);
            let row = gtk::ListBoxRow::builder()
                .child(&column)
                .activatable(true)
                .name(msg.id.as_str())
                .build();
            s.results.append(&row);
        }
        s.pages
            .set_visible_child_name(if found.is_empty() { "none" } else { "results" });
        self.content_stack.set_visible_child_name("search");
    }

    /// Back from the results to the conversation.
    pub(super) fn show_conversation(&self) {
        if self.content_stack.visible_child_name().as_deref() == Some("search") {
            self.content_stack.set_visible_child_name("messages");
        }
    }

    /// Shows a message in the conversation, loading up to it if it is
    /// older than the pages on screen.
    pub(super) fn jump_to(&self, msg_id: &str) {
        self.search.bar.set_search_mode(false);
        if self.position_of_message(msg_id).is_some() {
            return self.reveal_message(msg_id);
        }
        let Some((account_id, chat_jid)) = self.open_ids() else {
            return;
        };
        let request = request::Kind::ListMessages(ipc::ListMessages {
            account_id,
            chat_jid,
            from_id: msg_id.to_owned(),
            ..Default::default()
        });
        if let Some(id) = self.send(request) {
            self.search.jump_request.set(id);
            self.search.jump_target.replace(Some(msg_id.to_owned()));
        }
    }

    /// Replaces the conversation with the messages from a search result on.
    pub(super) fn show_from(&self, msgs: Vec<ipc::Message>) {
        let Some(target) = self.search.jump_target.take() else {
            return;
        };
        if !msgs.first().is_some_and(|m| self.is_open(m)) {
            return; // Another chat was opened meanwhile.
        }
        let items: Vec<_> = msgs
            .into_iter()
            .rev()
            .map(glib::BoxedAnyObject::new)
            .collect();
        self.messages.splice(0, self.messages.n_items(), &items);
        self.has_more.set(true); // Older pages load as usual when scrolled up to.
        self.reveal_message(&target);
    }

    /// Scrolls to a loaded message and highlights it for a moment.
    fn reveal_message(&self, msg_id: &str) {
        let Some(pos) = self.position_of_message(msg_id) else {
            return;
        };
        self.stick_bottom.set(false);
        self.message_list
            .scroll_to(pos, gtk::ListScrollFlags::NONE, None);
        let (rows, id) = (self.rows.clone(), msg_id.to_owned());
        // The row is bound once the list has scrolled to it.
        glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
            let bubble = rows
                .borrow()
                .get(&id)
                .and_then(|r| r.status.upgrade())
                .and_then(|status| status.parent())
                .and_then(|footer| footer.parent());
            if let Some(bubble) = bubble {
                bubble.add_css_class("found");
                glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                    bubble.remove_css_class("found");
                });
            }
        });
    }

    fn position_of_message(&self, msg_id: &str) -> Option<u32> {
        (0..self.messages.n_items()).find(|&i| {
            self.messages
                .item(i)
                .and_downcast::<glib::BoxedAnyObject>()
                .is_some_and(|m| m.borrow::<ipc::Message>().id == msg_id)
        })
    }
}
