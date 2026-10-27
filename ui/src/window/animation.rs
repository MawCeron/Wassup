//! Animated stickers, which WhatsApp sends as animated WebP files.
//!
//! The frames are decoded one at a time with libwebp's animation decoder, so
//! a playing sticker costs about two frames of memory (a few megabytes)
//! however long it is, where decoding them all at once would cost one per
//! frame. A sticker plays only while its row shows it.

use std::collections::HashMap;
use std::ffi::c_int;
use std::rc::{Rc, Weak};
use std::time::Duration;

use libwebp_sys as webp;

use super::*;

// libwebp-sys links libwebp, which does not include the animation decoder.
#[link(name = "webpdemux")]
unsafe extern "C" {}

/// Shortest time a frame is shown: browsers do the same with the 0 ms and
/// 10 ms delays some tools write, which would otherwise play too fast.
const MIN_FRAME: Duration = Duration::from_millis(20);
/// How often a sticker that is not on screen checks whether it is back.
const PAUSED_CHECK: Duration = Duration::from_millis(500);

/// Whether the start of a file is a WebP with its animation flag set.
fn is_animated(head: &[u8]) -> bool {
    head.len() > 20
        && &head[..4] == b"RIFF"
        && &head[8..12] == b"WEBP"
        && &head[12..16] == b"VP8X"
        && head[20] & 0x02 != 0
}

/// An animated WebP being played.
struct Animation {
    decoder: *mut webp::WebPAnimDecoder,
    /// The decoder reads from this for as long as it lives.
    _data: Box<[u8]>,
    width: i32,
    height: i32,
    /// End of the last frame, in milliseconds from the start of the loop.
    last_end: c_int,
}

impl Animation {
    /// None if the file is not an animation, or libwebp cannot read it.
    fn open(path: &str) -> Option<Self> {
        let data = std::fs::read(path).ok()?.into_boxed_slice();
        if !is_animated(&data) {
            return None;
        }
        let webp_data = webp::WebPData {
            bytes: data.as_ptr(),
            size: data.len(),
        };
        // SAFETY: the options and the info are plain structs libwebp fills in,
        // and `data`, which the decoder keeps pointing to, is stored with it.
        unsafe {
            let mut options: webp::WebPAnimDecoderOptions = std::mem::zeroed();
            if webp::WebPAnimDecoderOptionsInit(&mut options) == 0 {
                return None;
            }
            options.color_mode = webp::WEBP_CSP_MODE::MODE_RGBA;
            options.use_threads = 0;
            let decoder = webp::WebPAnimDecoderNew(&webp_data, &options);
            if decoder.is_null() {
                return None;
            }
            let mut info: webp::WebPAnimInfo = std::mem::zeroed();
            if webp::WebPAnimDecoderGetInfo(decoder, &mut info) == 0 || info.frame_count < 2 {
                webp::WebPAnimDecoderDelete(decoder);
                return None;
            }
            Some(Self {
                decoder,
                _data: data,
                width: i32::try_from(info.canvas_width).ok()?,
                height: i32::try_from(info.canvas_height).ok()?,
                last_end: 0,
            })
        }
    }

    /// The next frame and how long to show it. After the last frame it
    /// starts over, as the sticker loops.
    fn next_frame(&mut self) -> Option<(gdk::Texture, Duration)> {
        // SAFETY: `decoder` is valid until drop; the buffer GetNext returns
        // holds the whole canvas and stays valid until the next call, and it
        // is copied before then.
        unsafe {
            if webp::WebPAnimDecoderHasMoreFrames(self.decoder) == 0 {
                webp::WebPAnimDecoderReset(self.decoder);
                self.last_end = 0;
            }
            let mut buffer: *mut u8 = std::ptr::null_mut();
            let mut end: c_int = 0;
            if webp::WebPAnimDecoderGetNext(self.decoder, &mut buffer, &mut end) == 0
                || buffer.is_null()
            {
                return None;
            }
            let stride = self.width as usize * 4;
            let pixels = std::slice::from_raw_parts(buffer, stride * self.height as usize);
            let texture = gdk::MemoryTexture::new(
                self.width,
                self.height,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from(pixels),
                stride,
            );
            let shown = Duration::from_millis((end - self.last_end).max(0) as u64);
            self.last_end = end;
            Some((texture.upcast(), shown.max(MIN_FRAME)))
        }
    }
}

impl Drop for Animation {
    fn drop(&mut self) {
        // SAFETY: `decoder` came from WebPAnimDecoderNew and is deleted once.
        unsafe { webp::WebPAnimDecoderDelete(self.decoder) }
    }
}

thread_local! {
    /// The animation each picture is playing, to stop the one it replaces.
    static PLAYING: RefCell<HashMap<usize, Rc<()>>> = RefCell::default();
}

/// Plays the sticker in `path` in a message row's picture, if it is animated,
/// until the row shows another message or goes away.
pub(super) fn play(picture: &gtk::Picture, path: &str, msg_id: &str, rows: &BoundRows) {
    let Some(animation) = Animation::open(path) else {
        return;
    };
    // A row can bind the same message again: only the newest playback runs.
    let token = Rc::new(());
    PLAYING.with(|p| {
        p.borrow_mut()
            .insert(picture.as_ptr() as usize, token.clone())
    });
    tick(
        animation,
        Rc::downgrade(&token),
        picture.downgrade(),
        msg_id.to_owned(),
        rows.clone(),
    );
}

fn tick(
    mut animation: Animation,
    token: Weak<()>,
    picture: glib::WeakRef<gtk::Picture>,
    msg_id: String,
    rows: BoundRows,
) {
    let Some(widget) = picture.upgrade() else {
        return;
    };
    let bound = rows.borrow().get(&msg_id).and_then(|r| r.picture.upgrade());
    if token.upgrade().is_none() || bound.as_ref() != Some(&widget) {
        return;
    }
    let delay = if widget.is_mapped() {
        let Some((texture, shown)) = animation.next_frame() else {
            return;
        };
        widget.set_paintable(Some(&texture));
        shown
    } else {
        PAUSED_CHECK
    };
    glib::timeout_add_local_once(delay, move || tick(animation, token, picture, msg_id, rows));
}

#[cfg(test)]
mod tests {
    use super::is_animated;

    fn webp(chunk: &[u8; 4], flags: u8) -> Vec<u8> {
        let mut head = b"RIFF\0\0\0\0WEBP".to_vec();
        head.extend_from_slice(chunk);
        head.extend_from_slice(&[10, 0, 0, 0, flags, 0, 0, 0, 0, 0]);
        head
    }

    #[test]
    fn only_a_webp_with_the_animation_flag_is_animated() {
        assert!(is_animated(&webp(b"VP8X", 0x12)));
        assert!(!is_animated(&webp(b"VP8X", 0x10)), "a still sticker");
        assert!(!is_animated(&webp(b"VP8 ", 0x12)), "a plain lossy WebP");
        assert!(!is_animated(b"RIFF"), "a truncated file");
        assert!(
            !is_animated(&[0x89, b'P', b'N', b'G'].repeat(6)),
            "another format"
        );
    }
}
