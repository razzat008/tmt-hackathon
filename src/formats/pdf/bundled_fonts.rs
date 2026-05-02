/// Noto Sans Devanagari – Regular weight (Apache 2.0 licence).
/// Embedded so the binary works without any system font installation.
#[cfg(feature = "pdf")]
pub static NOTO_DEVA_REGULAR: &[u8] =
    include_bytes!("../../../assets/fonts/NotoSansDevanagari-Regular.ttf");

/// Noto Sans Devanagari – Bold weight.
#[cfg(feature = "pdf")]
pub static NOTO_DEVA_BOLD: &[u8] =
    include_bytes!("../../../assets/fonts/NotoSansDevanagari-Bold.ttf");

/// Return `true` when a PDF font name string (e.g. `"BAAAAA+Arial-BoldMT"`) indicates bold.
pub fn is_bold_font_name(font_name: Option<&str>) -> bool {
    let Some(name) = font_name else { return false };
    let n = name.split('+').last().unwrap_or(name).to_lowercase();
    ["bold", "heavy", "black", "demi", "semibold"]
        .iter()
        .any(|w| n.contains(w))
}
