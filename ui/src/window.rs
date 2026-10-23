//! The main window and its reaction to daemon events.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::ipc::{self, event, request};

/// Widest a message bubble gets, in pixels, however wide the window.
const BUBBLE_MAX_WIDTH: i32 = 520;

/// Messages fetched per request when opening a chat or scrolling back.
const PAGE_SIZE: u32 = 50;

const CSS: &str = "
.bubble { border-radius: 12px; padding: 6px 10px; background: var(--card-bg-color); }
.bubble.mine { background: color-mix(in srgb, var(--accent-bg-color) 35%, var(--card-bg-color)); }
.composer { border-radius: 10px; background: var(--card-bg-color); }
";

/// Status label of each bound message row, by message id, so receipts
/// update a row in place instead of replacing it in the model, which makes
/// the list re-measure and lose its scroll position.
type StatusLabels = Rc<RefCell<std::collections::HashMap<String, glib::WeakRef<gtk::Label>>>>;

pub struct Window {
    status_labels: StatusLabels,
    window: adw::ApplicationWindow,
    /// Last chat reported to the daemon as being looked at, as "account/chat".
    reported_active: RefCell<String>,
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
    content_stack: gtk::Stack,
    messages: gio::ListStore,
    message_list: gtk::ListView,
    message_scroll: gtk::ScrolledWindow,
    loading: Cell<bool>,
    has_more: Cell<bool>,
    /// Keep the newest message in view while the list grows, until the user scrolls up.
    stick_bottom: Cell<bool>,
    composer: gtk::TextView,
    content_view: adw::ToolbarView,
    /// Text of each SendText request awaiting its reply, by request id, so
    /// a message that fails can be put back.
    sending: RefCell<std::collections::HashMap<u64, String>>,
    /// Chat that was open when the window last closed, reopened once chats load.
    restore_chat: RefCell<Option<String>>,
    conn: RefCell<Option<ipc::Connection>>,
    account: RefCell<Option<ipc::Account>>,
}

impl Window {
    pub fn build(app: &adw::Application) -> Rc<Self> {
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
        let status_labels = StatusLabels::default();
        let messages = gio::ListStore::new::<glib::BoxedAnyObject>();
        let message_list = gtk::ListView::builder()
            .model(&gtk::NoSelection::new(Some(messages.clone())))
            .factory(&message_row_factory(status_labels.clone()))
            .build();
        let message_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&message_list)
            .build();
        let content_stack = gtk::Stack::new();
        content_stack.add_named(&placeholder, Some("empty"));
        content_stack.add_named(&message_scroll, Some("messages"));
        let composer = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(10)
            .right_margin(10)
            .css_classes(["composer"])
            .build();
        composer.update_property(&[gtk::accessible::Property::Label("Message")]);
        // Grows with the text up to a few lines, then scrolls. No scrollbar:
        // reserving room for one would make the empty field two lines tall,
        // and the text view keeps the cursor in sight on its own.
        let composer_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .propagate_natural_height(true)
            .max_content_height(140)
            .hexpand(true)
            .child(&composer)
            .build();
        let send_button = gtk::Button::builder()
            .icon_name("mail-send-symbolic")
            .tooltip_text("Send")
            .valign(gtk::Align::End)
            .css_classes(["circular", "suggested-action"])
            .build();
        let composer_bar = gtk::Box::builder()
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        composer_bar.append(&composer_scroll);
        composer_bar.append(&send_button);
        let content_view = with_header(&content_stack);
        content_view.add_bottom_bar(&composer_bar);
        content_view.set_reveal_bottom_bars(false);
        let content = adw::NavigationPage::new(&content_view, "Wassup");
        let split = adw::NavigationSplitView::builder()
            .sidebar(&adw::NavigationPage::new(&sidebar, "Chats"))
            .content(&content)
            .build();

        let stack = gtk::Stack::new();
        stack.add_named(&with_header(&status), Some("status"));
        stack.add_named(&with_header(&link), Some("link"));
        stack.add_named(&split, Some("main"));

        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("a display"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));
        let saved = SavedState::load(&state_path());
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Wassup")
            .default_width(if saved.width > 0 { saved.width } else { 900 })
            .default_height(if saved.height > 0 { saved.height } else { 640 })
            .maximized(saved.maximized)
            .content(&toasts)
            .build();
        let narrow = adw::Breakpoint::new(
            adw::BreakpointCondition::parse("max-width: 600sp").expect("valid condition"),
        );
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(narrow);

        let ui = Rc::new(Self {
            status_labels,
            window: window.clone(),
            reported_active: RefCell::new(String::new()),
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
            content_stack,
            messages,
            message_list,
            message_scroll,
            loading: Cell::new(false),
            has_more: Cell::new(false),
            stick_bottom: Cell::new(false),
            composer,
            content_view,
            sending: RefCell::default(),
            restore_chat: RefCell::new(saved.chat),
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
        let adj = ui.message_scroll.vadjustment();
        // Rows get their real height only once drawn, so the bottom keeps moving.
        adj.connect_changed(glib::clone!(
            #[weak]
            ui,
            move |_| {
                if ui.stick_bottom.get() {
                    ui.show_last_row();
                }
            }
        ));
        adj.connect_value_changed(glib::clone!(
            #[weak]
            ui,
            move |adj| {
                let at_bottom = adj.value() + adj.page_size() >= adj.upper() - 1.0;
                ui.stick_bottom.set(at_bottom);
            }
        ));
        ui.message_scroll.connect_edge_reached(glib::clone!(
            #[weak]
            ui,
            move |_, edge| {
                if edge == gtk::PositionType::Top && ui.has_more.get() && !ui.loading.get() {
                    let first = ui.messages.item(0).and_downcast::<glib::BoxedAnyObject>();
                    if let Some(first) = first {
                        let id = first.borrow::<ipc::Message>().id.clone();
                        ui.load_messages(id);
                    }
                }
            }
        ));
        // Enter sends; Shift+Enter starts a new line.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak]
            ui,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                let enter = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter);
                if enter && !modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
                    ui.send_text();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        ));
        ui.composer.add_controller(keys);
        send_button.connect_clicked(glib::clone!(
            #[weak]
            ui,
            move |_| ui.send_text()
        ));
        window.connect_close_request(glib::clone!(
            #[weak]
            ui,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |window| {
                let (width, height) = window.default_size();
                let state = SavedState {
                    width,
                    height,
                    maximized: window.is_maximized(),
                    chat: ui.current_chat.borrow().clone(),
                };
                if let Err(e) = state.save(&state_path()) {
                    eprintln!("wassup: couldn't save window state: {e}");
                }
                glib::Propagation::Proceed
            }
        ));
        let report = glib::clone!(
            #[weak]
            ui,
            move || ui.report_active()
        );
        ui.window.connect_is_active_notify({
            let report = report.clone();
            move |_| report()
        });
        ui.split.connect_show_content_notify({
            let report = report.clone();
            move |_| report()
        });
        ui.split.connect_collapsed_notify(move |_| report());
        ui.connect();
        ui
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn connect_destroy(&self, f: impl Fn() + 'static) {
        self.window.connect_destroy(move |_| f());
    }

    /// Closes the window if the user is looking at it, shows it otherwise.
    /// Closing ends the UI process, which is the point: it frees its memory.
    pub fn toggle(&self) {
        if self.window.is_active() {
            self.window.close();
        } else {
            self.present();
        }
    }

    /// Shows a chat, e.g. after a notification click.
    pub fn show_chat(&self, account_id: &str, chat_jid: &str) {
        self.present();
        let other_account = self
            .account
            .borrow()
            .as_ref()
            .is_some_and(|a| a.id != account_id);
        if other_account {
            return; // TODO: switch account once the UI supports several.
        }
        match self.position_of(chat_jid) {
            Some(pos) => {
                self.chat_selection.set_selected(pos);
                self.split.set_show_content(true);
            }
            // Chats are not loaded yet; open it when they are.
            None => {
                self.restore_chat.replace(Some(chat_jid.to_owned()));
            }
        }
    }

    /// Tells the daemon which chat is on screen, so it does not notify about it.
    fn report_active(&self) {
        let showing = !self.split.is_collapsed() || self.split.shows_content();
        let (account_id, chat_jid) = match self.open_ids() {
            Some(ids) if self.window.is_active() && showing => ids,
            _ => Default::default(),
        };
        let key = format!("{account_id}/{chat_jid}");
        if *self.reported_active.borrow() == key {
            return;
        }
        self.reported_active.replace(key);
        self.send(request::Kind::SetActiveChat(ipc::SetActiveChat {
            account_id,
            chat_jid,
        }));
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
        // The daemon forgets the active chat of a closed connection.
        self.reported_active.replace("/".to_owned());
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

    /// Sends a request; returns its id, or None if it could not be sent.
    pub fn send(&self, kind: request::Kind) -> Option<u64> {
        let sent = self.conn.borrow().as_ref()?.send(kind);
        sent.inspect_err(|e| self.toast(&format!("Couldn't reach the daemon: {e}")))
            .ok()
    }

    fn handle(self: &Rc<Self>, event: ipc::Event) {
        let pending = self.sending.borrow_mut().remove(&event.request_id);
        if let Some(text) = pending {
            let buffer = self.composer.buffer();
            // Put a failed message back so it can be sent again, unless the
            // user has already started a new one.
            if !matches!(event.kind, Some(event::Kind::MessageReceived(_)))
                && buffer.char_count() == 0
            {
                buffer.set_text(&text);
            }
        }
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
                let restore = self.restore_chat.take();
                if let Some(pos) = restore.and_then(|jid| self.position_of(&jid)) {
                    self.chat_selection.set_selected(pos);
                }
            }
            Some(event::Kind::ChatChanged(chat)) if self.is_current(&chat.account_id) => {
                self.upsert_chat(chat)
            }
            Some(event::Kind::Messages(list)) => self.add_page(list.messages),
            Some(event::Kind::MessageReceived(msg)) if self.is_open(&msg) => self.add_message(msg),
            Some(event::Kind::MessageStatusChanged(change)) => self.update_status(change),
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
        self.report_active();
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
        self.report_active();
        self.content_stack.set_visible_child_name("messages");
        self.content_view.set_reveal_bottom_bars(true);
        self.composer.grab_focus();
        self.messages.remove_all();
        self.load_messages(String::new());
        if chat.unread_count > 0 {
            self.mark_read();
        }
    }

    fn send_text(&self) {
        let buffer = self.composer.buffer();
        let (start, end) = buffer.bounds();
        let text = buffer.text(&start, &end, false).trim().to_owned();
        let Some((account_id, chat_jid)) = self.open_ids() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        // The composer stays usable while sending: disabling it would move
        // the keyboard focus into the message list and scroll it.
        let request = request::Kind::SendText(ipc::SendText {
            account_id,
            chat_jid,
            text: text.clone(),
        });
        if let Some(id) = self.send(request) {
            buffer.set_text("");
            self.sending.borrow_mut().insert(id, text);
        }
    }

    fn is_open(&self, msg: &ipc::Message) -> bool {
        self.is_current(&msg.account_id)
            && self.current_chat.borrow().as_deref() == Some(msg.chat_jid.as_str())
    }

    /// Account and chat ids of the open chat.
    fn open_ids(&self) -> Option<(String, String)> {
        let account = self.account.borrow().as_ref()?.id.clone();
        Some((account, self.current_chat.borrow().clone()?))
    }

    fn load_messages(&self, before_id: String) {
        let Some((account_id, chat_jid)) = self.open_ids() else {
            return;
        };
        self.loading.set(true);
        self.send(request::Kind::ListMessages(ipc::ListMessages {
            account_id,
            chat_jid,
            before_id,
            limit: PAGE_SIZE,
        }));
    }

    fn mark_read(&self) {
        if let Some((account_id, chat_jid)) = self.open_ids() {
            self.send(request::Kind::MarkRead(ipc::MarkRead {
                account_id,
                chat_jid,
            }));
        }
    }

    /// Prepends an older page; the first page also scrolls to the newest message.
    fn add_page(&self, page: Vec<ipc::Message>) {
        self.loading.set(false);
        // A page for a chat that is no longer open is dropped.
        if !page.first().is_some_and(|m| self.is_open(m)) {
            self.has_more.set(false);
            return;
        }
        self.has_more.set(page.len() == PAGE_SIZE as usize);
        let first_page = self.messages.n_items() == 0;
        let items: Vec<_> = page
            .into_iter()
            .rev()
            .map(glib::BoxedAnyObject::new)
            .collect();
        self.messages.splice(0, 0, &items);
        if first_page {
            self.scroll_to_bottom();
        }
    }

    fn add_message(&self, msg: ipc::Message) {
        let incoming = !msg.from_me;
        // Own messages always come into view; others only if already following.
        if !incoming {
            self.scroll_to_bottom();
        }
        self.messages.append(&glib::BoxedAnyObject::new(msg));
        let window = self.toasts.root().and_downcast::<gtk::Window>();
        if incoming && window.is_some_and(|w| w.is_active()) {
            self.mark_read();
        }
    }

    fn update_status(&self, change: ipc::MessageStatusChanged) {
        let open = self.is_current(&change.account_id)
            && self.current_chat.borrow().as_deref() == Some(change.chat_jid.as_str());
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
            let hit = change
                .message_ids
                .contains(&item.borrow::<ipc::Message>().id);
            if hit {
                let mut msg = item.borrow_mut::<ipc::Message>();
                msg.status = change.status;
                let label = self
                    .status_labels
                    .borrow()
                    .get(&msg.id)
                    .and_then(|l| l.upgrade());
                if let Some(label) = label {
                    show_status(&label, change.status());
                }
            }
        }
    }

    fn scroll_to_bottom(&self) {
        self.stick_bottom.set(true);
        self.show_last_row();
    }

    /// Scrolls through the list view rather than by setting the adjustment:
    /// the list keeps its own anchor, and moving the adjustment behind its
    /// back makes the next row resize (a receipt, say) jump to the top.
    fn show_last_row(&self) {
        let n = self.messages.n_items();
        if n > 0 {
            self.message_list
                .scroll_to(n - 1, gtk::ListScrollFlags::NONE, None);
        }
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

/// Window state kept between runs, so closing the UI to save memory does
/// not lose the user's place.
#[derive(Debug, Default, PartialEq)]
struct SavedState {
    width: i32,
    height: i32,
    maximized: bool,
    chat: Option<String>,
}

fn state_path() -> std::path::PathBuf {
    glib::user_state_dir().join("wassup").join("ui.ini")
}

impl SavedState {
    /// Missing or unreadable state just means defaults.
    fn load(path: &std::path::Path) -> Self {
        let file = glib::KeyFile::new();
        if file.load_from_file(path, glib::KeyFileFlags::NONE).is_err() {
            return Self::default();
        }
        Self {
            width: file.integer("window", "width").unwrap_or(0),
            height: file.integer("window", "height").unwrap_or(0),
            maximized: file.boolean("window", "maximized").unwrap_or(false),
            chat: file
                .string("window", "chat")
                .ok()
                .map(String::from)
                .filter(|chat| !chat.is_empty()),
        }
    }

    fn save(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        let file = glib::KeyFile::new();
        file.set_integer("window", "width", self.width);
        file.set_integer("window", "height", self.height);
        file.set_boolean("window", "maximized", self.maximized);
        file.set_string("window", "chat", self.chat.as_deref().unwrap_or_default());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        file.save_to_file(path)?;
        Ok(())
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
    if chat.name.is_empty() {
        user_label(&chat.jid)
    } else {
        chat.name.clone()
    }
}

/// How to show a user without a known name. Hidden (LID) ids are not phone
/// numbers, so they are never shown as one.
fn user_label(jid: &str) -> String {
    match jid.split_once('@') {
        Some((user, "s.whatsapp.net")) => format!("+{user}"),
        Some((_, "lid")) => "Unknown contact".to_owned(),
        Some((user, _)) => user.to_owned(),
        None => jid.to_owned(),
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

fn time_of_day(ms: i64) -> String {
    glib::DateTime::from_unix_local(ms / 1000)
        .and_then(|at| at.format("%H:%M"))
        .map(Into::into)
        .unwrap_or_default()
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

fn show_status(label: &gtk::Label, status: ipc::MessageStatus) {
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

fn message_row_factory(status_labels: StatusLabels) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        item.set_activatable(false);
        // A focused row would scroll the list to itself.
        item.set_focusable(false);
        item.set_selectable(false);
        let sender = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["caption-heading", "accent"])
            .build();
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
    let bound = status_labels.clone();
    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let msg = item.item().and_downcast::<glib::BoxedAnyObject>().unwrap();
        let msg = msg.borrow::<ipc::Message>();
        let clamp = item.child().and_then(|row| row.first_child()).unwrap();
        let bubble = clamp.first_child().unwrap();
        let sender = bubble.first_child().and_downcast::<gtk::Label>().unwrap();
        let text = sender.next_sibling().and_downcast::<gtk::Label>().unwrap();
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
        text.set_label(&msg.text);
        time.set_label(&time_of_day(msg.timestamp_ms));
        show_status(&status, msg.status());
        bound
            .borrow_mut()
            .insert(msg.id.clone(), status.downgrade());
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
            status_labels
                .borrow_mut()
                .remove(&msg.borrow::<ipc::Message>().id);
        }
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
    fn saved_state_round_trips() {
        let path = std::env::temp_dir()
            .join(format!("wassup-state-{}", std::process::id()))
            .join("ui.ini");
        assert_eq!(SavedState::load(&path), SavedState::default());
        let state = SavedState {
            width: 1000,
            height: 700,
            maximized: true,
            chat: Some("123@g.us".into()),
        };
        state.save(&path).unwrap();
        assert_eq!(SavedState::load(&path), state);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn hidden_ids_are_not_shown_as_numbers() {
        assert_eq!(user_label("5215550000000@s.whatsapp.net"), "+5215550000000");
        assert_eq!(user_label("99128348550@lid"), "Unknown contact");
    }

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
