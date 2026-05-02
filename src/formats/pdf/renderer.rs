use crate::formats::pdf::error::RenderError;

#[derive(Debug, Clone)]
pub struct GlyphInfo {
    pub glyph_id: u32,
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

/// Shape `text` with HarfBuzz via rustybuzz.
/// `script` is a 4-char ISO 15924 tag string (e.g. "Deva", "Latn").
#[cfg(feature = "pdf")]
pub fn shape_text(
    font_data: &[u8],
    text: &str,
    script: &str,
    lang: &str,
) -> Result<Vec<GlyphInfo>, RenderError> {
    use std::str::FromStr;

    use rustybuzz::{Direction, Face, Language, Script, Tag, UnicodeBuffer, script as scripts, shape};

    let face = Face::from_slice(font_data, 0).ok_or_else(|| RenderError::Font {
        message: "invalid font data".to_string(),
    })?;

    let mut buf = UnicodeBuffer::new();
    buf.push_str(text);

    let tag = Tag::from_bytes_lossy(script.as_bytes());
    let rb_script = Script::from_iso15924_tag(tag).unwrap_or(scripts::LATIN);
    buf.set_script(rb_script);

    // Language::from_str is provided via the FromStr trait impl in rustybuzz.
    if let Ok(language) = Language::from_str(lang) {
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

/// Rasterise shaped glyphs onto an RGBA canvas using ab_glyph.
///
/// `upem` is the font's units-per-em (from rustybuzz Face::units_per_em).
/// HarfBuzz positions are in font units; we scale them to pixels via `font_size_px / upem`.
#[cfg(feature = "pdf")]
pub fn render_glyphs(
    font_data: &[u8],
    glyphs: &[GlyphInfo],
    font_size_px: u32,
    upem: i32,
    width: u32,
    height: u32,
) -> Result<image::RgbaImage, RenderError> {
    use ab_glyph::{Font, FontRef, Glyph, GlyphId, PxScale, ScaleFont};
    use ab_glyph::point;
    use image::Rgba;

    let font = FontRef::try_from_slice(font_data)
        .map_err(|e| RenderError::Font { message: e.to_string() })?;

    // ab_glyph's PxScale is defined so that ascender + |descender| ≈ scale.y pixels.
    // We use the font's own line metrics to derive a px_per_unit factor, then set
    // scale = px_per_unit * upem so that a full em equals font_size_px pixels.
    // This keeps ab_glyph's internal advance measurements in sync with HarfBuzz's
    // design-unit positions (both share the same upem coordinate space).
    let scale = PxScale::from(font_size_px as f32);

    // Use ab_glyph's scaled metrics to position the baseline consistently.
    let scaled = font.as_scaled(scale);
    let ascent_px = scaled.ascent();
    let descent_px = scaled.descent();
    let line_gap_px = scaled.line_gap();
    let total_line_height = ascent_px - descent_px + line_gap_px;

    // Expand the canvas height to hold the full line metrics to avoid clipping.
    // We render into an oversized buffer and let the PDF matrix scale it to rect.
    let canvas_h = total_line_height.ceil() as u32 + 2;
    let canvas_w = width.max(canvas_h * 10); // at least as wide as the text
    let baseline_y = ascent_px;  // pixels from top of canvas to baseline

    eprintln!(
        "[render_glyphs] upem={upem} font_size_px={font_size_px} \
         ascent={ascent_px:.1} descent={descent_px:.1} \
         canvas={canvas_w}x{canvas_h} baseline_y={baseline_y:.1} \
         n_glyphs={}",
        glyphs.len()
    );

    // Opaque white background so JPEG conversion (which strips alpha) renders correctly.
    let mut img = image::RgbaImage::from_pixel(canvas_w, canvas_h, Rgba([255, 255, 255, 255]));
    let mut x_pen: f32 = 0.0;

    // px_per_unit converts HarfBuzz design units → pixel space used by ab_glyph.
    let px_per_unit = font_size_px as f32 / upem as f32;

    for (i, glyph_info) in glyphs.iter().enumerate() {
        let x_offset_px = glyph_info.x_offset as f32 * px_per_unit;
        let y_offset_px = glyph_info.y_offset as f32 * px_per_unit;
        let x_advance_px = glyph_info.x_advance as f32 * px_per_unit;

        let pos_x = x_pen + x_offset_px;
        let pos_y = baseline_y - y_offset_px;

        let glyph = Glyph {
            id: GlyphId(glyph_info.glyph_id as u16),
            scale,
            position: point(pos_x, pos_y),
        };

        let outlined = font.outline_glyph(glyph);
        if i < 4 {
            let bounds = outlined.as_ref().map(|o| o.px_bounds());
            eprintln!(
                "  glyph[{i}] id={} adv={x_advance_px:.1} off=({x_offset_px:.1},{y_offset_px:.1}) \
                 pos=({pos_x:.1},{pos_y:.1}) bounds={bounds:?} outlined={}",
                glyph_info.glyph_id,
                outlined.is_some()
            );
        }

        if let Some(outlined) = outlined {
            let bounds = outlined.px_bounds();
            // ab_glyph's draw() callback passes RELATIVE pixel coords within the glyph's
            // bounding box (0..width, 0..height). We must add bounds.min to get canvas coords.
            let bb_min_x = bounds.min.x as i32;
            let bb_min_y = bounds.min.y as i32;
            outlined.draw(|rel_px, rel_py, coverage| {
                if coverage <= 0.0 {
                    return;
                }
                let cx = bb_min_x + rel_px as i32;
                let cy = bb_min_y + rel_py as i32;
                if cx < 0 || cy < 0 || (cx as u32) >= canvas_w || (cy as u32) >= canvas_h {
                    return;
                }
                let cx = cx as u32;
                let cy = cy as u32;
                // Composite black glyph over existing pixel (which starts white).
                let existing = img.get_pixel(cx, cy);
                let inv = 1.0 - coverage.min(1.0);
                img.put_pixel(cx, cy, Rgba([
                    (existing[0] as f32 * inv) as u8,
                    (existing[1] as f32 * inv) as u8,
                    (existing[2] as f32 * inv) as u8,
                    255,
                ]));
            });
        }

        x_pen += x_advance_px;
    }

    // Scale down to the requested dimensions (the PDF matrix handles physical sizing)
    let final_img = if canvas_w != width || canvas_h != height {
        image::imageops::resize(&img, width, height, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    Ok(final_img)
}

/// Flatten RGBA onto white and encode as JPEG.
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

// ── non-pdf stubs (never called; exist only so the module compiles) ──────────

#[cfg(not(feature = "pdf"))]
pub fn shape_text(
    _font_data: &[u8],
    _text: &str,
    _script: &str,
    _lang: &str,
) -> Result<Vec<GlyphInfo>, RenderError> {
    Err(RenderError::FeatureDisabled)
}

#[cfg(not(feature = "pdf"))]
pub fn render_glyphs(
    _font_data: &[u8],
    _glyphs: &[GlyphInfo],
    _font_size_px: u32,
    _upem: i32,
    _width: u32,
    _height: u32,
) -> Result<image::RgbaImage, RenderError> {
    Err(RenderError::FeatureDisabled)
}

#[cfg(not(feature = "pdf"))]
pub fn to_jpeg(_img: image::RgbaImage, _quality: u8) -> Result<Vec<u8>, RenderError> {
    Err(RenderError::FeatureDisabled)
}

