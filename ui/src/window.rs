//! The main window and its reaction to daemon events.

use std::cell::{Cell, RefCell};
use std::os::unix::fs::DirBuilderExt;
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
.media { border-radius: 8px; background: alpha(currentColor, 0.08); }
.document { border-radius: 8px; padding: 8px 10px; background: alpha(currentColor, 0.08); }
.bubble.mine { background: color-mix(in srgb, var(--accent-bg-color) 35%, var(--card-bg-color)); }
/* After .mine, which has the same weight: own stickers have no bubble either. */
.bubble.sticker { background: none; padding: 0; }
.composer { border-radius: 10px; background: var(--card-bg-color); }
";

/// Widgets of each bound message row, by message id, so receipts and
/// downloads update a row in place instead of replacing it in the model,
/// which makes the list re-measure and lose its scroll position.
type BoundRows = Rc<RefCell<std::collections::HashMap<String, RowWidgets>>>;

struct RowWidgets {
    status: glib::WeakRef<gtk::Label>,
    picture: glib::WeakRef<gtk::Picture>,
    audio: glib::WeakRef<gtk::Box>,
}

/// The audio being played. Only one plays at a time, and the stream exists
/// only while it does.
struct Player {
    msg_id: String,
    duration: u32,
    media: gtk::MediaFile,
}

/// Lets the row factory, built before the window, reach it later.
type Owner = Rc<std::cell::OnceCell<std::rc::Weak<Window>>>;

pub struct Window {
    /// Shown over the current page while reconnecting after a disconnection.
    banner: adw::Banner,
    /// Whether any connection succeeded; until then failures fill the window.
    ever_connected: Cell<bool>,
    reconnect_attempts: Cell<u32>,
    reconnect_pending: Cell<bool>,
    rows: BoundRows,
    /// Messages whose media was asked for and has not arrived yet.
    media_requested: RefCell<std::collections::HashSet<String>>,
    /// Chats whose picture was asked for in this connection.
    avatar_requested: RefCell<std::collections::HashSet<String>>,
    header_avatar: adw::Avatar,
    header_title: gtk::Label,
    player: RefCell<Option<Player>>,
    /// Audio to play as soon as its download ends.
    play_when_ready: RefCell<std::collections::HashSet<String>>,
    /// Files to open as soon as their download ends.
    open_when_ready: RefCell<std::collections::HashSet<String>>,
    /// "Sending…" notice of each upload, by request id.
    uploads: RefCell<std::collections::HashMap<u64, adw::Toast>>,
    sticker_popover: gtk::Popover,
    /// Favorites synced from the phone, then recent stickers, each with its heading.
    sticker_sections: [(gtk::Label, gtk::FlowBox); 2],
    sticker_empty: gtk::Label,
    /// Stickers made here, deleted once the daemon has them, by request id.
    sticker_files: RefCell<std::collections::HashMap<u64, std::path::PathBuf>>,
    /// Picker pictures waiting for their sticker's download, by message id.
    sticker_pictures: RefCell<std::collections::HashMap<String, glib::WeakRef<gtk::Picture>>>,
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
    current_chat: Rc<RefCell<Option<String>>>,
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

        let owner = Owner::default();
        let chats = gio::ListStore::new::<glib::BoxedAnyObject>();
        let chat_selection = gtk::SingleSelection::builder()
            .model(&chats)
            .autoselect(false)
            .can_unselect(true)
            .build();
        let chat_list = gtk::ListView::builder()
            .model(&chat_selection)
            .factory(&chat_row_factory(owner.clone()))
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
        let rows = BoundRows::default();
        let messages = gio::ListStore::new::<glib::BoxedAnyObject>();
        let message_list = gtk::ListView::builder()
            .model(&gtk::NoSelection::new(Some(messages.clone())))
            .factory(&message_row_factory(rows.clone(), owner.clone()))
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
        let attach = gtk::Button::builder()
            .icon_name("mail-attachment-symbolic")
            .tooltip_text("Send a File")
            .valign(gtk::Align::End)
            .css_classes(["circular", "flat"])
            .build();
        // Sticker picker: stickers recently sent or received.
        let sticker_sections = ["Favorites", "Recent"].map(|title| {
            let heading = gtk::Label::builder()
                .label(title)
                .xalign(0.0)
                .css_classes(["heading"])
                .build();
            let grid = gtk::FlowBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .homogeneous(true)
                .min_children_per_line(4)
                .max_children_per_line(6)
                .build();
            (heading, grid)
        });
        let sticker_lists = gtk::Box::new(gtk::Orientation::Vertical, 6);
        for (heading, grid) in &sticker_sections {
            sticker_lists.append(heading);
            sticker_lists.append(grid);
        }
        let sticker_empty = gtk::Label::builder()
            .label("Your favorite stickers and the ones you send show up here.")
            .wrap(true)
            .css_classes(["dim-label"])
            .build();
        let create_sticker = gtk::Button::builder()
            .label("Create from Image…")
            .css_classes(["flat"])
            .build();
        let sticker_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        sticker_box.append(&create_sticker);
        sticker_box.append(&sticker_empty);
        sticker_box.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .min_content_width(STICKER_THUMB * 5)
                .max_content_height(STICKER_THUMB * 4)
                .propagate_natural_height(true)
                .child(&sticker_lists)
                .build(),
        );
        let sticker_popover = gtk::Popover::builder().child(&sticker_box).build();
        let stickers_button = gtk::MenuButton::builder()
            .icon_name("face-smile-symbolic")
            .tooltip_text("Stickers")
            .valign(gtk::Align::End)
            .popover(&sticker_popover)
            .css_classes(["circular", "flat"])
            .build();
        composer_bar.append(&attach);
        composer_bar.append(&stickers_button);
        composer_bar.append(&composer_scroll);
        composer_bar.append(&send_button);
        // The conversation's header shows the chat's picture next to its name.
        let header_avatar = adw::Avatar::builder().size(28).show_initials(true).build();
        let header_title = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["heading"])
            .build();
        let header_box = gtk::Box::builder().spacing(8).visible(false).build();
        header_box.append(&header_avatar);
        header_box.append(&header_title);
        let content_header = adw::HeaderBar::new();
        content_header.set_title_widget(Some(&header_box));
        let content_view = adw::ToolbarView::new();
        content_view.add_top_bar(&content_header);
        content_view.set_content(Some(&content_stack));
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
        let banner = adw::Banner::builder()
            .title("Reconnecting to the Wassup daemon…")
            .button_label("Retry")
            .build();
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.append(&banner);
        page.append(&stack);
        stack.set_vexpand(true);
        toasts.set_child(Some(&page));
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
            banner,
            ever_connected: Cell::new(false),
            reconnect_attempts: Cell::new(0),
            reconnect_pending: Cell::new(false),
            rows,
            media_requested: RefCell::default(),
            open_when_ready: RefCell::default(),
            player: RefCell::default(),
            play_when_ready: RefCell::default(),
            avatar_requested: RefCell::default(),
            header_avatar,
            header_title,
            uploads: RefCell::default(),
            sticker_popover,
            sticker_sections,
            sticker_empty,
            sticker_files: RefCell::default(),
            sticker_pictures: RefCell::default(),
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
            current_chat: Rc::default(),
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
        ui.banner.connect_button_clicked(glib::clone!(
            #[weak]
            ui,
            move |_| ui.connect()
        ));
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
        ui.sticker_popover.connect_show(glib::clone!(
            #[weak]
            ui,
            move |_| {
                if let Some(account) = ui.account.borrow().as_ref() {
                    ui.send(request::Kind::ListStickers(ipc::ListStickers {
                        account_id: account.id.clone(),
                    }));
                }
            }
        ));
        create_sticker.connect_clicked(glib::clone!(
            #[weak]
            ui,
            move |_| ui.choose_sticker_image()
        ));
        attach.connect_clicked(glib::clone!(
            #[weak]
            ui,
            move |_| ui.choose_file()
        ));
        owner.set(Rc::downgrade(&ui)).ok();
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
        if self.conn.borrow().is_some() {
            return; // A pending retry raced with a manual one.
        }
        let (conn, events) = match ipc::Connection::open() {
            Ok(pair) => pair,
            Err(e) => return self.disconnected("Can't reach the Wassup daemon", &e.to_string()),
        };
        self.conn.replace(Some(conn));
        self.reconnect_attempts.set(0);
        self.banner.set_revealed(false);
        if self.ever_connected.get() {
            // Reopen the chat once chats load, so messages missed while
            // disconnected show up.
            let open = self.current_chat.take();
            self.restore_chat.replace(open);
        }
        // Events may have been missed while disconnected.
        self.chats_loaded.set(false);
        self.media_requested.borrow_mut().clear();
        self.avatar_requested.borrow_mut().clear();
        // The daemon forgets the active chat of a closed connection.
        self.reported_active.replace("/".to_owned());
        if !self.ever_connected.get() {
            self.show_status("Connecting…", "", false);
        }
        self.send(request::Kind::ListAccounts(ipc::ListAccounts {}));

        let ui = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(event) = events.recv().await {
                let Some(ui) = ui.upgrade() else { break };
                match event {
                    Ok(event) => ui.handle(event),
                    Err(e) => {
                        ui.conn.take();
                        ui.disconnected("Lost the connection to the daemon", &e.to_string());
                        break;
                    }
                }
            }
        });
    }

    /// Retries the connection with growing pauses, since the daemon usually
    /// comes back on its own, e.g. after an update restarts it. Before the
    /// first success the failure fills the window; after it, a banner keeps
    /// the current page in place.
    fn disconnected(self: &Rc<Self>, title: &str, detail: &str) {
        if self.ever_connected.get() {
            self.banner.set_revealed(true);
        } else {
            self.show_status(title, detail, true);
        }
        if self.reconnect_pending.replace(true) {
            return;
        }
        let attempt = self.reconnect_attempts.get();
        self.reconnect_attempts.set(attempt + 1);
        let delay = [1, 2, 4, 8][attempt.min(3) as usize];
        let ui = Rc::downgrade(self);
        glib::timeout_add_seconds_local_once(delay, move || {
            if let Some(ui) = ui.upgrade() {
                ui.reconnect_pending.set(false);
                ui.connect();
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
        if let Some(toast) = self.uploads.borrow_mut().remove(&event.request_id) {
            toast.dismiss();
        }
        if let Some(file) = self.sticker_files.borrow_mut().remove(&event.request_id) {
            std::fs::remove_file(file).ok();
        }
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
                self.ever_connected.set(true);
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
            Some(event::Kind::MediaReady(ready)) => self.media_ready(ready),
            Some(event::Kind::Stickers(list)) => self.show_stickers(list.stickers),
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

    fn show_header(&self, chat: &ipc::Chat) {
        self.header_title.set_label(&chat_title(chat));
        avatar_placeholder(&self.header_avatar, chat);
        if let Some(header) = self.header_title.parent() {
            header.set_visible(true);
        }
        if !chat.avatar_path.is_empty() {
            let (current, jid) = (self.current_chat.clone(), chat.jid.clone());
            show_avatar(&self.header_avatar, &chat.avatar_path, move || {
                current.borrow().as_deref() == Some(jid.as_str())
            });
        }
    }

    /// Asks the daemon for a chat's picture, once per connection.
    fn request_avatar(&self, chat: &ipc::Chat) {
        if !self.avatar_requested.borrow_mut().insert(chat.jid.clone()) {
            return;
        }
        self.send(request::Kind::FetchAvatar(ipc::FetchAvatar {
            account_id: chat.account_id.clone(),
            chat_jid: chat.jid.clone(),
        }));
    }

    /// Moves or inserts a chat, keeping the list newest first.
    fn upsert_chat(&self, chat: ipc::Chat) {
        if self.current_chat.borrow().as_deref() == Some(chat.jid.as_str()) {
            self.show_header(&chat);
        }
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
        self.stop_audio();
        self.current_chat.replace(Some(chat.jid.clone()));
        self.content.set_title(&chat_title(chat));
        self.show_header(chat);
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

    fn choose_file(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title("Send a File")
            .modal(true)
            .build();
        let ui = Rc::downgrade(self);
        dialog.open(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                let (Some(ui), Ok(file)) = (ui.upgrade(), result) else {
                    return; // Closed, or cancelled.
                };
                if let Some(path) = file.path() {
                    ui.send_file(&path.to_string_lossy());
                }
            },
        );
    }

    /// Sends a file, with whatever is in the composer as its caption.
    fn send_file(&self, path: &str) {
        let Some((account_id, chat_jid)) = self.open_ids() else {
            return;
        };
        let buffer = self.composer.buffer();
        let (start, end) = buffer.bounds();
        let caption = buffer.text(&start, &end, false).trim().to_owned();
        let request = request::Kind::SendMedia(ipc::SendMedia {
            account_id,
            chat_jid,
            path: path.to_owned(),
            caption: caption.clone(),
            as_sticker: false,
        });
        if let Some(id) = self.send(request) {
            buffer.set_text("");
            self.sending.borrow_mut().insert(id, caption);
            let name = std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let toast = adw::Toast::builder()
                .title(format!("Sending {name}…"))
                .timeout(0)
                .build();
            self.toasts.add_toast(toast.clone());
            self.uploads.borrow_mut().insert(id, toast);
        }
    }

    /// Opens a message's file with the desktop's default application,
    /// downloading it first if needed.
    fn open_media(&self, msg: &ipc::Message) {
        let Some(media) = &msg.media else { return };
        if media.path.is_empty() {
            self.open_when_ready.borrow_mut().insert(msg.id.clone());
            self.request_media(msg);
        } else {
            self.launch(&media.path);
        }
    }

    /// Plays or pauses a message's audio, downloading it first if needed.
    fn toggle_audio(self: &Rc<Self>, msg: &ipc::Message) {
        let Some(media) = &msg.media else { return };
        let current = self
            .player
            .borrow()
            .as_ref()
            .filter(|p| p.msg_id == msg.id)
            .map(|p| p.media.clone());
        if let Some(stream) = current {
            stream.set_playing(!stream.is_playing());
        } else if media.path.is_empty() {
            self.play_when_ready.borrow_mut().insert(msg.id.clone());
            self.request_media(msg);
        } else {
            self.start_audio(&msg.id, &media.path);
        }
    }

    fn start_audio(self: &Rc<Self>, msg_id: &str, path: &str) {
        self.stop_audio();
        let duration = (0..self.messages.n_items())
            .filter_map(|i| self.messages.item(i).and_downcast::<glib::BoxedAnyObject>())
            .find_map(|item| {
                let m = item.borrow::<ipc::Message>();
                (m.id == msg_id).then(|| m.media.as_ref().map_or(0, |a| a.duration_seconds))
            })
            .unwrap_or(0);
        let media = gtk::MediaFile::for_filename(path);
        let ui = Rc::downgrade(self);
        let refresh = {
            let ui = ui.clone();
            move || {
                if let Some(ui) = ui.upgrade() {
                    ui.show_audio_state();
                }
            }
        };
        media.connect_timestamp_notify({
            let refresh = refresh.clone();
            move |_| refresh()
        });
        media.connect_playing_notify(move |_| refresh());
        media.connect_ended_notify(move |stream| {
            if let Some(ui) = ui.upgrade().filter(|_| stream.is_ended()) {
                ui.stop_audio();
            }
        });
        media.connect_error_notify(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |stream| {
                if let Some(e) = stream.error() {
                    ui.toast(&format!("Couldn't play the audio: {e}"));
                    ui.stop_audio();
                }
            }
        ));
        media.play();
        self.player.replace(Some(Player {
            msg_id: msg_id.to_owned(),
            duration,
            media,
        }));
        self.show_audio_state();
    }

    /// Frees the player; its row goes back to the start.
    fn stop_audio(&self) {
        let Some(player) = self.player.take() else {
            return;
        };
        player.media.pause();
        let row = self
            .rows
            .borrow()
            .get(&player.msg_id)
            .and_then(|r| r.audio.upgrade());
        if let Some(row) = row {
            show_audio(&row, player.duration, None);
        }
    }

    fn seek_audio(&self, msg_id: &str, seconds: f64) {
        if let Some(player) = self.player.borrow().as_ref().filter(|p| p.msg_id == msg_id) {
            player.media.seek((seconds * 1_000_000.0) as i64);
        }
    }

    /// Updates the playing message's row, if it is on screen.
    fn show_audio_state(&self) {
        let player = self.player.borrow();
        let Some(player) = player.as_ref() else {
            return;
        };
        let row = self
            .rows
            .borrow()
            .get(&player.msg_id)
            .and_then(|r| r.audio.upgrade());
        if let Some(row) = row {
            show_audio(&row, player.duration, Some(&player.media));
        }
    }

    fn launch(&self, path: &str) {
        gtk::FileLauncher::new(Some(&gio::File::for_path(path))).launch(
            Some(&self.window),
            None::<&gio::Cancellable>,
            |result| {
                if let Err(e) = result {
                    eprintln!("wassup: couldn't open file: {e}");
                }
            },
        );
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

    /// Asks the daemon to download a message's media, once.
    fn request_media(&self, msg: &ipc::Message) {
        if !self.media_requested.borrow_mut().insert(msg.id.clone()) {
            return;
        }
        self.send(request::Kind::DownloadMedia(ipc::DownloadMedia {
            account_id: msg.account_id.clone(),
            chat_jid: msg.chat_jid.clone(),
            message_id: msg.id.clone(),
        }));
    }

    fn show_stickers(self: &Rc<Self>, stickers: Vec<ipc::Sticker>) {
        for (heading, grid) in &self.sticker_sections {
            while let Some(child) = grid.first_child() {
                grid.remove(&child);
            }
            heading.set_visible(false);
        }
        self.sticker_pictures.borrow_mut().clear();
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
                .build();
            let button = gtk::Button::builder()
                .child(&picture)
                .css_classes(["flat"])
                .build();
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

    fn choose_sticker_image(self: &Rc<Self>) {
        self.sticker_popover.popdown();
        let images = gtk::FileFilter::new();
        images.set_name(Some("Images"));
        images.add_mime_type("image/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&images);
        let dialog = gtk::FileDialog::builder()
            .title("Create a Sticker")
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
    async fn send_new_sticker(&self, image: &std::path::Path) {
        let webp = match crate::sticker::from_file(image).await {
            Ok(webp) => webp,
            Err(e) => return self.toast(&format!("Couldn't make a sticker: {e}")),
        };
        let Some((account_id, chat_jid)) = self.open_ids() else {
            return;
        };
        // The daemon reads the file from disk; media never goes over the socket.
        let dir = glib::user_cache_dir().join("wassup").join("outgoing");
        let file = dir.join(format!("sticker-{}.webp", glib::monotonic_time()));
        let written = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .and_then(|_| std::fs::write(&file, webp));
        if let Err(e) = written {
            return self.toast(&format!("Couldn't make a sticker: {e}"));
        }
        let request = request::Kind::SendMedia(ipc::SendMedia {
            account_id,
            chat_jid,
            path: file.to_string_lossy().into_owned(),
            caption: String::new(),
            as_sticker: true,
        });
        match self.send(request) {
            Some(id) => {
                let toast = adw::Toast::builder()
                    .title("Sending sticker…")
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

    fn send_sticker(&self, sticker: &ipc::Sticker) {
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

    fn media_ready(self: &Rc<Self>, ready: ipc::MediaReady) {
        self.media_requested.borrow_mut().remove(&ready.message_id);
        let waiting = self.sticker_pictures.borrow_mut().remove(&ready.message_id);
        if let Some(picture) = waiting.and_then(|p| p.upgrade())
            && ready.error.is_empty()
        {
            show_thumb(&picture, &ready.path);
        }
        let open = self.open_when_ready.borrow_mut().remove(&ready.message_id);
        if !ready.error.is_empty() {
            eprintln!("wassup: couldn't download media: {}", ready.error);
            if open {
                self.toast("Couldn't download the file");
            }
            return;
        }
        if open {
            self.launch(&ready.path);
        }
        if self.play_when_ready.borrow_mut().remove(&ready.message_id) {
            self.start_audio(&ready.message_id, &ready.path);
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
            if msg.id != ready.message_id || msg.chat_jid != ready.chat_jid {
                continue;
            }
            let Some(media) = msg.media.as_mut() else {
                break;
            };
            media.path = ready.path.clone();
            if matches!(
                media.kind(),
                ipc::MediaKind::Document | ipc::MediaKind::Audio
            ) {
                break; // Nothing to draw.
            }
            let size = media_size(media);
            let picture = self
                .rows
                .borrow()
                .get(&msg.id)
                .and_then(|r| r.picture.upgrade());
            if let Some(picture) = picture {
                let (width, height) = size;
                show_media(&picture, &ready.path, width, height, &msg.id, &self.rows);
            }
            break;
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
                    .rows
                    .borrow()
                    .get(&msg.id)
                    .and_then(|r| r.status.upgrade());
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

fn chat_row_factory(owner: Owner) -> gtk::SignalListItemFactory {
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

fn message_row_factory(rows: BoundRows, owner: Owner) -> gtk::SignalListItemFactory {
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
        let picture = sender
            .next_sibling()
            .and_downcast::<gtk::Picture>()
            .unwrap();
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
const IMAGE_MAX: i32 = 280;
const STICKER_MAX: i32 = 160;

/// On-screen size of media, scaled down to fit and keeping its proportions.
fn media_size(media: &ipc::Media) -> (i32, i32) {
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
const TEXTURE_CACHE: usize = 32;

/// Decodes an image file already scaled to fit width x height logical
/// pixels: a full 1600 px photo would take about 8 MB of memory, a 280 px
/// one about 0.3 MB. Decoding runs in glycin's sandboxed loaders, off the
/// main loop. Recently decoded files come from a small cache.
fn load_texture(
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
fn show_media(
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
fn avatar_placeholder(avatar: &adw::Avatar, chat: &ipc::Chat) {
    avatar.set_custom_image(None::<&gdk::Paintable>);
    avatar.set_text(Some(&chat.name));
    avatar.set_show_initials(!chat.is_group && !chat.name.is_empty());
    avatar.set_icon_name(Some(if chat.is_group {
        "system-users-symbolic"
    } else {
        "avatar-default-symbolic"
    }));
}

/// Draws an audio row: play or pause, how far it got, and the time.
fn show_audio(row: &gtk::Box, duration: u32, playing: Option<&gtk::MediaFile>) {
    let play = row
        .first_child()
        .and_then(|w| w.next_sibling())
        .and_downcast::<gtk::Button>()
        .unwrap();
    let progress = play.next_sibling().and_downcast::<gtk::Scale>().unwrap();
    let length = progress
        .next_sibling()
        .and_downcast::<gtk::Label>()
        .unwrap();
    let total = match playing.map(|m| m.duration()) {
        Some(us) if us > 0 => us as f64 / 1e6,
        _ => f64::from(duration.max(1)),
    };
    let at = playing.map_or(0.0, |m| m.timestamp() as f64 / 1e6);
    let running = playing.is_some_and(|m| m.is_playing());
    play.set_icon_name(if running {
        "media-playback-pause-symbolic"
    } else {
        "media-playback-start-symbolic"
    });
    play.set_tooltip_text(Some(if running { "Pause" } else { "Play" }));
    progress.set_range(0.0, total);
    progress.set_value(at);
    length.set_label(&clock(if playing.is_some() {
        at
    } else {
        f64::from(duration)
    }));
}

/// "m:ss", the way chat apps show audio lengths.
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Size of a sticker in the picker, in pixels.
const STICKER_THUMB: i32 = 64;

fn show_thumb(picture: &gtk::Picture, path: &str) {
    let scale = picture.scale_factor().max(1);
    let picture = picture.downgrade();
    load_texture(path, STICKER_THUMB, STICKER_THUMB, scale, move |texture| {
        if let Some(picture) = picture.upgrade() {
            picture.set_paintable(Some(&texture));
        }
    });
}

/// Puts a chat's cached picture in an avatar, unless by the time it is
/// decoded the avatar shows another chat (`still_for` says whether it does).
fn show_avatar(avatar: &adw::Avatar, path: &str, still_for: impl Fn() -> bool + 'static) {
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

    #[test]
    fn audio_clock() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(7.9), "0:07");
        assert_eq!(clock(125.0), "2:05");
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
