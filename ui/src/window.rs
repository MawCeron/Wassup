//! The main window and its reaction to daemon events.

use std::cell::{Cell, RefCell};
use std::os::unix::fs::DirBuilderExt;
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, gio, glib};

use crate::ipc::{self, event, request};

mod audio;
mod preview;
mod qr;
mod reply;
mod rows;
mod search;
mod state;
mod stickers;
mod viewer;

use audio::*;
use qr::*;
use reply::*;
use rows::*;
use search::*;
use state::*;
use stickers::*;
use viewer::*;

/// Side of the QR code on the link screen, in pixels: easy to scan without
/// crowding the instructions.
const QR_SIZE: i32 = 240;

/// Widest a message bubble gets, in pixels, however wide the window.
const BUBBLE_MAX_WIDTH: i32 = 520;

/// Messages fetched per request when opening a chat or scrolling back.
const PAGE_SIZE: u32 = 50;

const CSS: &str = "
.bubble { border-radius: 12px; padding: 6px 10px; background: var(--card-bg-color); }
.media { border-radius: 8px; background: alpha(currentColor, 0.08); }
.document { border-radius: 8px; padding: 8px 10px; background: alpha(currentColor, 0.08); }
.quote { border-left: 3px solid var(--accent-color); border-radius: 6px; padding: 4px 8px; background: alpha(currentColor, 0.06); }
.media-button { padding: 0; }
.unread-pill { font-size: 0.8em; font-weight: bold; min-width: 12px; padding: 1px 6px; border-radius: 999px; background: var(--accent-bg-color); color: var(--accent-fg-color); }
/* Opaque, with a ring of the backdrop, as they overlap the bubble's edge. */
.reactions { font-size: 0.9em; padding: 2px 8px; border-radius: 999px; background: var(--popover-bg-color); box-shadow: 0 0 0 2px var(--view-bg-color); }
.reactions.mine { background: color-mix(in srgb, var(--accent-bg-color) 25%, var(--popover-bg-color)); }
.day-separator { font-size: 0.85em; font-weight: bold; padding: 4px 12px; border-radius: 8px; background: var(--popover-bg-color); box-shadow: 0 1px 2px alpha(black, 0.1); }
.reaction-mine { background: color-mix(in srgb, var(--accent-bg-color) 30%, transparent); }
.link-card { padding: 6px; border-radius: 8px; background: alpha(currentColor, 0.06); }
.link-thumb { border-radius: 6px; }
.video-badge { border-radius: 999px; padding: 10px; }
.video-length { border-radius: 6px; padding: 1px 6px; }
.viewer { background: rgba(0, 0, 0, 0.92); }
.viewer-text { color: white; }
.viewer-caption { background: rgba(0, 0, 0, 0.5); border-radius: 8px; padding: 6px 10px; }
.qr-card { background: white; border-radius: 12px; padding: 12px; }
.link-step { font-size: 1.15em; }
.bubble.mine { background: color-mix(in srgb, var(--accent-bg-color) 35%, var(--card-bg-color)); }
/* After .mine, which has the same weight: own stickers have no bubble either. */
.bubble.sticker { background: none; padding: 0; }
.composer { border-radius: 10px; background: var(--card-bg-color); outline: 0 solid transparent; outline-offset: -2px; }
/* The focus ring libadwaita draws on entries. */
.composer:focus-within { outline: 2px solid color-mix(in srgb, var(--accent-color) 50%, transparent); }
/* Clipped by the widget's overflow, so opaque stickers get round corners too. */
.sticker-image { border-radius: 12px; }
/* The doodles stay put while messages scroll over them. White bubbles
   need a darker backdrop than the white view. */
.chat-bg-light { background: color-mix(in srgb, var(--view-fg-color) 6%, var(--view-bg-color)) url(\"resource:///io/github/MawCeron/Wassup/backgrounds/wassup-pattern-light.svg\") repeat; }
.chat-bg-dark { background: var(--view-bg-color) url(\"resource:///io/github/MawCeron/Wassup/backgrounds/wassup-pattern-dark.svg\") repeat; }
.chat-bg-light listview, .chat-bg-dark listview { background: none; }
/* Links in own messages take the bubble's color, the accent being too close. */
.bubble.mine link { color: inherit; text-decoration-line: underline; }
/* Last, to win over .mine: a search result just jumped to. */
.bubble.found { background: color-mix(in srgb, var(--accent-bg-color) 60%, var(--card-bg-color)); transition: background 300ms; }
";

/// Widgets of each bound message row, by message id, so receipts and
/// downloads update a row in place instead of replacing it in the model,
/// which makes the list re-measure and lose its scroll position.
type BoundRows = Rc<RefCell<std::collections::HashMap<String, RowWidgets>>>;

struct RowWidgets {
    status: glib::WeakRef<gtk::Image>,
    picture: glib::WeakRef<gtk::Picture>,
    audio: glib::WeakRef<gtk::Box>,
    reactions: glib::WeakRef<gtk::Label>,
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
    /// What the link screen says is going on: waiting, or why it failed.
    link_status: gtk::Label,
    /// The QR code, a spinner while it comes, or an error icon.
    qr_stack: gtk::Stack,
    qr: gtk::Image,
    link_retry: gtk::Button,
    linking: Cell<bool>,
    split: adw::NavigationSplitView,
    sidebar_title: adw::WindowTitle,
    content: adw::NavigationPage,
    chats: gio::ListStore,
    chat_selection: gtk::SingleSelection,
    chat_list: gtk::ListView,
    chats_loaded: Cell<bool>,
    current_chat: Rc<RefCell<Option<String>>>,
    content_stack: gtk::Stack,
    messages: gio::ListStore,
    /// Everything under the photo viewer.
    page: gtk::Box,
    viewer: Rc<Viewer>,
    search: MessageSearch,
    reply: ReplyBar,
    message_list: gtk::ListView,
    message_scroll: gtk::ScrolledWindow,
    loading: Cell<bool>,
    has_more: Cell<bool>,
    /// Keep the newest message in view while the list grows, until the user scrolls up.
    stick_bottom: Cell<bool>,
    /// A scroll to the newest message is queued for after layout.
    pin_pending: Cell<bool>,
    composer: gtk::TextView,
    content_view: adw::ToolbarView,
    /// Text of each SendText request awaiting its reply, by request id, so
    /// a message that fails can be put back.
    sending: RefCell<std::collections::HashMap<u64, String>>,
    /// Chat that was open when the window last closed, reopened once chats load.
    restore_chat: RefCell<Option<String>>,
    /// Showing the chats locked on the phone, after their secret code.
    restricted: Rc<Cell<bool>>,
    chat_search: gtk::SearchEntry,
    chat_query: Rc<RefCell<String>>,
    /// The chat list, or a message when the search matches nothing.
    sidebar_stack: gtk::Stack,
    chat_filter: gtk::CustomFilter,
    shown_chats: gtk::FilterListModel,
    restricted_entry: gtk::Button,
    restricted_back: gtk::Button,
    /// Id of the UnlockChats request awaiting its reply, or 0.
    unlock_request: Cell<u64>,
    /// Id of the GetSettings request that opens the preferences, or 0.
    settings_request: Cell<u64>,
    conn: RefCell<Option<ipc::Connection>>,
    account: RefCell<Option<ipc::Account>>,
}

impl Window {
    pub fn build(app: &adw::Application) -> Rc<Self> {
        let retry = gtk::Button::builder()
            .label(gettext("Retry"))
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .build();
        let status = adw::StatusPage::builder().child(&retry).build();
        // Link screen: steps on one side, the QR code on the other; stacked
        // on narrow windows.
        let link_title = gtk::Label::builder()
            .label(gettext("Link Wassup to Your Phone"))
            .wrap(true)
            .xalign(0.0)
            .css_classes(["title-1"])
            .build();
        let steps = gtk::Box::new(gtk::Orientation::Vertical, 10);
        for (n, step) in [
            gettext("Open WhatsApp on your phone."),
            gettext("Open Settings, or the ⋮ menu on Android, and choose Linked devices."),
            gettext("Tap Link a device."),
            gettext("Point your phone at this screen to scan the code."),
        ]
        .iter()
        .enumerate()
        {
            steps.append(
                &gtk::Label::builder()
                    .label(format!("{}.  {step}", n + 1))
                    .wrap(true)
                    .xalign(0.0)
                    .css_classes(["link-step"])
                    .build(),
            );
        }
        let link_status = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        let link_retry = gtk::Button::builder()
            .label(gettext("Try Again"))
            .halign(gtk::Align::Start)
            .css_classes(["pill", "suggested-action"])
            .build();
        let link_text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        link_text.append(&link_title);
        link_text.append(&steps);
        link_text.append(&link_status);
        link_text.append(&link_retry);
        let qr = gtk::Image::builder()
            .pixel_size(QR_SIZE)
            .accessible_role(gtk::AccessibleRole::Img)
            .build();
        qr.update_property(&[gtk::accessible::Property::Label(&gettext(
            "QR code to link this device",
        ))]);
        let qr_stack = gtk::Stack::builder()
            .width_request(QR_SIZE)
            .height_request(QR_SIZE)
            .build();
        qr_stack.add_named(&adw::Spinner::new(), Some("waiting"));
        qr_stack.add_named(&qr, Some("code"));
        qr_stack.add_named(
            &gtk::Image::builder()
                .icon_name("dialog-error-symbolic")
                .pixel_size(64)
                .css_classes(["dim-label"])
                .build(),
            Some("failed"),
        );
        let qr_card = gtk::Box::builder()
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["qr-card"])
            .build();
        qr_card.append(&qr_stack);
        let link_columns = gtk::Box::builder()
            .spacing(48)
            .margin_top(36)
            .margin_bottom(36)
            .margin_start(24)
            .margin_end(24)
            .valign(gtk::Align::Center)
            .build();
        link_columns.append(&link_text);
        link_columns.append(&qr_card);
        let link = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(820)
                    .child(&link_columns)
                    .build(),
            )
            .build();

        let owner = Owner::default();
        let chats = gio::ListStore::new::<glib::BoxedAnyObject>();
        // Locked chats show only in the restricted view, and only there.
        let restricted = Rc::new(Cell::new(false));
        // What is typed in the chat search, folded by search_key.
        let chat_query = Rc::new(RefCell::new(String::new()));
        let chat_filter = gtk::CustomFilter::new({
            let (restricted, query) = (restricted.clone(), chat_query.clone());
            move |item| {
                let chat = item.downcast_ref::<glib::BoxedAnyObject>().unwrap();
                let chat = chat.borrow::<ipc::Chat>();
                chat.locked == restricted.get()
                    && search_key(&chat_title(&chat)).contains(query.borrow().as_str())
            }
        });
        let shown_chats = gtk::FilterListModel::new(Some(chats.clone()), Some(chat_filter.clone()));
        let chat_selection = gtk::SingleSelection::builder()
            .model(&shown_chats)
            .autoselect(false)
            .can_unselect(true)
            .build();
        let chat_list = gtk::ListView::builder()
            .model(&chat_selection)
            .factory(&chat_row_factory(owner.clone()))
            .css_classes(["navigation-sidebar"])
            .build();
        let sidebar_title = adw::WindowTitle::new(&gettext("Chats"), "");
        let sidebar_header = adw::HeaderBar::new();
        sidebar_header.set_title_widget(Some(&sidebar_title));
        let restricted_back = gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .tooltip_text(gettext("Lock Restricted Chats"))
            .visible(false)
            .build();
        sidebar_header.pack_start(&restricted_back);
        let menu = gio::Menu::new();
        menu.append(Some(&gettext("_Preferences")), Some("win.preferences"));
        menu.append(Some(&gettext("_Keyboard Shortcuts")), Some("win.shortcuts"));
        menu.append(Some(&gettext("_About Wassup")), Some("win.about"));
        sidebar_header.pack_end(
            &gtk::MenuButton::builder()
                .icon_name("open-menu-symbolic")
                .tooltip_text(gettext("Main Menu"))
                .primary(true)
                .menu_model(&menu)
                .build(),
        );
        let restricted_entry = gtk::Button::builder()
            .child(
                &adw::ButtonContent::builder()
                    .icon_name("wassup-lock-symbolic")
                    .label(gettext("Restricted Chats"))
                    .halign(gtk::Align::Start)
                    .build(),
            )
            .css_classes(["flat"])
            .margin_start(6)
            .margin_end(6)
            .visible(false)
            .build();
        let chat_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        chat_column.append(&restricted_entry);
        chat_column.append(&chat_list);
        let chat_search = gtk::SearchEntry::builder()
            .placeholder_text(gettext("Search chats"))
            .margin_start(6)
            .margin_end(6)
            .margin_bottom(6)
            .build();
        let no_chats_found = adw::StatusPage::builder()
            .icon_name("edit-find-symbolic")
            .title(gettext("No Chats Found"))
            .description(gettext("No chat or group has that name"))
            .css_classes(["compact"])
            .build();
        let sidebar_stack = gtk::Stack::new();
        sidebar_stack.add_named(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&chat_column)
                .build(),
            Some("chats"),
        );
        sidebar_stack.add_named(&no_chats_found, Some("empty"));
        let sidebar = adw::ToolbarView::new();
        sidebar.add_top_bar(&sidebar_header);
        sidebar.add_top_bar(&chat_search);
        sidebar.set_content(Some(&sidebar_stack));
        let placeholder = adw::StatusPage::builder()
            .icon_name("chat-message-new-symbolic")
            .title(gettext("No Chat Selected"))
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
        let search = MessageSearch::new();
        content_stack.add_named(&search.pages, Some("search"));
        let composer = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(10)
            .right_margin(10)
            .css_classes(["composer"])
            .build();
        composer.update_property(&[gtk::accessible::Property::Label(&gettext("Message"))]);
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
            .icon_name("wassup-send-symbolic")
            .tooltip_text(gettext("Send"))
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
            .icon_name("wassup-attach-symbolic")
            .tooltip_text(gettext("Send a File"))
            .valign(gtk::Align::End)
            .css_classes(["circular", "flat"])
            .build();
        // Sticker picker: stickers recently sent or received.
        let sticker_sections = [gettext("Favorites"), gettext("Recent")].map(|title| {
            let heading = gtk::Label::builder()
                .label(&title)
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
            .label(gettext(
                "Your favorite stickers and the ones you send show up here.",
            ))
            .wrap(true)
            .css_classes(["dim-label"])
            .build();
        let create_sticker = gtk::Button::builder()
            .label(gettext("Create from Image…"))
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
            .icon_name("wassup-stickers-symbolic")
            .tooltip_text(gettext("Stickers"))
            .valign(gtk::Align::End)
            .popover(&sticker_popover)
            .css_classes(["circular", "flat"])
            .build();
        // GTK's own emoji picker, inserting at the cursor.
        let emoji_chooser = gtk::EmojiChooser::new();
        let emoji_button = gtk::MenuButton::builder()
            .icon_name("wassup-emoji-symbolic")
            .tooltip_text(gettext("Emoji"))
            .valign(gtk::Align::End)
            .popover(&emoji_chooser)
            .css_classes(["circular", "flat"])
            .build();
        emoji_chooser.connect_emoji_picked(glib::clone!(
            #[weak]
            composer,
            move |_, emoji| {
                composer.buffer().insert_at_cursor(emoji);
                composer.grab_focus();
            }
        ));
        composer_bar.append(&attach);
        composer_bar.append(&emoji_button);
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
        // Searching makes sense only with a chat open, like the header.
        header_box
            .bind_property("visible", &search.button, "visible")
            .sync_create()
            .build();
        content_header.pack_end(&search.button);
        let content_view = adw::ToolbarView::new();
        content_view.add_top_bar(&content_header);
        content_view.add_top_bar(&search.bar);
        content_view.set_content(Some(&content_stack));
        let reply = ReplyBar::new();
        content_view.add_bottom_bar(&reply.revealer);
        content_view.add_bottom_bar(&composer_bar);
        content_view.set_reveal_bottom_bars(false);
        let content = adw::NavigationPage::new(&content_view, "Wassup");
        let split = adw::NavigationSplitView::builder()
            .sidebar(&adw::NavigationPage::new(&sidebar, &gettext("Chats")))
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
            .title(gettext("Reconnecting to the Wassup service…"))
            .button_label(gettext("Retry"))
            .build();
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.append(&banner);
        page.append(&stack);
        stack.set_vexpand(true);
        // The photo viewer covers the whole window; toasts stay above it.
        let viewer = Rc::new(Viewer::new());
        let root = gtk::Overlay::builder().child(&page).build();
        root.add_overlay(&viewer.root);
        toasts.set_child(Some(&root));
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
        narrow.add_setter(
            &link_columns,
            "orientation",
            Some(&gtk::Orientation::Vertical.to_value()),
        );
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
            link_status,
            qr_stack,
            qr,
            link_retry,
            linking: Cell::new(false),
            split,
            sidebar_title,
            content,
            chats,
            chat_selection,
            chat_list,
            restricted,
            chat_search,
            chat_query,
            sidebar_stack,
            chat_filter,
            shown_chats,
            restricted_entry,
            restricted_back,
            unlock_request: Cell::new(0),
            settings_request: Cell::new(0),
            chats_loaded: Cell::new(false),
            current_chat: Rc::default(),
            content_stack,
            messages,
            page,
            viewer,
            search,
            reply,
            message_list,
            message_scroll,
            loading: Cell::new(false),
            has_more: Cell::new(false),
            stick_bottom: Cell::new(false),
            pin_pending: Cell::new(false),
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
        let preferences = gio::SimpleAction::new("preferences", None);
        preferences.connect_activate(glib::clone!(
            #[weak]
            ui,
            move |_, _| {
                if let Some(id) = ui.send(request::Kind::GetSettings(ipc::GetSettings {})) {
                    ui.settings_request.set(id);
                }
            }
        ));
        ui.window.add_action(&preferences);
        app.set_accels_for_action("win.preferences", &["<Control>comma"]);
        ui.chat_search.connect_search_changed(glib::clone!(
            #[weak]
            ui,
            move |entry| ui.filter_chats(&entry.text())
        ));
        // Escape in the search box clears it.
        ui.chat_search
            .connect_stop_search(|entry| entry.set_text(""));
        let search_chats = gio::SimpleAction::new("search-chats", None);
        search_chats.connect_activate(glib::clone!(
            #[weak]
            ui,
            move |_, _| {
                ui.split.set_show_content(false);
                ui.chat_search.grab_focus();
            }
        ));
        ui.window.add_action(&search_chats);
        app.set_accels_for_action("win.search-chats", &["<Control>f"]);
        let shortcuts = gio::SimpleAction::new("shortcuts", None);
        shortcuts.connect_activate(glib::clone!(
            #[weak]
            ui,
            move |_, _| ui.show_shortcuts()
        ));
        ui.window.add_action(&shortcuts);
        app.set_accels_for_action("win.shortcuts", &["<Control>question"]);
        let about = gio::SimpleAction::new("about", None);
        about.connect_activate(glib::clone!(
            #[weak]
            ui,
            move |_, _| ui.show_about()
        ));
        ui.window.add_action(&about);
        // GtkWindow already has window.close, which leaves the daemon
        // notifying. Quit stops it too, until Wassup is opened again.
        app.set_accels_for_action("window.close", &["<Control>w"]);
        let quit = gio::SimpleAction::new("quit", None);
        quit.connect_activate(glib::clone!(
            #[weak]
            app,
            #[weak]
            ui,
            move |_, _| ui.confirm_quit(&app)
        ));
        app.add_action(&quit);
        app.set_accels_for_action("app.quit", &["<Control>q"]);
        ui.connect_viewer();
        ui.connect_message_search();
        ui.connect_reply_bar();
        let search_messages = gio::SimpleAction::new("search-messages", None);
        search_messages.connect_activate(glib::clone!(
            #[weak]
            ui,
            move |_, _| {
                if ui.current_chat.borrow().is_some() {
                    ui.search.bar.set_search_mode(true);
                    ui.search.entry.grab_focus();
                }
            }
        ));
        ui.window.add_action(&search_messages);
        app.set_accels_for_action("win.search-messages", &["<Control><Shift>f"]);
        ui.restricted_entry.connect_clicked(glib::clone!(
            #[weak]
            ui,
            move |_| ui.ask_secret_code()
        ));
        ui.restricted_back.connect_clicked(glib::clone!(
            #[weak]
            ui,
            move |_| ui.leave_restricted()
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
        // CSS media queries do not follow libadwaita's color scheme, so the
        // background tile is switched from here.
        let style = adw::StyleManager::default();
        let set_background = glib::clone!(
            #[weak(rename_to = scroll)]
            ui.message_scroll,
            move |style: &adw::StyleManager| {
                let dark = style.is_dark();
                scroll.set_css_classes(&[if dark {
                    "chat-bg-dark"
                } else {
                    "chat-bg-light"
                }]);
            }
        );
        set_background(&style);
        style.connect_dark_notify(set_background);
        let adj = ui.message_scroll.vadjustment();
        // Rows get their real height only once drawn, so the bottom keeps moving.
        adj.connect_changed(glib::clone!(
            #[weak]
            ui,
            move |_| {
                // This fires mid-layout, while the list still holds the old
                // row positions; scroll once the layout is done.
                if ui.stick_bottom.get() && !ui.pin_pending.replace(true) {
                    let ui = Rc::downgrade(&ui);
                    glib::idle_add_local_once(move || {
                        if let Some(ui) = ui.upgrade() {
                            ui.pin_pending.set(false);
                            if ui.stick_bottom.get() {
                                // Layout is done, so this moves the view the
                                // way a user's scroll would and the list
                                // re-anchors itself correctly.
                                let adj = ui.message_scroll.vadjustment();
                                adj.set_value(adj.upper() - adj.page_size());
                            }
                        }
                    });
                }
            }
        ));
        // Reaching the bottom, by any means, follows new messages again.
        adj.connect_value_changed(glib::clone!(
            #[weak]
            ui,
            move |adj| {
                if adj.value() + adj.page_size() >= adj.upper() - 1.0 {
                    ui.stick_bottom.set(true);
                }
            }
        ));
        // Only the user stops following: the list also moves the view itself,
        // e.g. when a new row turns out taller than estimated, and that must
        // not count as scrolling away.
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
        wheel.connect_scroll(glib::clone!(
            #[weak]
            ui,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, _, dy| {
                if dy < 0.0 {
                    ui.stick_bottom.set(false);
                }
                glib::Propagation::Proceed
            }
        ));
        ui.message_scroll.add_controller(wheel);
        let grab = gtk::GestureClick::new();
        grab.connect_pressed(glib::clone!(
            #[weak]
            ui,
            move |_, _, _, _| ui.stick_bottom.set(false)
        ));
        ui.message_scroll.vscrollbar().add_controller(grab);
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
        // Arrow keys in the chat list open each chat but keep the focus
        // there; Enter or a click moves on to writing.
        ui.chat_list.connect_activate(glib::clone!(
            #[weak]
            ui,
            move |_, _| {
                ui.composer.grab_focus();
            }
        ));
        let click = gtk::GestureClick::new();
        click.connect_released(glib::clone!(
            #[weak]
            ui,
            move |_, _, _, _| {
                // After the list has handled the click and focused the row.
                glib::idle_add_local_once(move || {
                    ui.composer.grab_focus();
                });
            }
        ));
        ui.chat_list.add_controller(click);
        // Enter sends; Shift+Enter starts a new line, Escape goes back to the chats.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak]
            ui,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                if key == gdk::Key::Escape {
                    if ui.cancel_reply() {
                        return glib::Propagation::Stop;
                    }
                    ui.split.set_show_content(false);
                    ui.chat_list.grab_focus();
                    return glib::Propagation::Stop;
                }
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
        // Stickers made by a previous run that never reached the daemon.
        if let Ok(entries) = std::fs::read_dir(outgoing_dir()) {
            for entry in entries.flatten() {
                std::fs::remove_file(entry.path()).ok();
            }
        }
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
        match self.shown_position(chat_jid) {
            Some(pos) => {
                self.chat_selection.set_selected(pos);
                self.split.set_show_content(true);
                self.composer.grab_focus();
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
            Err(e) => {
                return self
                    .disconnected(&gettext("Can't Reach the Wassup Service"), &e.to_string());
            }
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
            self.show_status(&gettext("Connecting…"), "", false);
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
                        ui.disconnected(
                            &gettext("Lost Connection to the Wassup Service"),
                            &e.to_string(),
                        );
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
        sent.inspect_err(|e| {
            // TRANSLATORS: {} is the error from the system.
            self.toast(
                &gettext("Couldn't reach the Wassup service: {}").replace("{}", &e.to_string()),
            )
        })
        .ok()
    }

    fn handle(self: &Rc<Self>, event: ipc::Event) {
        if event.request_id != 0 && event.request_id == self.unlock_request.get() {
            self.unlock_request.set(0);
            match event.kind {
                Some(event::Kind::Ack(_)) => self.enter_restricted(),
                Some(event::Kind::Error(e)) => {
                    // The daemon's errors are lowercase, Go style.
                    let mut chars = e.message.chars();
                    let message = chars
                        .next()
                        .map(|c| c.to_uppercase().chain(chars).collect::<String>())
                        .unwrap_or_default();
                    self.toast(&message);
                }
                _ => {}
            }
            return;
        }
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
                    None => self.show_status(
                        &gettext("No Account"),
                        &gettext("The Wassup service has no account yet."),
                        false,
                    ),
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
                self.show_restricted_entry();
                let restore = self.restore_chat.take();
                if let Some(pos) = restore.and_then(|jid| self.shown_position(&jid)) {
                    self.chat_selection.set_selected(pos);
                }
            }
            Some(event::Kind::ChatChanged(chat)) if self.is_current(&chat.account_id) => {
                self.upsert_chat(chat)
            }
            Some(event::Kind::Messages(list))
                if event.request_id != 0 && event.request_id == self.search.request.get() =>
            {
                self.show_results(list.messages)
            }
            Some(event::Kind::Messages(list))
                if event.request_id != 0 && event.request_id == self.search.jump_request.get() =>
            {
                self.search.jump_request.set(0);
                self.show_from(list.messages)
            }
            Some(event::Kind::Messages(list)) => self.add_page(list.messages),
            Some(event::Kind::MessageReceived(msg)) if self.is_open(&msg) => {
                if !msg.from_me {
                    self.announce(&msg);
                }
                self.add_message(msg)
            }
            Some(event::Kind::MessageStatusChanged(change)) => self.update_status(change),
            Some(event::Kind::MediaReady(ready)) => self.media_ready(ready),
            Some(event::Kind::MessagesExpired(gone)) => self.remove_expired(gone),
            Some(event::Kind::Stickers(list)) => self.show_stickers(list.stickers),
            Some(event::Kind::ReactionsChanged(changed)) => self.reactions_changed(changed),
            Some(event::Kind::Settings(settings))
                if event.request_id != 0 && event.request_id == self.settings_request.get() =>
            {
                self.settings_request.set(0);
                self.show_preferences(settings);
            }
            Some(event::Kind::QrCode(qr)) if self.is_current(&qr.account_id) => {
                self.qr.set_paintable(Some(&qr_texture(&qr.code)));
                self.qr_stack.set_visible_child_name("code");
                self.link_status
                    .set_label(&gettext("The code changes every minute or so."));
            }
            Some(event::Kind::LinkFailed(failed)) if self.is_current(&failed.account_id) => {
                self.linking.set(false);
                self.qr.set_paintable(None::<&gdk::Paintable>);
                self.qr_stack.set_visible_child_name("failed");
                self.link_status
                    // TRANSLATORS: {} is the reason, from WhatsApp.
                    .set_label(&gettext("Linking failed: {}").replace("{}", &failed.reason));
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
            ipc::ConnectionState::Connected => String::new(),
            ipc::ConnectionState::Connecting => gettext("Connecting…"),
            _ => gettext("Offline"),
        };
        self.sidebar_title.set_subtitle(&subtitle);
        self.sidebar_title.set_title(&if name.is_empty() {
            gettext("Chats")
        } else {
            name
        });
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

    fn filter_chats(&self, text: &str) {
        let query = search_key(text.trim());
        let wider = query.starts_with(self.chat_query.borrow().as_str());
        let narrower = self.chat_query.borrow().starts_with(query.as_str());
        self.chat_query.replace(query);
        self.chat_filter.changed(match (wider, narrower) {
            (true, false) => gtk::FilterChange::MoreStrict,
            (false, true) => gtk::FilterChange::LessStrict,
            _ => gtk::FilterChange::Different,
        });
        let nothing = self.shown_chats.n_items() == 0 && !self.chat_query.borrow().is_empty();
        self.sidebar_stack
            .set_visible_child_name(if nothing { "empty" } else { "chats" });
        self.reselect_current();
    }

    /// Position of a chat in the list on screen, which hides either the
    /// locked chats or the others.
    fn shown_position(&self, jid: &str) -> Option<u32> {
        (0..self.shown_chats.n_items()).find(|&i| {
            self.shown_chats
                .item(i)
                .and_downcast::<glib::BoxedAnyObject>()
                .is_some_and(|c| c.borrow::<ipc::Chat>().jid == jid)
        })
    }

    /// The entry to the restricted view shows while there are locked chats
    /// and they are hidden.
    fn show_restricted_entry(&self) {
        let any_locked = (0..self.chats.n_items()).any(|i| {
            self.chat_at(i)
                .is_some_and(|c| c.borrow::<ipc::Chat>().locked)
        });
        self.restricted_entry
            .set_visible(any_locked && !self.restricted.get());
    }

    fn show_preferences(self: &Rc<Self>, settings: ipc::Settings) {
        let hide_content = adw::SwitchRow::builder()
            .title(gettext("Hide Message Content"))
            .subtitle(gettext("Notifications say who wrote, but not what"))
            .active(settings.hide_notification_content)
            .build();
        let notifications = adw::PreferencesGroup::builder()
            .title(gettext("Notifications"))
            .build();
        notifications.add(&hide_content);
        let tray_icon = adw::SwitchRow::builder()
            .title(gettext("Status Icon"))
            .subtitle(gettext(
                "Shows the unread count in the top bar, on desktops that support status icons",
            ))
            .active(settings.tray_icon)
            .build();
        notifications.add(&tray_icon);
        let link_previews = adw::SwitchRow::builder()
            .title(gettext("Link Previews"))
            .subtitle(gettext(
                "Links you send show the page's title and image. Wassup visits the page to get them, as your phone does.",
            ))
            .active(settings.link_previews)
            .build();
        let messages_group = adw::PreferencesGroup::builder()
            .title(gettext("Messages"))
            .build();
        messages_group.add(&link_previews);
        let cache = adw::SpinRow::builder()
            .title(gettext("Media Cache Size"))
            .subtitle(gettext("Downloaded photos, stickers, documents and audio, in MB. The oldest are deleted first."))
            .adjustment(&gtk::Adjustment::new(
                settings.media_cache_mb as f64,
                50.0,
                20_000.0,
                50.0,
                500.0,
                0.0,
            ))
            .build();
        let storage = adw::PreferencesGroup::builder()
            .title(gettext("Storage"))
            .build();
        storage.add(&cache);
        let gpu = adw::SwitchRow::builder()
            .title(gettext("GPU Rendering"))
            .subtitle(gettext("Smoother animations, but uses about 50 MB more memory. Applies after restarting Wassup."))
            .active(settings.gpu_rendering)
            .build();
        let performance = adw::PreferencesGroup::builder()
            .title(gettext("Performance"))
            .build();
        performance.add(&gpu);
        // Names in their own language, so anyone can find theirs.
        let languages = [
            ("", gettext("System Language")),
            ("en", "English".to_owned()),
            ("es", "Español".to_owned()),
        ];
        let language = adw::ComboRow::builder()
            .title(gettext("Language"))
            .subtitle(gettext("Applies after restarting Wassup"))
            .model(&gtk::StringList::new(
                &languages
                    .iter()
                    .map(|(_, name)| name.as_str())
                    .collect::<Vec<_>>(),
            ))
            .selected(
                languages
                    .iter()
                    .position(|(code, _)| *code == settings.language)
                    .unwrap_or(0) as u32,
            )
            .build();
        let general = adw::PreferencesGroup::new();
        general.add(&language);
        let page = adw::PreferencesPage::new();
        page.add(&general);
        page.add(&notifications);
        page.add(&messages_group);
        page.add(&storage);
        page.add(&performance);
        let dialog = adw::PreferencesDialog::new();
        dialog.add(&page);

        // Every change saves all of them; the daemon writes the file.
        let save = Rc::new(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            #[weak]
            hide_content,
            #[weak]
            cache,
            #[weak]
            gpu,
            #[weak]
            language,
            #[weak]
            tray_icon,
            #[weak]
            link_previews,
            move || {
                ui.send(request::Kind::SetSettings(ipc::SetSettings {
                    settings: Some(ipc::Settings {
                        hide_notification_content: hide_content.is_active(),
                        media_cache_mb: cache.value() as u32,
                        gpu_rendering: gpu.is_active(),
                        language: languages[language.selected() as usize].0.to_owned(),
                        tray_icon: tray_icon.is_active(),
                        link_previews: link_previews.is_active(),
                    }),
                }));
            }
        ));
        for row in [&hide_content, &gpu, &tray_icon, &link_previews] {
            let save = save.clone();
            row.connect_active_notify(move |_| save());
        }
        cache.connect_value_notify({
            let save = save.clone();
            move |_| save()
        });
        language.connect_selected_notify(move |_| save());
        dialog.present(Some(&self.window));
    }

    /// Quitting also stops notifications, unlike closing the window, so
    /// it asks first.
    fn confirm_quit(self: &Rc<Self>, app: &adw::Application) {
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Quit Wassup?"))
            .body(gettext(
                "You won't get notifications of new messages until you open Wassup again. To keep getting them, close the window instead.",
            ))
            .default_response("quit")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[("cancel", &gettext("Cancel")), ("quit", &gettext("Quit"))]);
        dialog.set_response_appearance("quit", adw::ResponseAppearance::Destructive);
        dialog.connect_response(
            Some("quit"),
            glib::clone!(
                #[weak(rename_to = ui)]
                self,
                #[weak]
                app,
                move |_, _| {
                    ui.send(request::Kind::Quit(ipc::Quit {}));
                    app.quit();
                }
            ),
        );
        dialog.present(Some(&self.window));
    }

    fn show_shortcuts(&self) {
        let section = |title: Option<String>, items: &[(String, &str)]| {
            let section = adw::ShortcutsSection::new(title.as_deref());
            for (title, accel) in items {
                section.add(adw::ShortcutsItem::new(title, accel));
            }
            section
        };
        let dialog = adw::ShortcutsDialog::new();
        dialog.add(section(
            Some(gettext("Chats")),
            &[
                (gettext("Search chats"), "<Control>f"),
                (gettext("Search messages"), "<Control><Shift>f"),
                (gettext("Open the next or previous chat"), "Up Down"),
                (gettext("Write in the open chat"), "Return"),
                (gettext("Back to the chat list"), "Escape"),
            ],
        ));
        dialog.add(section(
            Some(gettext("Writing")),
            &[
                (gettext("Send the message"), "Return"),
                (gettext("New line"), "<Shift>Return"),
            ],
        ));
        dialog.add(section(
            Some(gettext("General")),
            &[
                (gettext("Preferences"), "<Control>comma"),
                (gettext("Keyboard shortcuts"), "<Control>question"),
                (gettext("Close the window"), "<Control>w"),
                (gettext("Quit and stop notifications"), "<Control>q"),
            ],
        ));
        dialog.present(Some(&self.window));
    }

    fn show_about(&self) {
        let about = adw::AboutDialog::builder()
            .application_name("Wassup")
            .application_icon("io.github.MawCeron.Wassup")
            .developer_name("Mauricio Cerón Medina")
            .version(env!("CARGO_PKG_VERSION"))
            .website("https://github.com/MawCeron/Wassup")
            .issue_url("https://github.com/MawCeron/Wassup/issues")
            .license_type(gtk::License::Gpl30)
            .copyright("© 2026 Mauricio Cerón Medina")
            .comments(gettext(
                "An unofficial WhatsApp client for GNOME. Wassup is not affiliated with WhatsApp or Meta.",
            ))
            // TRANSLATORS: Put your name here, as it should be credited.
            .translator_credits(gettext("translator-credits"))
            .build();
        about.present(Some(&self.window));
    }

    /// Reads a message that arrived in the open chat to screen reader users.
    fn announce(&self, msg: &ipc::Message) {
        let what = summary(msg);
        if what.is_empty() {
            return;
        }
        let text = if msg.sender_name.is_empty() {
            what
        } else {
            format!("{}: {what}", msg.sender_name)
        };
        // libadwaita's binding does not list Accessible for its windows.
        self.window
            .upcast_ref::<gtk::Widget>()
            .announce(&text, gtk::AccessibleAnnouncementPriority::Medium);
    }

    fn ask_secret_code(self: &Rc<Self>) {
        let code = gtk::PasswordEntry::builder()
            .show_peek_icon(true)
            .placeholder_text(gettext("Secret code"))
            .activates_default(true)
            .build();
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Enter Secret Code"))
            .body(gettext(
                "Enter the secret code you set on your phone to see your restricted chats.",
            ))
            .extra_child(&code)
            .default_response("continue")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[
            ("cancel", &gettext("Cancel")),
            ("continue", &gettext("Unlock")),
        ]);
        dialog.set_response_appearance("continue", adw::ResponseAppearance::Suggested);
        dialog.set_response_enabled("continue", false);
        code.connect_changed(glib::clone!(
            #[weak]
            dialog,
            move |code| dialog.set_response_enabled("continue", !code.text().is_empty())
        ));
        dialog.connect_response(
            Some("continue"),
            glib::clone!(
                #[weak(rename_to = ui)]
                self,
                #[weak]
                code,
                move |_, _| {
                    let Some(account) = ui.account.borrow().as_ref().map(|a| a.id.clone()) else {
                        return;
                    };
                    let request = request::Kind::UnlockChats(ipc::UnlockChats {
                        account_id: account,
                        secret_code: code.text().to_string(),
                    });
                    if let Some(id) = ui.send(request) {
                        ui.unlock_request.set(id);
                    }
                }
            ),
        );
        dialog.present(Some(&self.window));
    }

    fn enter_restricted(&self) {
        self.restricted.set(true);
        self.chat_filter.changed(gtk::FilterChange::Different);
        self.restricted_back.set_visible(true);
        self.show_restricted_entry();
        self.sidebar_title.set_title(&gettext("Restricted Chats"));
        self.reselect_current();
        self.toast(&gettext("Restricted chats unlocked"));
    }

    /// Hides the locked chats again, closing one if it is open.
    fn leave_restricted(&self) {
        self.restricted.set(false);
        self.chat_filter.changed(gtk::FilterChange::Different);
        self.restricted_back.set_visible(false);
        self.show_restricted_entry();
        let name = self
            .account
            .borrow()
            .as_ref()
            .map(|a| a.push_name.clone())
            .unwrap_or_default();
        self.sidebar_title.set_title(&if name.is_empty() {
            gettext("Chats")
        } else {
            name
        });
        if self.open_chat_is_locked() {
            self.close_chat();
        }
        self.reselect_current();
        self.toast(&gettext("Restricted chats locked"));
    }

    fn open_chat_is_locked(&self) -> bool {
        let current = self.current_chat.borrow().clone();
        current
            .and_then(|jid| self.position_of(&jid))
            .and_then(|pos| self.chat_at(pos))
            .is_some_and(|c| c.borrow::<ipc::Chat>().locked)
    }

    /// Leaves the conversation pane empty, as when no chat is selected.
    fn close_chat(&self) {
        self.close_viewer();
        self.cancel_reply();
        self.search.bar.set_search_mode(false);
        self.stop_audio();
        self.current_chat.replace(None);
        self.messages.remove_all();
        self.content.set_title("Wassup");
        self.content_stack.set_visible_child_name("empty");
        self.content_view.set_reveal_bottom_bars(false);
        if let Some(header) = self.header_title.parent() {
            header.set_visible(false);
        }
        self.chat_selection.set_selected(gtk::INVALID_LIST_POSITION);
        self.split.set_show_content(false);
        self.report_active();
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
        let is_open = self.current_chat.borrow().as_deref() == Some(chat.jid.as_str());
        if is_open && chat.locked && !self.restricted.get() {
            self.close_chat(); // Locked on the phone while on screen.
        } else if is_open {
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
        self.show_restricted_entry();
    }

    fn reselect_current(&self) {
        let current = self.current_chat.borrow().clone();
        if let Some(pos) = current.and_then(|jid| self.shown_position(&jid)) {
            self.chat_selection.set_selected(pos);
        }
    }

    fn open_chat(&self, chat: &ipc::Chat) {
        self.close_viewer();
        self.cancel_reply();
        self.search.bar.set_search_mode(false);
        if chat.locked && !self.restricted.get() {
            return; // Only reachable through the restricted view.
        }
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
        self.messages.remove_all();
        self.load_messages(String::new());
        if chat.unread_count > 0 {
            self.mark_read();
        }
    }

    fn choose_file(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Send a File"))
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
                    ui.preview_file(&path.to_string_lossy());
                }
            },
        );
    }

    /// Sends a file with a caption. Returns whether the request went out.
    fn send_file(&self, path: &str, caption: &str) -> bool {
        let Some((account_id, chat_jid)) = self.open_ids() else {
            return false;
        };
        let caption = caption.to_owned();
        let request = request::Kind::SendMedia(ipc::SendMedia {
            account_id,
            chat_jid,
            path: path.to_owned(),
            caption: caption.clone(),
            as_sticker: false,
        });
        let Some(id) = self.send(request) else {
            return false;
        };
        self.sending.borrow_mut().insert(id, caption);
        let name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let toast = adw::Toast::builder()
            // TRANSLATORS: {} is a file name.
            .title(gettext("Sending {}…").replace("{}", &name))
            .timeout(0)
            .build();
        self.toasts.add_toast(toast.clone());
        self.uploads.borrow_mut().insert(id, toast);
        true
    }

    /// Opens a message's file with the desktop's default application,
    /// downloading it first if needed.
    fn open_media(&self, msg: &ipc::Message) {
        let Some(media) = &msg.media else { return };
        if matches!(media.kind(), ipc::MediaKind::Image | ipc::MediaKind::Video) {
            return self.show_in_viewer(msg);
        }
        if media.path.is_empty() {
            self.open_when_ready.borrow_mut().insert(msg.id.clone());
            self.request_media(msg);
        } else {
            self.launch(&media.path);
        }
    }

    /// Opens a web address with the default browser.
    pub(super) fn open_link(&self, url: &str) {
        gtk::UriLauncher::new(url).launch(
            Some(&self.window),
            None::<&gio::Cancellable>,
            |result| {
                if let Err(e) = result {
                    eprintln!("wassup: couldn't open link: {e}");
                }
            },
        );
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
            reply_to_id: self.reply.target.borrow().clone().unwrap_or_default(),
        });
        if let Some(id) = self.send(request) {
            buffer.set_text("");
            self.cancel_reply();
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
            ..Default::default()
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
        // The message that was first may share its day with the page above
        // it: bind it again so it drops its day separator.
        if !first_page && !items.is_empty() {
            self.messages.items_changed(items.len() as u32, 1, 1);
        }
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

    fn media_ready(self: &Rc<Self>, ready: ipc::MediaReady) {
        self.media_requested.borrow_mut().remove(&ready.message_id);
        let waiting = self.sticker_pictures.borrow_mut().remove(&ready.message_id);
        if let Some(picture) = waiting.and_then(|p| p.upgrade())
            && ready.error.is_empty()
        {
            show_thumb(&picture, &ready.path);
        }
        self.photo_ready(&ready);
        let open = self.open_when_ready.borrow_mut().remove(&ready.message_id);
        if !ready.error.is_empty() {
            eprintln!("wassup: couldn't download media: {}", ready.error);
            if open {
                self.toast(&gettext("Couldn't download the file"));
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
                ipc::MediaKind::Document | ipc::MediaKind::Audio | ipc::MediaKind::Video
            ) {
                break; // Nothing to draw: a video's row keeps its thumbnail.
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

    /// Drops disappearing messages whose time ran out from the open chat.
    fn remove_expired(&self, gone: ipc::MessagesExpired) {
        let open = self.is_current(&gone.account_id)
            && self.current_chat.borrow().as_deref() == Some(gone.chat_jid.as_str());
        if !open {
            return;
        }
        let playing = self.player.borrow().as_ref().map(|p| p.msg_id.clone());
        if playing.is_some_and(|id| gone.message_ids.contains(&id)) {
            self.stop_audio();
        }
        // From the end, so earlier positions stay valid while removing.
        for pos in (0..self.messages.n_items()).rev() {
            let hit = self
                .messages
                .item(pos)
                .and_downcast::<glib::BoxedAnyObject>()
                .is_some_and(|item| gone.message_ids.contains(&item.borrow::<ipc::Message>().id));
            if hit {
                self.messages.remove(pos);
            }
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
                let icon = self
                    .rows
                    .borrow()
                    .get(&msg.id)
                    .and_then(|r| r.status.upgrade());
                if let Some(icon) = icon {
                    show_status(&icon, change.status());
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
        self.qr_stack.set_visible_child_name("waiting");
        self.link_status.set_label(&gettext("Preparing a code…"));
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

/// Text folded for searching: lowercase and without accents, so "noemi"
/// finds "Noemí".
fn search_key(text: &str) -> String {
    glib::normalize(text.to_lowercase(), glib::NormalizeMode::All)
        .chars()
        // Combining diacritical marks, left apart by the decomposition.
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .collect()
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
        Some((_, "lid")) => gettext("Unknown contact"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ignores_case_and_accents() {
        assert_eq!(search_key("Noemí Cerón"), "noemi ceron");
        assert!(search_key("Familia MEDINA").contains(&search_key("medi")));
        assert_eq!(search_key("Ñandú"), "nandu");
    }

    #[test]
    fn hidden_ids_are_not_shown_as_numbers() {
        assert_eq!(user_label("5215550000000@s.whatsapp.net"), "+5215550000000");
        assert_eq!(user_label("99128348550@lid"), "Unknown contact");
    }
}
