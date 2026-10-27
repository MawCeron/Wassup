//! GIFs: the videos WhatsApp shows as short, silent, looping animations.
//!
//! A GIF is an MP4 file, played through GTK's media support, which uses
//! GStreamer. A playing one holds a pipeline, so it plays only while its row
//! shows it on screen, and lets go of it as soon as it does not.

use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use super::*;

/// How often a playing GIF checks that its row still wants it.
const CHECK: Duration = Duration::from_millis(500);

thread_local! {
    /// The GIF each picture is playing, to stop the one it replaces.
    static PLAYING: RefCell<HashMap<usize, Rc<()>>> = RefCell::default();
}

/// Plays the GIF in `path` in a picture for as long as `wanted` says it should.
/// Until its first frame is ready the picture keeps what it shows, the
/// thumbnail. Playing in a picture again stops the earlier playback.
pub(super) fn play(
    picture: &gtk::Picture,
    path: &str,
    wanted: impl Fn(&gtk::Picture) -> bool + 'static,
) {
    let token = Rc::new(());
    PLAYING.with(|p| {
        p.borrow_mut()
            .insert(picture.as_ptr() as usize, token.clone())
    });
    let media = gtk::MediaFile::for_filename(path);
    media.set_loop(true);
    media.set_muted(true);
    media.connect_prepared_notify(glib::clone!(
        #[weak]
        picture,
        move |media| {
            if media.is_prepared() {
                picture.set_paintable(Some(media));
                media.play();
            }
        }
    ));
    let token = Rc::downgrade(&token);
    let picture = picture.downgrade();
    glib::timeout_add_local(CHECK, move || check(&media, &token, &picture, &wanted));
}

/// Keeps a GIF playing while it is wanted and visible, and ends it otherwise.
fn check(
    media: &gtk::MediaFile,
    token: &Weak<()>,
    picture: &glib::WeakRef<gtk::Picture>,
    wanted: &impl Fn(&gtk::Picture) -> bool,
) -> glib::ControlFlow {
    let picture = picture
        .upgrade()
        .filter(|p| token.upgrade().is_some() && wanted(p));
    let Some(picture) = picture else {
        media.pause();
        media.clear();
        return glib::ControlFlow::Break;
    };
    if media.is_prepared() {
        if picture.is_mapped() && !media.is_playing() {
            media.play();
        } else if !picture.is_mapped() && media.is_playing() {
            media.pause();
        }
    }
    glib::ControlFlow::Continue
}
