use std::path::Path;

use crate::formats::pdf::error::RenderError;

#[derive(Debug, Clone)]
pub struct GlyphInfo {
    pub glyph_id: u32,
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

#[cfg(feature = "pdf")]
pub fn shape_text(
    font_data: &[u8],
    text: &str,
    script: &str,
    lang: &str,
) -> Result<Vec<GlyphInfo>, RenderError> {
    use rustybuzz::{Direction, Face, Language, Script, UnicodeBuffer, shape};

    let face = Face::from_slice(font_data, 0).ok_or_else(|| RenderError::Font {
        message: "invalid font data".to_string(),
    })?;
    let mut buf = UnicodeBuffer::new();
    buf.push_str(text);

    // rustybuzz uses ttf_parser's Tag-based Script constants.
    // Construct them via Script::from_str or use the tag! macro / Tag directly.
    let script = match script {
        "Deva" => Script::from_str("Deva").unwrap_or_default(),
        "Latn" => Script::from_str("Latn").unwrap_or_default(),
        _ => Script::default(),
    };

    buf.set_script(script);
    // Language::from_str is the correct method name
    if let Some(language) = Language::from_str(lang) {
        buf.set_language(language);
    }
    buf.set_direction(Direction::LeftToRight);

    let shaped = shape(&face, &[], buf);

    let glyphs = shaped
        .glyph_infos()
        .iter()
        .zip(shaped.glyph_positions())
        .map(|(info, pos)| GlyphInfo {
            glyph_id: info.glyph_id,
            x_advance: pos.x_advance,
            y_advance: pos.y_advance,
            x_offset: pos.x_offset,
            y_offset: pos.y_offset,
        })
        .collect();

    Ok(glyphs)
}

#[cfg(feature = "pdf")]
pub fn shape_text(
    font_data: &[u8],
    text: &str,
    script: &str,
    lang: &str,
) -> Result<Vec<GlyphInfo>, RenderError> {
    use rustybuzz::{Direction, Face, Language, Script, Tag, UnicodeBuffer, shape};

    let face = Face::from_slice(font_data, 0).ok_or_else(|| RenderError::Font {
        message: "invalid font data".to_string(),
    })?;
    let mut buf = UnicodeBuffer::new();
    buf.push_str(text);

    // Tag::from_bytes_lossy pads/truncates to exactly 4 bytes
    let tag = Tag::from_bytes_lossy(script.as_bytes());
    // from_iso15924_tag returns Option; fall back to Latin if unrecognised
    let script = Script::from_iso15924_tag(tag)
        .unwrap_or_else(|| Script::from_iso15924_tag(Tag::from_bytes_lossy(b"Latn")).unwrap());

    buf.set_script(script);
    // Language::from(&str) is the correct constructor — returns Language directly
    buf.set_language(Language::from(lang));
    buf.set_direction(Direction::LeftToRight);

    let shaped = shape(&face, &[], buf);

    let glyphs = shaped
        .glyph_infos()
        .iter()
        .zip(shaped.glyph_positions())
        .map(|(info, pos)| GlyphInfo {
            glyph_id: info.glyph_id,
            x_advance: pos.x_advance,
            y_advance: pos.y_advance,
            x_offset: pos.x_offset,
            y_offset: pos.y_offset,
        })
        .collect();

    Ok(glyphs)
}

#[cfg(feature = "pdf")]
pub fn render_glyphs(
    font_path: &Path,
    glyphs: &[GlyphInfo],
    font_size_px: u32,
    upem: i32,
    width: u32,
    height: u32,
) -> Result<image::RgbaImage, RenderError> {
    use freetype::Library; // re-exported at crate root in v0.7.x
    use freetype::face::LoadFlag; // face module is pub in v0.7.x
    use image::Rgba;

    let lib = Library::init().map_err(|err| RenderError::Font {
        message: err.to_string(),
    })?;
    let face = lib
        .new_face(font_path, 0)
        .map_err(|err| RenderError::Font {
            message: err.to_string(),
        })?;
    face.set_pixel_sizes(0, font_size_px)
        .map_err(|err| RenderError::Font {
            message: err.to_string(),
        })?;

    let mut img = image::RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 0]));
    let baseline = (height as f32 * 0.80) as i32;
    let mut x_pen: i32 = 0;

    for glyph in glyphs {
        face.load_glyph(glyph.glyph_id, LoadFlag::RENDER)
            .map_err(|err| RenderError::Font {
                message: err.to_string(),
            })?;
        let bitmap = face.glyph().bitmap();
        let bx = face.glyph().bitmap_left();
        let by = face.glyph().bitmap_top();

        let x_offset = glyph.x_offset * font_size_px as i32 / upem;
        let y_offset = glyph.y_offset * font_size_px as i32 / upem;
        let x_advance = glyph.x_advance * font_size_px as i32 / upem;

        for row in 0..bitmap.rows() {
            for col in 0..bitmap.width() {
                let idx = (row * bitmap.pitch() + col) as usize;
                let val = bitmap.buffer()[idx];
                if val > 0 {
                    let px = x_pen + x_offset + bx + col as i32;
                    let py = baseline - y_offset - by + row as i32;
                    if px >= 0 && py >= 0 {
                        let (px, py) = (px as u32, py as u32);
                        if px < width && py < height {
                            let existing = img.get_pixel(px, py);
                            let alpha = existing[3].saturating_add(val);
                            img.put_pixel(px, py, Rgba([0, 0, 0, alpha]));
                        }
                    }
                }
            }
        }
        x_pen += x_advance;
    }

    Ok(img)
}

#[cfg(feature = "pdf")]
pub fn render_glyphs(
    font_path: &Path,
    glyphs: &[GlyphInfo],
    font_size_px: u32,
    upem: i32,
    width: u32,
    height: u32,
) -> Result<image::RgbaImage, RenderError> {
    // freetype crate re-exports Library at the crate root
    use freetype::Library;
    use image::Rgba;

    let lib = Library::init().map_err(|err: freetype::Error| RenderError::Font {
        message: err.to_string(),
    })?;
    let face = lib
        .new_face(font_path, 0)
        .map_err(|err: freetype::Error| RenderError::Font {
            message: err.to_string(),
        })?;
    face.set_pixel_sizes(0, font_size_px)
        .map_err(|err: freetype::Error| RenderError::Font {
            message: err.to_string(),
        })?;

    let mut img = image::RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 0]));

    let baseline = (height as f32 * 0.80) as i32;
    let mut x_pen: i32 = 0;

    for glyph in glyphs {
        // LoadFlag is at freetype::face::LoadFlag
        face.load_glyph(glyph.glyph_id, freetype::face::LoadFlag::RENDER)
            .map_err(|err: freetype::Error| RenderError::Font {
                message: err.to_string(),
            })?;
        let bitmap = face.glyph().bitmap();
        let bx = face.glyph().bitmap_left();
        let by = face.glyph().bitmap_top();

        let x_offset = glyph.x_offset * font_size_px as i32 / upem;
        let y_offset = glyph.y_offset * font_size_px as i32 / upem;
        let x_advance = glyph.x_advance * font_size_px as i32 / upem;

        for row in 0..bitmap.rows() {
            for col in 0..bitmap.width() {
                let idx = (row * bitmap.pitch() + col) as usize;
                let val = bitmap.buffer()[idx];
                if val > 0 {
                    let px = (x_pen + x_offset + bx + col as i32) as i32;
                    let py = (baseline - y_offset - by + row as i32) as i32;
                    if px >= 0 && py >= 0 {
                        let (px, py) = (px as u32, py as u32);
                        if px < width && py < height {
                            let existing = img.get_pixel(px, py);
                            let alpha = existing[3].saturating_add(val);
                            img.put_pixel(px, py, Rgba([0, 0, 0, alpha]));
                        }
                    }
                }
            }
        }
        x_pen += x_advance;
    }

    Ok(img)
}

#[cfg(not(feature = "pdf"))]
pub fn render_glyphs(
    _font_path: &Path,
    _glyphs: &[GlyphInfo],
    _font_size_px: u32,
    _upem: i32,
    _width: u32,
    _height: u32,
) -> Result<Vec<u8>, RenderError> {
    Err(RenderError::FeatureDisabled)
}

#[cfg(feature = "pdf")]
pub fn to_jpeg(img: image::RgbaImage, quality: u8) -> Result<Vec<u8>, RenderError> {
    let rgb = image::DynamicImage::ImageRgba8(img).into_rgb8();
    let mut buf = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
    encoder
        .encode_image(&image::DynamicImage::ImageRgb8(rgb))
        .map_err(|err| RenderError::Image {
            message: err.to_string(),
        })?;
    Ok(buf)
}

#[cfg(not(feature = "pdf"))]
pub fn to_jpeg(_img: Vec<u8>, _quality: u8) -> Result<Vec<u8>, RenderError> {
    Err(RenderError::FeatureDisabled)
}
