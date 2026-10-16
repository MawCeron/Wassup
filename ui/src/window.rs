//! The main window and its reaction to daemon events.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::ipc::{self, event, request};

pub struct Window {
    stack: gtk::Stack,
    status: adw::StatusPage,
    retry: gtk::Button,
    toasts: adw::ToastOverlay,
    link: adw::StatusPage,
    qr: gtk::Picture,
    link_retry: gtk::Button,
    linking: Cell<bool>,
    split: adw::NavigationSplitView,
    sidebar_title: adw::WindowTitle,
    content: adw::NavigationPage,
    chats: gio::ListStore,
    chat_selection: gtk::SingleSelection,
    chats_loaded: Cell<bool>,
    current_chat: RefCell<Option<String>>,
    conn: RefCell<Option<ipc::Connection>>,
    account: RefCell<Option<ipc::Account>>,
}

impl Window {
    pub fn build(app: &adw::Application) -> adw::ApplicationWindow {
        let retry = gtk::Button::builder()
            .label("Retry")
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .build();
        let status = adw::StatusPage::builder().child(&retry).build();
        let qr = gtk::Picture::builder()
            .width_request(264)
            .height_request(264)
            .halign(gtk::Align::Center)
            .alternative_text("QR code to link this device")
            .build();
        let link_retry = gtk::Button::builder()
            .label("Try Again")
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .build();
        let link_box = gtk::Box::new(gtk::Orientation::Vertical, 24);
        link_box.append(&qr);
        link_box.append(&link_retry);
        let link = adw::StatusPage::builder()
            .title("Link This Device")
            .child(&link_box)
            .build();

        let chats = gio::ListStore::new::<glib::BoxedAnyObject>();
        let chat_selection = gtk::SingleSelection::builder()
            .model(&chats)
            .autoselect(false)
            .can_unselect(true)
            .build();
        let chat_list = gtk::ListView::builder()
            .model(&chat_selection)
            .factory(&chat_row_factory())
            .css_classes(["navigation-sidebar"])
            .build();
        let sidebar_title = adw::WindowTitle::new("Chats", "");
        let sidebar_header = adw::HeaderBar::new();
        sidebar_header.set_title_widget(Some(&sidebar_title));
        let sidebar = adw::ToolbarView::new();
        sidebar.add_top_bar(&sidebar_header);
        sidebar.set_content(Some(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&chat_list)
                .build(),
        ));
        let placeholder = adw::StatusPage::builder()
            .icon_name("chat-message-new-symbolic")
            .title("No Chat Selected")
            .build();
        let content = adw::NavigationPage::new(&with_header(&placeholder), "Wassup");
        let split = adw::NavigationSplitView::builder()
            .sidebar(&adw::NavigationPage::new(&sidebar, "Chats"))
            .content(&content)
            .build();

        let stack = gtk::Stack::new();
        stack.add_named(&with_header(&status), Some("status"));
        stack.add_named(&with_header(&link), Some("link"));
        stack.add_named(&split, Some("main"));

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Wassup")
            .default_width(900)
            .default_height(640)
            .content(&toasts)
            .build();
        let narrow = adw::Breakpoint::new(
            adw::BreakpointCondition::parse("max-width: 600sp").expect("valid condition"),
        );
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(narrow);

        let ui = Rc::new(Self {
            stack,
            status,
            retry,
            toasts,
            link,
            qr,
            link_retry,
            linking: Cell::new(false),
            split,
            sidebar_title,
            content,
            chats,
            chat_selection,
            chats_loaded: Cell::new(false),
            current_chat: RefCell::new(None),
            conn: RefCell::new(None),
            account: RefCell::new(None),
        });
        // The button owns the state for as long as the window lives.
        ui.retry.connect_clicked(glib::clone!(
            #[strong]
            ui,
            move |_| ui.connect()
        ));
        ui.link_retry.connect_clicked(glib::clone!(
            #[weak]
            ui,
            move |_| ui.start_link()
        ));
        ui.chat_selection.connect_selected_item_notify(glib::clone!(
            #[weak]
            ui,
            move |selection| {
                // Re-sorting a chat briefly unselects it; that is not a user action.
                if let Some(item) = selection.selected_item() {
                    let chat = item.downcast::<glib::BoxedAnyObject>().unwrap();
                    ui.open_chat(&chat.borrow::<ipc::Chat>());
                }
            }
        ));
        ui.connect();
        window
    }

    fn connect(self: &Rc<Self>) {
        let (conn, events) = match ipc::Connection::open() {
            Ok(pair) => pair,
            Err(e) => {
                return self.show_status("Can't reach the Wassup daemon", &e.to_string(), true);
            }
        };
        self.conn.replace(Some(conn));
        // Events may have been missed while disconnected.
        self.chats_loaded.set(false);
        self.show_status("Connecting…", "", false);
        self.send(request::Kind::ListAccounts(ipc::ListAccounts {}));

        let ui = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(event) = events.recv().await {
                let Some(ui) = ui.upgrade() else { break };
                match event {
                    Ok(event) => ui.handle(event),
                    Err(e) => {
                        ui.conn.take();
                        ui.show_status("Lost the connection to the daemon", &e.to_string(), true);
                        break;
                    }
                }
            }
        });
    }

    pub fn send(&self, kind: request::Kind) {
        let failed = match &*self.conn.borrow() {
            Some(conn) => conn.send(kind).err(),
            None => return,
        };
        if let Some(e) = failed {
            self.toast(&format!("Couldn't reach the daemon: {e}"));
        }
    }

    fn handle(self: &Rc<Self>, event: ipc::Event) {
        match event.kind {
            Some(event::Kind::Error(e)) => self.toast(&e.message),
            Some(event::Kind::Accounts(list)) => {
                // TODO: pick the account once the UI supports several.
                match list.accounts.into_iter().next() {
                    Some(account) => self.set_account(account),
                    None => self.show_status("No account", "The daemon has no account yet.", false),
                }
            }
            Some(event::Kind::AccountChanged(account)) if self.is_current(&account.id) => {
                self.set_account(account)
            }
            Some(event::Kind::Chats(list)) => {
                self.chats.remove_all();
                for chat in list.chats {
                    self.chats.append(&glib::BoxedAnyObject::new(chat));
                }
                self.reselect_current();
            }
            Some(event::Kind::ChatChanged(chat)) if self.is_current(&chat.account_id) => {
                self.upsert_chat(chat)
            }
            Some(event::Kind::QrCode(qr)) if self.is_current(&qr.account_id) => {
                self.qr.set_paintable(Some(&qr_texture(&qr.code)));
                self.link.set_description(Some(
                    "Open WhatsApp on your phone, go to Linked Devices and scan this code.",
                ));
            }
            Some(event::Kind::LinkFailed(failed)) if self.is_current(&failed.account_id) => {
                self.linking.set(false);
                self.qr.set_paintable(None::<&gdk::Paintable>);
                self.link
                    .set_description(Some(&format!("Linking failed: {}", failed.reason)));
                self.link_retry.set_visible(true);
            }
            _ => {}
        }
    }

    fn is_current(&self, account_id: &str) -> bool {
        self.account
            .borrow()
            .as_ref()
            .is_some_and(|a| a.id == account_id)
    }

    fn set_account(self: &Rc<Self>, account: ipc::Account) {
        let linked = !account.jid.is_empty();
        let name = account.push_name.clone();
        let state = account.state();
        self.account.replace(Some(account));
        if !linked {
            self.stack.set_visible_child_name("link");
            if !self.linking.get() {
                self.start_link();
            }
            return;
        }
        self.linking.set(false);
        let subtitle = match state {
            ipc::ConnectionState::Connected => "",
            ipc::ConnectionState::Connecting => "Connecting…",
            _ => "Offline",
        };
        self.sidebar_title.set_subtitle(subtitle);
        self.sidebar_title
            .set_title(if name.is_empty() { "Chats" } else { &name });
        self.stack.set_visible_child_name("main");
        if !self.chats_loaded.replace(true) {
            let account_id = self.account.borrow().as_ref().unwrap().id.clone();
            self.send(request::Kind::ListChats(ipc::ListChats { account_id }));
        }
    }

    fn chat_at(&self, pos: u32) -> Option<glib::BoxedAnyObject> {
        self.chats.item(pos).and_downcast()
    }

    fn position_of(&self, jid: &str) -> Option<u32> {
        (0..self.chats.n_items()).find(|&i| {
            self.chat_at(i)
                .is_some_and(|c| c.borrow::<ipc::Chat>().jid == jid)
        })
    }

    /// Moves or inserts a chat, keeping the list newest first.
    fn upsert_chat(&self, chat: ipc::Chat) {
        if let Some(pos) = self.position_of(&chat.jid) {
            self.chats.remove(pos);
        }
        let pos = (0..self.chats.n_items())
            .find(|&i| {
                self.chat_at(i).is_some_and(|c| {
                    c.borrow::<ipc::Chat>().last_message_at_ms < chat.last_message_at_ms
                })
            })
            .unwrap_or(self.chats.n_items());
        self.chats.insert(pos, &glib::BoxedAnyObject::new(chat));
        self.reselect_current();
    }

    fn reselect_current(&self) {
        let current = self.current_chat.borrow().clone();
        if let Some(pos) = current.and_then(|jid| self.position_of(&jid)) {
            self.chat_selection.set_selected(pos);
        }
    }

    fn open_chat(&self, chat: &ipc::Chat) {
        self.split.set_show_content(true);
        if self.current_chat.borrow().as_deref() == Some(chat.jid.as_str()) {
            return;
        }
        self.current_chat.replace(Some(chat.jid.clone()));
        self.content.set_title(&chat_title(chat));
    }

    fn start_link(&self) {
        let Some(id) = self.account.borrow().as_ref().map(|a| a.id.clone()) else {
            return;
        };
        self.linking.set(true);
        self.link_retry.set_visible(false);
        self.qr.set_paintable(None::<&gdk::Paintable>);
        self.link.set_description(Some("Preparing a code…"));
        self.send(request::Kind::LinkAccount(ipc::LinkAccount {
            account_id: id,
        }));
    }

    fn show_status(&self, title: &str, description: &str, retry: bool) {
        self.status
            .set_icon_name(retry.then_some("network-offline-symbolic"));
        self.status.set_title(title);
        self.status.set_description(Some(description));
        self.retry.set_visible(retry);
        self.stack.set_visible_child_name("status");
    }

    fn toast(&self, message: &str) {
        self.toasts.add_toast(adw::Toast::new(message));
    }
}

/// Wraps a page in a toolbar view with a plain header bar.
fn with_header(child: &impl IsA<gtk::Widget>) -> adw::ToolbarView {
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(child));
    view
}

/// A chat's name, or its phone number when the name is unknown.
fn chat_title(chat: &ipc::Chat) -> String {
    if !chat.name.is_empty() {
        return chat.name.clone();
    }
    match chat.jid.split_once('@') {
        Some((user, "s.whatsapp.net")) => format!("+{user}"),
        Some((user, _)) => user.to_owned(),
        None => chat.jid.clone(),
    }
}

/// Time of day for today, the date otherwise, in the user's locale.
fn format_time(ms: i64) -> String {
    let (Ok(at), Ok(now)) = (
        glib::DateTime::from_unix_local(ms / 1000),
        glib::DateTime::now_local(),
    ) else {
        return String::new();
    };
    let format = if at.ymd() == now.ymd() { "%H:%M" } else { "%x" };
    at.format(format).map(Into::into).unwrap_or_default()
}

fn chat_row_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
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
        row.append(&title);
        row.append(&time);
        row.append(&unread);
        item.downcast_ref::<gtk::ListItem>()
            .unwrap()
            .set_child(Some(&row));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let chat = item.item().and_downcast::<glib::BoxedAnyObject>().unwrap();
        let chat = chat.borrow::<ipc::Chat>();
        let row = item.child().unwrap();
        let title = row.first_child().and_downcast::<gtk::Label>().unwrap();
        let time = title.next_sibling().and_downcast::<gtk::Label>().unwrap();
        let unread = time.next_sibling().and_downcast::<gtk::Label>().unwrap();
        title.set_label(&chat_title(&chat));
        time.set_label(&format_time(chat.last_message_at_ms));
        unread.set_label(&chat.unread_count.to_string());
        unread.set_visible(chat.unread_count > 0);
    });
    factory
}

/// Pixels per QR module, and the blank margin the QR spec asks for, in modules.
const QR_SCALE: usize = 8;
const QR_QUIET_ZONE: usize = 4;

/// Renders `code` as a grayscale bitmap: black modules on white. Returns
/// the pixels and the side length.
fn qr_pixels(code: &str) -> (Vec<u8>, usize) {
    use qrcodegen::{QrCode, QrCodeEcc};
    // Linking codes are a few hundred bytes, far below the QR limit.
    let qr = QrCode::encode_text(code, QrCodeEcc::Medium).expect("linking code fits in a QR");
    let side = (qr.size() as usize + 2 * QR_QUIET_ZONE) * QR_SCALE;
    let mut pixels = vec![255; side * side];
    for y in 0..side {
        for x in 0..side {
            let module = |p: usize| (p / QR_SCALE) as i32 - QR_QUIET_ZONE as i32;
            // get_module() is false outside the code, which draws the quiet zone.
            if qr.get_module(module(x), module(y)) {
                pixels[y * side + x] = 0;
            }
        }
    }
    (pixels, side)
}

fn qr_texture(code: &str) -> gdk::Texture {
    let (pixels, side) = qr_pixels(code);
    gdk::MemoryTexture::new(
        side as i32,
        side as i32,
        gdk::MemoryFormat::G8,
        &glib::Bytes::from_owned(pixels),
        side,
    )
    .upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_has_quiet_zone_and_finder() {
        let (pixels, side) = qr_pixels("https://wa.me/settings/linked_devices#test");
        let at = |x: usize, y: usize| pixels[y * side + x];
        assert_eq!(at(0, 0), 255, "quiet zone must be white");
        let edge = QR_QUIET_ZONE * QR_SCALE;
        assert_eq!(
            at(edge, edge),
            0,
            "finder pattern starts right after the quiet zone"
        );
        assert_eq!(at(edge - 1, edge - 1), 255);
    }
}
