//! Draws every emoji with the Twemoji font that Wassup carries.
//!
//! Cairo keeps the color glyphs of Noto Color Emoji in a cache that grows
//! to hundreds of megabytes and is never freed, while a bitmap font such as
//! Twemoji CBDT costs almost nothing. Pango picks the emoji font by itself,
//! so the only way to switch it for the chat and the emoji picker alike is
//! to take the other emoji fonts out of the process's fontconfig: both Noto
//! ones, since without the color one Pango falls back to the monochrome one,
//! and other Twemoji fonts, so that it cannot choose them. Symbola, a
//! monochrome symbol font that many systems have, goes too: it draws the
//! emoji GTK measures to decide how wide the others may be, which makes the
//! picker drop the wider ones, such as flags. The font is private to this
//! process: it is added to fontconfig here, not installed on the system.
//! See docs/performance.md.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

const FAMILY: &CStr = c"Twemoji CBDT";
/// Where the font is, under the data directories (`/usr/share`, `~/.local/share`).
const FONT: &str = "wassup/fonts/TwemojiCBDT.ttf";
const REJECT_FONTS: &CStr = c"<?xml version=\"1.0\"?><fontconfig><selectfont><rejectfont>\
    <pattern><patelt name=\"family\"><string>Noto Color Emoji</string></patelt></pattern>\
    <pattern><patelt name=\"family\"><string>Noto Emoji</string></patelt></pattern>\
    <pattern><patelt name=\"family\"><string>Symbola</string></patelt></pattern>\
    <pattern><patelt name=\"family\"><string>Twemoji</string></patelt></pattern>\
    <pattern><patelt name=\"family\"><string>Twemoji Mozilla</string></patelt></pattern>\
    </rejectfont></selectfont></fontconfig>";

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcConfigGetCurrent() -> *mut c_void;
    fn FcConfigParseAndLoadFromMemory(config: *mut c_void, s: *const u8, complain: c_int) -> c_int;
    fn FcConfigBuildFonts(config: *mut c_void) -> c_int;
    fn FcConfigAppFontAddFile(config: *mut c_void, file: *const u8) -> c_int;
    fn FcConfigSubstitute(config: *mut c_void, pattern: *mut c_void, kind: c_int) -> c_int;
    fn FcDefaultSubstitute(pattern: *mut c_void);
    fn FcPatternCreate() -> *mut c_void;
    fn FcPatternDestroy(pattern: *mut c_void);
    fn FcPatternAddString(pattern: *mut c_void, object: *const c_char, s: *const u8) -> c_int;
    fn FcPatternGetString(
        pattern: *mut c_void,
        object: *const c_char,
        n: c_int,
        s: *mut *const u8,
    ) -> c_int;
    fn FcFontMatch(config: *mut c_void, pattern: *mut c_void, result: *mut c_int) -> *mut c_void;
}

/// Whether the best match for the family Twemoji CBDT is Twemoji CBDT itself.
///
/// # Safety
/// `config` must be the current fontconfig configuration.
unsafe fn font_loaded(config: *mut c_void) -> bool {
    const FC_FAMILY: &CStr = c"family";
    const FC_MATCH_PATTERN: c_int = 0;
    // SAFETY: the pattern is created, used and destroyed here, and fontconfig
    // owns the strings it returns until the patterns that hold them are destroyed.
    unsafe {
        let pattern = FcPatternCreate();
        FcPatternAddString(pattern, FC_FAMILY.as_ptr(), FAMILY.as_ptr().cast());
        FcConfigSubstitute(config, pattern, FC_MATCH_PATTERN);
        FcDefaultSubstitute(pattern);
        let mut result = 0;
        let font = FcFontMatch(config, pattern, &mut result);
        let mut found = std::ptr::null();
        let installed = !font.is_null()
            && FcPatternGetString(font, FC_FAMILY.as_ptr(), 0, &mut found) == 0
            && !found.is_null()
            && CStr::from_ptr(found.cast()) == FAMILY;
        if !font.is_null() {
            FcPatternDestroy(font);
        }
        FcPatternDestroy(pattern);
        installed
    }
}

/// The font file, looking in the user's data directory before the system's,
/// so a copy there can replace the one the package installs.
fn bundled_font() -> Option<PathBuf> {
    std::iter::once(glib::user_data_dir())
        .chain(glib::system_data_dirs())
        .map(|dir| dir.join(FONT))
        .find(|path| path.is_file())
}

/// Loads the bundled emoji font into this process and hides the other emoji
/// fonts. Does nothing if the font is missing. Call before anything uses Pango.
pub fn use_bundled_font() {
    let Some(font) = bundled_font() else { return };
    let Ok(path) = CString::new(font.as_os_str().as_bytes()) else {
        return;
    };
    // SAFETY: called from main before GTK starts, so no other thread uses fontconfig.
    unsafe {
        let config = FcConfigGetCurrent();
        if config.is_null()
            || FcConfigAppFontAddFile(config, path.as_ptr().cast()) == 0
            || !font_loaded(config)
        {
            return;
        }
        if FcConfigParseAndLoadFromMemory(config, REJECT_FONTS.as_ptr().cast(), 0) != 0 {
            FcConfigBuildFonts(config);
        }
    }
}
