//! The main window and its reaction to daemon events.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::ipc::{self, event, request};

pub struct Window {
    stack: gtk::Stack,
    status: adw::StatusPage,
    retry: gtk::Button,
    toasts: adw::ToastOverlay,
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
        let status = adw::StatusPage::builder()
            .child(&retry)
            .build();
        let stack = gtk::Stack::new();
        stack.add_named(&with_header(&status), Some("status"));

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Wassup")
            .default_width(900)
            .default_height(640)
            .content(&toasts)
            .build();

        let ui = Rc::new(Self {
            stack,
            status,
            retry,
            toasts,
            conn: RefCell::new(None),
            account: RefCell::new(None),
        });
        // The button owns the state for as long as the window lives.
        ui.retry.connect_clicked(glib::clone!(
            #[strong]
            ui,
            move |_| ui.connect()
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
            Some(event::Kind::AccountChanged(account)) => {
                let current = self.account.borrow().as_ref().map(|a| a.id.clone());
                if current.as_deref() == Some(account.id.as_str()) {
                    self.set_account(account);
                }
            }
            _ => {}
        }
    }

    fn set_account(self: &Rc<Self>, account: ipc::Account) {
        let state = account.state();
        let name = account.push_name.clone();
        self.account.replace(Some(account));
        self.show_status(&format!("{state:?}"), &name, false);
    }

    fn show_status(&self, title: &str, description: &str, retry: bool) {
        self.status.set_icon_name(retry.then_some("network-offline-symbolic"));
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
