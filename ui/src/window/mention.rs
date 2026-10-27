//! Mentioning people in a group: typing "@" offers the members of the group.

use std::collections::{HashMap, HashSet};

use super::*;

/// Most members offered at once.
const OFFERED: usize = 8;

/// The list of members under the composer, and what was picked from it.
pub(super) struct Mentions {
    popover: gtk::Popover,
    list: gtk::ListBox,
    /// Members of each group, once the daemon has answered.
    members: RefCell<HashMap<String, Vec<ipc::Participant>>>,
    /// Groups whose members were asked for.
    asked: RefCell<HashSet<String>>,
    /// The members the list shows now.
    offered: RefCell<Vec<ipc::Participant>>,
    /// Who was picked for the message being written.
    picked: RefCell<Vec<ipc::Participant>>,
}

impl Mentions {
    pub(super) fn new(composer: &gtk::TextView) -> Self {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .max_content_height(240)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let popover = gtk::Popover::builder()
            .child(&scroll)
            .autohide(false)
            .has_arrow(false)
            .position(gtk::PositionType::Top)
            .can_focus(false)
            .build();
        popover.set_parent(composer);
        composer.connect_destroy(glib::clone!(
            #[weak]
            popover,
            move |_| popover.unparent()
        ));
        Self {
            popover,
            list,
            members: RefCell::default(),
            asked: RefCell::default(),
            offered: RefCell::default(),
            picked: RefCell::default(),
        }
    }
}

impl Window {
    pub(super) fn connect_mentions(self: &Rc<Self>) {
        let update = glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move || ui.update_mentions()
        );
        self.composer.buffer().connect_changed({
            let update = update.clone();
            move |_| update()
        });
        self.composer
            .buffer()
            .connect_notify_local(Some("cursor-position"), move |_, _| update());
        self.mentions.list.connect_row_activated(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_, row| ui.accept_mention(row.index() as usize)
        ));
    }

    /// The "@word" before the cursor: where it starts and the word after the "@".
    fn mention_query(&self) -> Option<(gtk::TextIter, String)> {
        let buffer = self.composer.buffer();
        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        let mut start = cursor;
        while start.backward_char() {
            let c = start.char();
            if c.is_whitespace() {
                start.forward_char();
                break;
            }
        }
        if start.char() != '@' {
            return None;
        }
        let query = buffer.text(&start, &cursor, false);
        Some((start, query[1..].to_lowercase()))
    }

    fn update_mentions(&self) {
        let m = &self.mentions;
        let group = self.open_ids().filter(|(_, chat)| chat.ends_with("@g.us"));
        let (Some((account_id, chat_jid)), Some((_, query))) = (group, self.mention_query()) else {
            m.popover.popdown();
            return;
        };
        let offered: Vec<_> = {
            let members = m.members.borrow();
            let Some(people) = members.get(&chat_jid) else {
                if m.asked.borrow_mut().insert(chat_jid.clone()) {
                    self.send(request::Kind::ListParticipants(ipc::ListParticipants {
                        account_id,
                        chat_jid,
                    }));
                }
                return;
            };
            people
                .iter()
                .filter(|p| p.name.to_lowercase().contains(&query))
                .take(OFFERED)
                .cloned()
                .collect()
        };
        if offered.is_empty() {
            m.popover.popdown();
            return;
        }
        m.list.remove_all();
        for person in &offered {
            let avatar = adw::Avatar::new(28, Some(&person.name), true);
            if !person.avatar_path.is_empty() {
                show_avatar(&avatar, &person.avatar_path, || true);
            }
            let label = gtk::Label::builder()
                .label(&person.name)
                .xalign(0.0)
                .hexpand(true)
                .build();
            let row = gtk::Box::builder()
                .spacing(10)
                .margin_top(4)
                .margin_bottom(4)
                .margin_start(8)
                .margin_end(10)
                .build();
            row.append(&avatar);
            row.append(&label);
            m.list.append(&row);
        }
        m.list.select_row(m.list.row_at_index(0).as_ref());
        *m.offered.borrow_mut() = offered;
        // Under the "@" being typed, where the cursor is.
        if let Some((at, _)) = self.mention_query() {
            let r = self.composer.iter_location(&at);
            let (x, y) =
                self.composer
                    .buffer_to_window_coords(gtk::TextWindowType::Widget, r.x(), r.y());
            m.popover
                .set_pointing_to(Some(&gdk::Rectangle::new(x, y, 1, r.height())));
        }
        m.popover.popup();
    }

    /// The list answers Up, Down, Enter, Tab and Escape while it shows.
    pub(super) fn mention_key(&self, key: gdk::Key) -> bool {
        let m = &self.mentions;
        if !m.popover.is_visible() {
            return false;
        }
        let selected = m.list.selected_row().map_or(0, |r| r.index());
        match key {
            gdk::Key::Down => m
                .list
                .select_row(m.list.row_at_index(selected + 1).as_ref()),
            gdk::Key::Up => m
                .list
                .select_row(m.list.row_at_index(selected - 1).as_ref()),
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::Tab => {
                self.accept_mention(selected as usize)
            }
            gdk::Key::Escape => m.popover.popdown(),
            _ => return false,
        }
        true
    }

    /// Writes "@name" in place of the "@word" being typed.
    fn accept_mention(&self, index: usize) {
        let m = &self.mentions;
        let (Some(person), Some((mut start, _))) =
            (m.offered.borrow().get(index).cloned(), self.mention_query())
        else {
            return;
        };
        let buffer = self.composer.buffer();
        let mut cursor = buffer.iter_at_mark(&buffer.get_insert());
        buffer.delete(&mut start, &mut cursor);
        buffer.insert(&mut start, &format!("@{} ", person.name));
        m.popover.popdown();
        m.picked.borrow_mut().push(person);
    }

    pub(super) fn participants_arrived(&self, list: ipc::ParticipantList) {
        self.mentions
            .members
            .borrow_mut()
            .insert(list.chat_jid, list.participants);
        self.update_mentions();
    }

    /// The text to send with the names written as WhatsApp wants mentions, and
    /// the ids of the people picked who are still in it. Forgets who was picked.
    pub(super) fn take_mentions(&self, text: &str) -> (String, Vec<String>) {
        mention_ids(
            text,
            std::mem::take(&mut *self.mentions.picked.borrow_mut()),
        )
    }
}

/// Writes each picked "@name" of `text` as "@" and the user part of the id.
fn mention_ids(text: &str, mut picked: Vec<ipc::Participant>) -> (String, Vec<String>) {
    // A name that begins another is replaced after it.
    picked.sort_by_key(|p| std::cmp::Reverse(p.name.len()));
    let (mut text, mut ids) = (text.to_owned(), Vec::new());
    for person in picked {
        let written = format!("@{}", person.name);
        if let Some(user) = person
            .jid
            .split('@')
            .next()
            .filter(|_| text.contains(&written))
        {
            text = text.replace(&written, &format!("@{user}"));
            ids.push(person.jid);
        }
    }
    (text, ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str, jid: &str) -> ipc::Participant {
        ipc::Participant {
            name: name.into(),
            jid: jid.into(),
            ..Default::default()
        }
    }

    #[test]
    fn picked_names_become_numbers_and_gone_ones_are_dropped() {
        let picked = vec![
            person("Ana", "111@lid"),
            person("Ana López", "222@lid"),
            person("Luis", "333@lid"),
        ];
        let (text, ids) = mention_ids("hi @Ana López and @Ana, bye", picked);
        assert_eq!(text, "hi @222 and @111, bye");
        assert_eq!(ids, ["222@lid", "111@lid"]);
    }
}
