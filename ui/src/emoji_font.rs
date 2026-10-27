//! Draws every emoji with Twemoji when it is installed.
//!
//! Cairo keeps the color glyphs of Noto Color Emoji in a cache that grows
//! to hundreds of megabytes and is never freed, while Fedora's Twemoji, a
//! bitmap font, costs almost nothing. Pango picks the emoji font by itself, so
//! the only way to switch it for the chat and the emoji picker alike is to
//! take both Noto emoji fonts out of the process's fontconfig, since without
//! the color one Pango falls back to the monochrome one. Symbola, a
//! monochrome symbol font that many systems have, goes too: it draws the
//! emoji GTK measures to decide how wide the others may be, which makes the
//! picker drop the wider ones, such as flags. See docs/performance.md.

use std::ffi::{CStr, c_char, c_int, c_void};

const FAMILY: &CStr = c"Twemoji";
const REJECT_FONTS: &CStr = c"<?xml version=\"1.0\"?><fontconfig><selectfont><rejectfont>\
    <pattern><patelt name=\"family\"><string>Noto Color Emoji</string></patelt></pattern>\
    <pattern><patelt name=\"family\"><string>Noto Emoji</string></patelt></pattern>\
    <pattern><patelt name=\"family\"><string>Symbola</string></patelt></pattern>\
    </rejectfont></selectfont></fontconfig>";

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcConfigGetCurrent() -> *mut c_void;
    fn FcConfigParseAndLoadFromMemory(config: *mut c_void, s: *const u8, complain: c_int) -> c_int;
    fn FcConfigBuildFonts(config: *mut c_void) -> c_int;
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

/// Whether the best match for the family Twemoji is Twemoji itself.
///
/// # Safety
/// `config` must be the current fontconfig configuration.
unsafe fn twemoji_installed(config: *mut c_void) -> bool {
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

/// Hides the other emoji fonts from this process when Twemoji can replace them.
/// Call before anything uses Pango.
pub fn prefer_twemoji() {
    // SAFETY: called from main before GTK starts, so no other thread uses fontconfig.
    unsafe {
        let config = FcConfigGetCurrent();
        if config.is_null() || !twemoji_installed(config) {
            return;
        }
        if FcConfigParseAndLoadFromMemory(config, REJECT_FONTS.as_ptr().cast(), 0) != 0 {
            FcConfigBuildFonts(config);
        }
    }
}
