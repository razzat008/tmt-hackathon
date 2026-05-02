use crate::formats::pdf::error::RenderError;

#[derive(Debug, Clone)]
pub struct GlyphInfo {
    pub glyph_id: u32,
    pub cluster: u32,
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
            cluster: info.cluster,   // byte offset of the source codepoint this glyph belongs to
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
    font_data: &[u8],
    glyphs: &[GlyphInfo],
    font_size_px: u32,
    upem: i32,
) -> Result<(image::RgbaImage, f32), RenderError> {
    use ab_glyph::{Font, FontRef, Glyph, GlyphId, PxScale, ScaleFont};
    use ab_glyph::point;
    use image::Rgba;

    let font = FontRef::try_from_slice(font_data)
        .map_err(|e| RenderError::Font { message: e.to_string() })?;

    let scale = PxScale::from(font_size_px as f32);
    let scaled = font.as_scaled(scale);
    let ascent_px  = scaled.ascent();
    let descent_px = scaled.descent(); // negative in ab_glyph convention

    // Bug 11B fix: explicit top/bottom headroom so Devanagari combining marks are not clipped.
    // top_extra (0.30×) covers vowel signs above the ascent line (ि ी ँ ं).
    // bottom_extra (0.35×) covers below-baseline matras (ु ू) that extend past descent.
    let top_extra    = (font_size_px as f32 * 0.30).ceil() as u32;
    let bottom_extra = (font_size_px as f32 * 0.35).ceil() as u32;
    let descent_abs  = (-descent_px).ceil() as u32;
    let canvas_h = (ascent_px.ceil() as u32 + descent_abs + top_extra + bottom_extra)
        .max((font_size_px as f32 * 1.5).ceil() as u32);

    // Baseline y: ascent pushed down by top_extra so marks above it stay within canvas.
    let baseline_y = ascent_px + top_extra as f32;

    // Bug 10A fix: canvas_w = natural advance sum, not the rect width.
    // Stamping the cropped canvas in the PDF at exactly shaped_width_pt removes the
    // artificial inter-glyph gaps caused by stretching a wide canvas over a small rect.
    let px_per_unit = font_size_px as f32 / upem as f32;
    let total_advance: f32 = glyphs.iter().map(|g| g.x_advance as f32 * px_per_unit).sum();
    // Step 4: +2px prevents JPEG DCT block artifacts at the right canvas edge (white→text boundary).
    let canvas_w = (total_advance.ceil() as u32 + 2).max(1);

    eprintln!(
        "[render_glyphs] upem={upem} font_size_px={font_size_px} \
         ascent={ascent_px:.1} descent={descent_px:.1} top_extra={top_extra} bottom_extra={bottom_extra} \
         canvas={canvas_w}x{canvas_h} baseline_y={baseline_y:.1} \
         n_glyphs={}",
        glyphs.len()
    );

    // Opaque white background so JPEG conversion (which strips alpha) renders correctly.
    let mut img = image::RgbaImage::from_pixel(canvas_w, canvas_h, Rgba([255, 255, 255, 255]));
    let mut x_pen: f32 = 0.0;

    for (i, glyph_info) in glyphs.iter().enumerate() {
        let x_offset_px = glyph_info.x_offset as f32 * px_per_unit;
        // Bug 11E: MINUS y_offset because HarfBuzz is positive-y-up but image coords are
        // positive-y-down.  A positive HarfBuzz y_offset lifts the glyph up → smaller
        // y pixel coordinate → subtract from baseline_y.
        let y_offset_px = glyph_info.y_offset as f32 * px_per_unit;
        let x_advance_px = glyph_info.x_advance as f32 * px_per_unit;

        let pos_x = x_pen + x_offset_px;
        let pos_y = baseline_y - y_offset_px; // MINUS: HarfBuzz y-up → image y-down

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
            // ab_glyph draw() passes RELATIVE coords within the glyph bbox.
            // Must add bounds.min to get canvas-absolute coordinates.
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

    Ok((img, baseline_y))
}

#[cfg(all(feature = "pdf", test))]
mod tests {
    use super::*;

    fn noto_regular() -> &'static [u8] {
        crate::formats::pdf::bundled_fonts::NOTO_DEVA_REGULAR
    }

    fn upem_for(font_data: &[u8]) -> i32 {
        use rustybuzz::Face;
        Face::from_slice(font_data, 0).map(|f| f.units_per_em() as i32).unwrap_or(1000)
    }

    // Regression: canvas_h must be ≥ font_size_px * 1.5 for any Devanagari text.
    #[test]
    fn canvas_h_at_least_one_and_half_times_font_size() {
        let font_data = noto_regular();
        let font_size_px = 14_u32;
        let upem = upem_for(font_data);
        let glyphs = shape_text(font_data, "नमस्ते", "Deva", "ne").unwrap();
        let (img, _baseline) = render_glyphs(font_data, &glyphs, font_size_px, upem).unwrap();
        assert!(
            img.height() >= (font_size_px as f32 * 1.5).ceil() as u32,
            "canvas_h={} < 1.5 * font_size_px={}",
            img.height(), font_size_px
        );
    }

    // Regression: canvas_w == shaped advance sum (no dead whitespace on the right).
    #[test]
    fn canvas_width_equals_shaped_advance() {
        let font_data = noto_regular();
        let font_size_px = 16_u32;
        let upem = upem_for(font_data);
        let glyphs = shape_text(font_data, "प्रारम्भिक", "Deva", "ne").unwrap();
        let px_per_unit = font_size_px as f32 / upem as f32;
        let expected_w: f32 = glyphs.iter().map(|g| g.x_advance as f32 * px_per_unit).sum();
        let (img, _) = render_glyphs(font_data, &glyphs, font_size_px, upem).unwrap();
        // canvas_w = ceil(advance) + 2 (JPEG edge artifact margin)
        assert_eq!(
            img.width(),
            expected_w.ceil() as u32 + 2,
            "canvas_w mismatch: got {} expected {}",
            img.width(), expected_w.ceil() as u32 + 2
        );
    }
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
) -> Result<(image::RgbaImage, f32), RenderError> {
    Err(RenderError::FeatureDisabled)
}

#[cfg(not(feature = "pdf"))]
pub fn to_jpeg(_img: image::RgbaImage, _quality: u8) -> Result<Vec<u8>, RenderError> {
    Err(RenderError::FeatureDisabled)
}

