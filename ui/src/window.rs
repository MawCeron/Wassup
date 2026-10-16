//! The main window and its reaction to daemon events.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

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

        let stack = gtk::Stack::new();
        stack.add_named(&with_header(&status), Some("status"));
        stack.add_named(&with_header(&link), Some("link"));

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
            link,
            qr,
            link_retry,
            linking: Cell::new(false),
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
            Some(event::Kind::AccountChanged(account)) if self.is_current(&account.id) => {
                self.set_account(account)
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
        self.show_status(&format!("{state:?}"), &name, false);
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
