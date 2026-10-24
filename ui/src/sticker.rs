//! Turning an image into a WhatsApp sticker: 512x512 WebP under 100 KB.

use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk::{gio, glib, prelude::*};

/// WhatsApp's sticker size and its limit for static stickers.
const SIZE: i32 = 512;
const MAX_BYTES: usize = 100 * 1024;
/// Tried in order until the result fits; photos need the lower ones.
const QUALITIES: [f32; 5] = [80.0, 65.0, 50.0, 35.0, 20.0];

/// Lossy WebP encoding through the system's libwebp.
fn encode(rgba: &[u8], width: i32, height: i32, stride: i32, quality: f32) -> Option<Vec<u8>> {
    if rgba.len() < (stride * height) as usize {
        return None;
    }
    let mut out = std::ptr::null_mut();
    // SAFETY: rgba holds height rows of stride bytes, checked above; libwebp
    // allocates the output, which is copied and then freed with WebPFree.
    unsafe {
        let len =
            libwebp_sys::WebPEncodeRGBA(rgba.as_ptr(), width, height, stride, quality, &mut out);
        if len == 0 || out.is_null() {
            return None;
        }
        let webp = std::slice::from_raw_parts(out, len).to_vec();
        libwebp_sys::WebPFree(out.cast());
        Some(webp)
    }
}

/// Centres an image on a transparent SIZE x SIZE canvas and encodes it,
/// lowering the quality until it fits WhatsApp's limit.
pub fn from_pixbuf(image: &Pixbuf) -> Result<Vec<u8>, String> {
    let image = image.add_alpha(false, 0, 0, 0).map_err(|e| e.to_string())?;
    let canvas = Pixbuf::new(Colorspace::Rgb, true, 8, SIZE, SIZE).ok_or("out of memory")?;
    canvas.fill(0); // Transparent.
    let (w, h) = (image.width().min(SIZE), image.height().min(SIZE));
    image.copy_area(0, 0, w, h, &canvas, (SIZE - w) / 2, (SIZE - h) / 2);
    let pixels = canvas.read_pixel_bytes();
    QUALITIES
        .iter()
        .filter_map(|&q| encode(&pixels, SIZE, SIZE, canvas.rowstride(), q))
        .find(|webp| webp.len() <= MAX_BYTES)
        .ok_or_else(|| "the image is too detailed to fit in a sticker".to_owned())
}

/// Reads an image file scaled down to fit a sticker. Decoding runs in
/// glycin's sandboxed loaders.
pub async fn from_file(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let stream = gio::File::for_path(path)
        .read_future(glib::Priority::DEFAULT)
        .await
        .map_err(|e| e.to_string())?;
    let image = Pixbuf::from_stream_at_scale_future(&stream, SIZE, SIZE, true)
        .await
        .map_err(|e| e.to_string())?;
    from_pixbuf(&image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_a_square_webp_within_the_limit() {
        // A noisy image is the hard case for size.
        let image = Pixbuf::new(Colorspace::Rgb, false, 8, 300, 200).unwrap();
        let mut seed = 1u32;
        let bytes: Vec<u8> = (0..image.rowstride() * 200)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (seed >> 16) as u8
            })
            .collect();
        let image = Pixbuf::from_bytes(
            &glib::Bytes::from_owned(bytes),
            Colorspace::Rgb,
            false,
            8,
            300,
            200,
            image.rowstride(),
        );
        let webp = from_pixbuf(&image).unwrap();
        assert!(webp.len() <= MAX_BYTES, "{} bytes", webp.len());
        assert_eq!(&webp[0..4], b"RIFF");
        assert_eq!(&webp[8..12], b"WEBP");
    }
}
