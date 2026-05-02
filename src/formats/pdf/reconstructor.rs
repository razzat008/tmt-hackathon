//! PDF reconstruction — in-place content-stream replacement.

use std::path::Path;

use crate::formats::pdf::{FontVariants, TranslateConfig, TranslatedLine, error::ReconstructError};

// ── non-pdf stub ──────────────────────────────────────────────────────────────

#[cfg(not(feature = "pdf"))]
pub fn reconstruct_pdf(
    _input_path: &Path,
    _output_path: &Path,
    _translated_lines: &[TranslatedLine],
    _font_variants: Option<&FontVariants>,
    _config: &TranslateConfig,
) -> Result<(), ReconstructError> {
    Err(ReconstructError::FeatureDisabled)
}

// ── pdf implementation ────────────────────────────────────────────────────────

#[cfg(feature = "pdf")]
use std::collections::{HashMap, HashSet};

#[cfg(feature = "pdf")]
use lopdf::content::Operation as ContentOp;
#[cfg(feature = "pdf")]
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};

#[cfg(feature = "pdf")]
use crate::formats::pdf::bundled_fonts::NOTO_DEVA_REGULAR;

/// Name used to register the fallback CIDFont in page Resources/Font.
#[cfg(feature = "pdf")]
const FALLBACK_FONT_RESOURCE: &str = "F_TMT";

#[cfg(feature = "pdf")]
const REFLOW_RATIO: f32 = 1.35;
#[cfg(feature = "pdf")]
const REFLOW_MIN_CHARS: usize = 12;
#[cfg(feature = "pdf")]
const REFLOW_MIN_LINE_CHARS: usize = 10;

// ── per-page glyph tracking (matches reference FontUsage) ────────────────────

#[cfg(feature = "pdf")]
#[derive(Default)]
struct FontUsage {
    glyphs: HashSet<u16>,
    cmap: HashMap<u16, String>,
}

// ── text-state tracker (matches reference TextState) ─────────────────────────

#[cfg(feature = "pdf")]
#[derive(Default, Clone)]
struct TextState {
    font_name: Option<Vec<u8>>,
    font_size: Option<f32>,
    leading: Option<f32>,
}

// ── encoding hint (matches reference EncodingHint) ───────────────────────────

#[cfg(feature = "pdf")]
#[derive(Clone, Copy)]
enum EncodingHint {
    Utf16Be,
    Utf16Le,
    SingleByte,
}

// ═════════════════════════════════════════════════════════════════════════════
// Public entry point
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "pdf")]
pub fn reconstruct_pdf(
    input_path: &Path,
    output_path: &Path,
    translated_lines: &[TranslatedLine],
    _font_variants: Option<&FontVariants>,
    config: &TranslateConfig,
) -> Result<(), ReconstructError> {
    if translated_lines.is_empty() {
        std::fs::copy(input_path, output_path)?;
        return Ok(());
    }

    // Build source-text → translated-text lookup context.
    let mut ctx = build_translation_context(translated_lines);

    let mut doc = Document::load(input_path).map_err(|e| ReconstructError::Pdf {
        message: e.to_string(),
    })?;

    let pages: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    let font_bytes = NOTO_DEVA_REGULAR.to_vec();

    let mut font_usage = FontUsage::default();
    let mut any_shaped = false;

    for page_id in &pages {
        let content_data = match doc.get_page_content(*page_id) {
            Ok(d) => d,
            Err(_) => continue,
        };

        // Reset per-page emission tracking so repeated lines (e.g. headers)
        // are translated on every page.
        ctx.reset_page();

        let content_bytes =
            process_page_content(&content_data, &mut ctx, &font_bytes, &mut font_usage)?;

        replace_page_content(&mut doc, *page_id, content_bytes)?;
        any_shaped = true;
    }

    // Embed the CIDFont once and add it to every page's Resources.
    if any_shaped && !font_usage.glyphs.is_empty() {
        let font_id = embed_font(&mut doc, &font_bytes, &font_usage)?;
        for page_id in &pages {
            add_font_resource(&mut doc, *page_id, FALLBACK_FONT_RESOURCE, font_id)?;
        }
    }

    doc.save(output_path).map_err(|e| ReconstructError::Pdf {
        message: e.to_string(),
    })?;

    Ok(())
}

// ═════════════════════════════════════════════════════════════════════════════
// Translation context — fragment-level lookup preserving original PDF layout
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "pdf")]
struct TranslationContext {
    map: HashMap<String, String>,
}

#[cfg(feature = "pdf")]
impl TranslationContext {
    fn reset_page(&mut self) {}

    fn lookup(&self, text: &str) -> Option<String> {
        let key = normalize_lookup_key(text);
        if key.is_empty() {
            return None;
        }
        if let Some(t) = self.map.get(&key) {
            return Some(t.clone());
        }

        // Fallback: if a fragment has punctuation/case differences, translate by
        // concatenating known per-word fragments in order.
        let mut parts = Vec::new();
        for word in key.split_whitespace() {
            let stripped = normalize_word_key(word);
            if let Some(t) = self.map.get(&stripped) {
                parts.push(t.as_str());
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" "))
        }
    }
}

#[cfg(feature = "pdf")]
fn build_translation_context(lines: &[TranslatedLine]) -> TranslationContext {
    let mut map: HashMap<String, String> = HashMap::new();

    for tl in lines {
        let src_words: Vec<&str> = tl.source.text.split_whitespace().collect();
        let tgt_words: Vec<&str> = tl.translated_text.split_whitespace().collect();

        let line_key = normalize_lookup_key(&tl.source.text);
        map.entry(line_key)
            .or_insert_with(|| tl.translated_text.clone());

        if src_words.is_empty() || tgt_words.is_empty() {
            continue;
        }

        let max_n = src_words.len().min(8);
        for start in 0..src_words.len() {
            for len in 1..=max_n.min(src_words.len() - start) {
                let end = start + len;
                let key = normalize_lookup_key(&src_words[start..end].join(" "));
                if key.is_empty() {
                    continue;
                }
                let translated = proportional_target_words(&tgt_words, src_words.len(), start, end);
                if !translated.is_empty() {
                    map.entry(key.clone()).or_insert(translated.clone());
                    map.entry(normalize_word_key(&key)).or_insert(translated);
                }
            }
        }
    }

    TranslationContext { map }
}

#[cfg(feature = "pdf")]
fn proportional_target_words(
    tgt_words: &[&str],
    src_len: usize,
    start: usize,
    end: usize,
) -> String {
    if tgt_words.is_empty() || src_len == 0 {
        return String::new();
    }
    let m = tgt_words.len();
    let t_start = (start * m + src_len - 1) / src_len;
    let t_end = (end * m + src_len - 1) / src_len;
    let t_start = t_start.min(m);
    let t_end = t_end.min(m);
    if t_start >= t_end {
        return String::new();
    }
    tgt_words[t_start..t_end].join(" ")
}

#[cfg(feature = "pdf")]
fn normalize_word_key(text: &str) -> String {
    normalize_lookup_key(text)
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string()
}

#[cfg(feature = "pdf")]
fn normalize_lookup_key(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ═════════════════════════════════════════════════════════════════════════════
// Content-stream processing  (mirrors reference translate_content)
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "pdf")]
fn process_page_content(
    raw: &[u8],
    ctx: &mut TranslationContext,
    font_bytes: &[u8],
    font_usage: &mut FontUsage,
) -> Result<Vec<u8>, ReconstructError> {
    use lopdf::content::Content;

    let mut content = Content::decode(raw).map_err(|e| ReconstructError::Pdf {
        message: e.to_string(),
    })?;

    let mut new_ops = Vec::with_capacity(content.operations.len());
    let mut state = TextState::default();

    for op in &content.operations {
        match op.operator.as_str() {
            "Tf" => {
                if let Some((name, size)) = parse_tf(op) {
                    state.font_name = Some(name);
                    state.font_size = Some(size);
                }
                new_ops.push(op.clone());
            }
            "TL" => {
                if let Some(lead) = op.operands.first().and_then(object_to_f32) {
                    state.leading = Some(lead);
                }
                new_ops.push(op.clone());
            }
            "Tj" => handle_tj_op(
                op,
                ctx,
                font_bytes,
                font_usage,
                &mut new_ops,
                &mut state,
                false,
            )?,
            "'" => handle_tj_op(
                op,
                ctx,
                font_bytes,
                font_usage,
                &mut new_ops,
                &mut state,
                true,
            )?,
            "\"" => {
                handle_double_quote_op(op, ctx, font_bytes, font_usage, &mut new_ops, &mut state)?
            }
            "TJ" => handle_tj_array_op(op, ctx, font_bytes, font_usage, &mut new_ops, &mut state)?,
            _ => new_ops.push(op.clone()),
        }
    }

    content.operations = new_ops;
    content.encode().map_err(|e| ReconstructError::Pdf {
        message: e.to_string(),
    })
}

// ─── Tj / ' operator ──────────────────────────────────────────────────────

#[cfg(feature = "pdf")]
fn handle_tj_op(
    op: &lopdf::content::Operation,
    ctx: &mut TranslationContext,
    font_bytes: &[u8],
    font_usage: &mut FontUsage,
    new_ops: &mut Vec<lopdf::content::Operation>,
    state: &mut TextState,
    line_break_first: bool,
) -> Result<(), ReconstructError> {
    let Some(Object::String(bytes, fmt)) = op.operands.first() else {
        new_ops.push(op.clone());
        return Ok(());
    };

    let (original, hint) = decode_pdf_string(bytes);
    if original.trim().is_empty() {
        new_ops.push(op.clone());
        return Ok(());
    }

    let Some(translated) = ctx.lookup(&original) else {
        new_ops.push(op.clone());
        return Ok(());
    };

    let lines = reflow_lines(&original, &translated);

    if needs_fallback_font(&translated) {
        let (prev_name, prev_size) = (state.font_name.clone(), state.font_size);
        push_font_switch(
            new_ops,
            FALLBACK_FONT_RESOURCE.as_bytes(),
            state.font_size.unwrap_or(12.0),
            state,
        );
        emit_shaped_lines(new_ops, &lines, font_bytes, font_usage, state, line_break_first)?;
        if let (Some(n), Some(s)) = (prev_name, prev_size) {
            push_font_switch(new_ops, &n, s, state);
        }
    } else if lines.len() == 1 {
        let encoded = encode_with_hint(&lines[0], hint);
        new_ops.push(ContentOp::new(
            op.operator.as_str(),
            vec![Object::String(encoded, *fmt)],
        ));
    } else {
        emit_text_lines(new_ops, &lines, hint, *fmt, line_break_first, state);
    }
    Ok(())
}

// ─── " operator ───────────────────────────────────────────────────────────

#[cfg(feature = "pdf")]
fn handle_double_quote_op(
    op: &lopdf::content::Operation,
    ctx: &mut TranslationContext,
    font_bytes: &[u8],
    font_usage: &mut FontUsage,
    new_ops: &mut Vec<lopdf::content::Operation>,
    state: &mut TextState,
) -> Result<(), ReconstructError> {
    if op.operands.len() < 3 {
        new_ops.push(op.clone());
        return Ok(());
    }
    let word_spacing = op.operands[0].clone();
    let char_spacing = op.operands[1].clone();
    let Some(Object::String(bytes, fmt)) = op.operands.get(2) else {
        new_ops.push(op.clone());
        return Ok(());
    };
    let (original, hint) = decode_pdf_string(bytes);
    if original.trim().is_empty() {
        new_ops.push(op.clone());
        return Ok(());
    }
    let Some(translated) = ctx.lookup(&original) else {
        new_ops.push(op.clone());
        return Ok(());
    };

    let lines = reflow_lines(&original, &translated);

    if needs_fallback_font(&translated) {
        let (prev_name, prev_size) = (state.font_name.clone(), state.font_size);
        push_font_switch(
            new_ops,
            FALLBACK_FONT_RESOURCE.as_bytes(),
            state.font_size.unwrap_or(12.0),
            state,
        );
        emit_shaped_lines(new_ops, &lines, font_bytes, font_usage, state, true)?;
        if let (Some(n), Some(s)) = (prev_name, prev_size) {
            push_font_switch(new_ops, &n, s, state);
        }
    } else if lines.len() == 1 {
        let encoded = encode_with_hint(&lines[0], hint);
        new_ops.push(ContentOp::new(
            "\"",
            vec![word_spacing, char_spacing, Object::String(encoded, *fmt)],
        ));
    } else {
        new_ops.push(ContentOp::new("Tw", vec![word_spacing]));
        new_ops.push(ContentOp::new("Tc", vec![char_spacing]));
        emit_text_lines(new_ops, &lines, hint, *fmt, true, state);
    }
    Ok(())
}

// ─── TJ operator ──────────────────────────────────────────────────────────

#[cfg(feature = "pdf")]
fn handle_tj_array_op(
    op: &lopdf::content::Operation,
    ctx: &mut TranslationContext,
    font_bytes: &[u8],
    font_usage: &mut FontUsage,
    new_ops: &mut Vec<lopdf::content::Operation>,
    state: &mut TextState,
) -> Result<(), ReconstructError> {
    let Some(Object::Array(items)) = op.operands.first() else {
        new_ops.push(op.clone());
        return Ok(());
    };

    let mut original_parts: Vec<String> = Vec::new();
    let mut string_indices: Vec<(usize, EncodingHint, StringFormat)> = Vec::new();
    let mut full_text = String::new();

    for (idx, item) in items.iter().enumerate() {
        match item {
            Object::String(bytes, fmt) => {
                let (text, hint) = decode_pdf_string(bytes);
                string_indices.push((idx, hint, *fmt));
                original_parts.push(text.clone());
                full_text.push_str(&text);
            }
            Object::Integer(v) => {
                if is_space_adjustment(*v as f32) && !full_text.ends_with(' ') {
                    full_text.push(' ');
                }
            }
            Object::Real(v) => {
                if is_space_adjustment(*v) && !full_text.ends_with(' ') {
                    full_text.push(' ');
                }
            }
            _ => {}
        }
    }

    if original_parts.is_empty() || full_text.trim().is_empty() {
        new_ops.push(op.clone());
        return Ok(());
    }

    let Some(translated) = ctx.lookup(&full_text) else {
        new_ops.push(op.clone());
        return Ok(());
    };

    let lines = reflow_lines(&full_text, &translated);

    let hint = string_indices.first().map(|(_, h, _)| *h).unwrap_or(EncodingHint::SingleByte);
    let fmt = string_indices.first().map(|(_, _, f)| *f).unwrap_or(StringFormat::Literal);

    if needs_fallback_font(&translated) {
        let (prev_name, prev_size) = (state.font_name.clone(), state.font_size);
        push_font_switch(
            new_ops,
            FALLBACK_FONT_RESOURCE.as_bytes(),
            state.font_size.unwrap_or(12.0),
            state,
        );
        emit_shaped_lines(new_ops, &lines, font_bytes, font_usage, state, false)?;
        if let (Some(n), Some(s)) = (prev_name, prev_size) {
            push_font_switch(new_ops, &n, s, state);
        }
    } else if lines.len() == 1 {
        // Preserve original TJ structure and kerning; redistribute translated text
        // proportionally across the original string slots.
        let redistributed = redistribute_by_length(&translated, &original_parts);
        let mut new_items = items.clone();
        for ((idx, _, _), part) in string_indices.iter().zip(redistributed) {
            let encoded = encode_with_hint(&part, hint);
            new_items[*idx] = Object::String(encoded, fmt);
        }
        new_ops.push(ContentOp::new("TJ", vec![Object::Array(new_items)]));
    } else {
        emit_text_lines(new_ops, &lines, hint, fmt, false, state);
    }
    Ok(())
}

// ═════════════════════════════════════════════════════════════════════════════
// Shaped-line emission  (mirrors reference emit_shaped_lines)
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "pdf")]
fn emit_shaped_lines(
    new_ops: &mut Vec<lopdf::content::Operation>,
    lines: &[String],
    font_bytes: &[u8],
    font_usage: &mut FontUsage,
    state: &mut TextState,
    initial_break: bool,
) -> Result<(), ReconstructError> {
    let mut first = true;
    for line in lines {
        if first {
            if initial_break {
                push_line_break(new_ops, state);
            }
        } else {
            push_line_break(new_ops, state);
        }

        let tj_array = shape_text_to_tj_array(line, font_bytes, font_usage)?;
        if !tj_array.is_empty() {
            new_ops.push(ContentOp::new("TJ", vec![Object::Array(tj_array)]));
        }
        first = false;
    }
    Ok(())
}

// ─── Non-shaping text emission (ASCII-compatible translations) ────────────────

#[cfg(feature = "pdf")]
fn emit_text_lines(
    new_ops: &mut Vec<lopdf::content::Operation>,
    lines: &[String],
    hint: EncodingHint,
    fmt: StringFormat,
    initial_break: bool,
    state: &mut TextState,
) {
    let mut first = true;
    for line in lines {
        if first {
            if initial_break {
                push_line_break(new_ops, state);
            }
        } else {
            push_line_break(new_ops, state);
        }
        let encoded = encode_with_hint(line, hint);
        new_ops.push(ContentOp::new("Tj", vec![Object::String(encoded, fmt)]));
        first = false;
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// HarfBuzz shaping → TJ array  (exact mirror of reference shape_text_to_tj_array)
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "pdf")]
fn shape_text_to_tj_array(
    text: &str,
    font_bytes: &[u8],
    font_usage: &mut FontUsage,
) -> Result<Vec<Object>, ReconstructError> {
    use rustybuzz::{Face as RbFace, UnicodeBuffer};
    use ttf_parser::Face;

    let rb_face = RbFace::from_slice(font_bytes, 0).ok_or_else(|| ReconstructError::Pdf {
        message: "Failed to parse font for shaping".into(),
    })?;

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    let glyph_buffer = rustybuzz::shape(&rb_face, &[], buffer);

    let infos = glyph_buffer.glyph_infos();
    let positions = glyph_buffer.glyph_positions();

    if infos.is_empty() {
        return Ok(vec![]);
    }

    let face = Face::parse(font_bytes, 0).map_err(|_| ReconstructError::Pdf {
        message: "Failed to parse font metrics".into(),
    })?;
    let units_per_em = face.units_per_em().max(1) as f32;

    let mut tj: Vec<Object> = Vec::with_capacity(infos.len() * 2);
    let text_bytes = text.as_bytes();

    for i in 0..infos.len() {
        let gid = infos[i].glyph_id as u16;
        font_usage.glyphs.insert(gid);

        // Cluster → Unicode slice for ToUnicode CMap.
        let cluster_start = infos[i].cluster as usize;
        let cluster_end = if i + 1 < infos.len() {
            infos[i + 1].cluster as usize
        } else {
            text_bytes.len()
        };
        if let Some(slice) = text.get(cluster_start..cluster_end) {
            font_usage
                .cmap
                .entry(gid)
                .or_insert_with(|| slice.to_string());
        }

        // x_offset correction (same sign convention as reference).
        let x_offset = positions[i].x_offset as f32 / 64.0;
        if x_offset.abs() > 0.01 {
            let adjust = -x_offset * 1000.0 / units_per_em;
            tj.push(Object::Real(adjust));
        }

        // Glyph encoded as 2-byte big-endian CID (GID == CID for Identity-H).
        let cid_bytes = [(gid >> 8) as u8, (gid & 0xFF) as u8];
        tj.push(Object::String(cid_bytes.to_vec(), StringFormat::Literal));
    }

    Ok(tj)
}

// ═════════════════════════════════════════════════════════════════════════════
// Font embedding  (exact mirror of reference embed_font + helpers)
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "pdf")]
fn embed_font(
    doc: &mut Document,
    font_bytes: &[u8],
    usage: &FontUsage,
) -> Result<ObjectId, ReconstructError> {
    use ttf_parser::Face;

    let face = Face::parse(font_bytes, 0).map_err(|_| ReconstructError::Pdf {
        message: "Font file is not a valid TrueType font".into(),
    })?;

    let base_font_name = sanitize_pdf_name(
        face_best_name(&face).as_deref().unwrap_or("TMTFont"),
        "TMTFont",
    );

    let font_file_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::from_iter([("Length1", Object::Integer(font_bytes.len() as i64))]),
        font_bytes.to_vec(),
    )));

    let bbox = face.global_bounding_box();
    let ascent = face.ascender() as i64;
    let descent = face.descender() as i64;
    let cap_height = face.capital_height().unwrap_or(face.ascender()) as i64;
    let italic_angle = face.italic_angle().unwrap_or(0.0);

    let font_descriptor = Dictionary::from_iter([
        ("Type", Object::Name(b"FontDescriptor".to_vec())),
        ("FontName", Object::Name(base_font_name.as_bytes().to_vec())),
        ("Flags", Object::Integer(4)),
        (
            "FontBBox",
            Object::Array(vec![
                Object::Integer(bbox.x_min as i64),
                Object::Integer(bbox.y_min as i64),
                Object::Integer(bbox.x_max as i64),
                Object::Integer(bbox.y_max as i64),
            ]),
        ),
        ("Ascent", Object::Integer(ascent)),
        ("Descent", Object::Integer(descent)),
        ("CapHeight", Object::Integer(cap_height)),
        ("ItalicAngle", Object::Real(italic_angle)),
        ("StemV", Object::Integer(0)),
        ("FontFile2", Object::Reference(font_file_id)),
    ]);
    let font_descriptor_id = doc.add_object(Object::Dictionary(font_descriptor));

    let (default_width, widths_array) = build_widths_from_glyphs(&face, &usage.glyphs);

    let cid_to_gid_map = build_cid_to_gid_map_identity(&usage.glyphs);
    let cid_to_gid_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        cid_to_gid_map,
    )));

    let cid_system_info = Dictionary::from_iter([
        (
            "Registry",
            Object::String(b"Adobe".to_vec(), StringFormat::Literal),
        ),
        (
            "Ordering",
            Object::String(b"Identity".to_vec(), StringFormat::Literal),
        ),
        ("Supplement", Object::Integer(0)),
    ]);

    let cid_font = Dictionary::from_iter([
        ("Type", Object::Name(b"Font".to_vec())),
        ("Subtype", Object::Name(b"CIDFontType2".to_vec())),
        ("BaseFont", Object::Name(base_font_name.as_bytes().to_vec())),
        ("CIDSystemInfo", Object::Dictionary(cid_system_info)),
        ("FontDescriptor", Object::Reference(font_descriptor_id)),
        ("DW", Object::Integer(default_width)),
        ("W", Object::Array(widths_array)),
        ("CIDToGIDMap", Object::Reference(cid_to_gid_id)),
    ]);
    let cid_font_id = doc.add_object(Object::Dictionary(cid_font));

    let to_unicode_id = if usage.cmap.is_empty() {
        None
    } else {
        let cmap = build_tounicode_cmap_from_mapping(&usage.cmap);
        Some(doc.add_object(Object::Stream(Stream::new(Dictionary::new(), cmap))))
    };

    let mut type0_font = Dictionary::from_iter([
        ("Type", Object::Name(b"Font".to_vec())),
        ("Subtype", Object::Name(b"Type0".to_vec())),
        ("BaseFont", Object::Name(base_font_name.as_bytes().to_vec())),
        ("Encoding", Object::Name(b"Identity-H".to_vec())),
        (
            "DescendantFonts",
            Object::Array(vec![Object::Reference(cid_font_id)]),
        ),
    ]);

    if let Some(id) = to_unicode_id {
        type0_font.set("ToUnicode", Object::Reference(id));
    }

    Ok(doc.add_object(Object::Dictionary(type0_font)))
}

#[cfg(feature = "pdf")]
fn build_widths_from_glyphs(face: &ttf_parser::Face, glyphs: &HashSet<u16>) -> (i64, Vec<Object>) {
    let upem = face.units_per_em().max(1) as f32;

    let default_width = face
        .glyph_index(' ')
        .and_then(|gid| face.glyph_hor_advance(gid))
        .map(|adv| ((adv as f32 / upem) * 1000.0).round() as i64)
        .unwrap_or(1000);

    if glyphs.is_empty() {
        return (default_width, vec![]);
    }

    let mut cids: Vec<u16> = glyphs.iter().copied().collect();
    cids.sort_unstable();

    let mut w_array: Vec<Object> = Vec::new();
    let mut i = 0usize;

    while i < cids.len() {
        let start = cids[i];
        let mut end = start;
        while i + 1 < cids.len() && cids[i + 1] == end + 1 {
            i += 1;
            end = cids[i];
        }
        let mut widths = Vec::new();
        for cid in start..=end {
            let gid = ttf_parser::GlyphId(cid);
            let w = face
                .glyph_hor_advance(gid)
                .map(|adv| ((adv as f32 / upem) * 1000.0).round() as i64)
                .unwrap_or(default_width);
            widths.push(Object::Integer(w));
        }
        w_array.push(Object::Integer(start as i64));
        w_array.push(Object::Array(widths));
        i += 1;
    }

    (default_width, w_array)
}

#[cfg(feature = "pdf")]
fn build_cid_to_gid_map_identity(glyphs: &HashSet<u16>) -> Vec<u8> {
    if glyphs.is_empty() {
        return vec![];
    }
    let max_cid = glyphs.iter().copied().max().unwrap_or(0) as usize;
    let mut map = vec![0u8; (max_cid + 1) * 2];
    for cid in 0..=max_cid {
        let off = cid * 2;
        map[off] = (cid >> 8) as u8;
        map[off + 1] = (cid & 0xFF) as u8;
    }
    map
}

#[cfg(feature = "pdf")]
fn build_tounicode_cmap_from_mapping(mapping: &HashMap<u16, String>) -> Vec<u8> {
    let mut cids: Vec<u16> = mapping.keys().copied().collect();
    cids.sort_unstable();

    let mut out = String::new();
    out.push_str("/CIDInit /ProcSet findresource begin\n");
    out.push_str("12 dict begin\n");
    out.push_str("begincmap\n");
    out.push_str("/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def\n");
    out.push_str("/CMapName /Adobe-Identity-UCS def\n");
    out.push_str("/CMapType 2 def\n");
    out.push_str("1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");

    for chunk in cids.chunks(100) {
        out.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for cid in chunk {
            let seq = mapping.get(cid).map(|s| s.as_str()).unwrap_or("");
            let utf16 = encode_utf16be_hex_str(seq);
            out.push_str(&format!("<{:04X}> <{}>\n", cid, utf16));
        }
        out.push_str("endbfchar\n");
    }

    out.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    out.into_bytes()
}

#[cfg(feature = "pdf")]
fn encode_utf16be_hex_str(s: &str) -> String {
    let mut out = String::new();
    for unit in s.encode_utf16() {
        out.push_str(&format!("{:04X}", unit));
    }
    out
}

// ═════════════════════════════════════════════════════════════════════════════
// Font resource registration  (mirrors reference add_font_resource)
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "pdf")]
fn add_font_resource(
    doc: &mut Document,
    page_id: ObjectId,
    resource_name: &str,
    font_id: ObjectId,
) -> Result<(), ReconstructError> {
    // Resolve indirect Resources ref if present.
    let resources_ref: Option<ObjectId> = {
        let page = doc.get_object(page_id).map_err(lop_err)?;
        let dict = page.as_dict().map_err(lop_err)?;
        match dict.get(b"Resources") {
            Ok(Object::Reference(id)) => Some(*id),
            _ => None,
        }
    };

    if let Some(res_id) = resources_ref {
        return add_font_to_resources_object(doc, res_id, resource_name, font_id);
    }

    // Ensure Resources dict exists inline.
    {
        let page = doc.get_object_mut(page_id).map_err(lop_err)?;
        let dict = page.as_dict_mut().map_err(lop_err)?;
        if dict.get(b"Resources").is_err() {
            dict.set("Resources", Dictionary::new());
        }
    }

    // Check if Font is an indirect ref inside inline Resources.
    let font_ref: Option<ObjectId> = {
        let page = doc.get_object(page_id).map_err(lop_err)?;
        let dict = page.as_dict().map_err(lop_err)?;
        let res = dict
            .get(b"Resources")
            .and_then(|o| o.as_dict())
            .map_err(lop_err)?;
        match res.get(b"Font") {
            Ok(Object::Reference(id)) => Some(*id),
            _ => None,
        }
    };

    if let Some(font_ref_id) = font_ref {
        let font_dict = doc
            .get_object_mut(font_ref_id)
            .map_err(lop_err)?
            .as_dict_mut()
            .map_err(lop_err)?;
        font_dict.set(resource_name, Object::Reference(font_id));
        return Ok(());
    }

    // Inline Resources/Font.
    let page = doc.get_object_mut(page_id).map_err(lop_err)?;
    let page_dict = page.as_dict_mut().map_err(lop_err)?;
    let resources = page_dict
        .get_mut(b"Resources")
        .map_err(lop_err)?
        .as_dict_mut()
        .map_err(lop_err)?;

    if resources.get(b"Font").is_err() {
        resources.set("Font", Dictionary::new());
    }

    let font_dict = resources
        .get_mut(b"Font")
        .map_err(lop_err)?
        .as_dict_mut()
        .map_err(lop_err)?;
    font_dict.set(resource_name, Object::Reference(font_id));
    Ok(())
}

#[cfg(feature = "pdf")]
fn add_font_to_resources_object(
    doc: &mut Document,
    resources_id: ObjectId,
    resource_name: &str,
    font_id: ObjectId,
) -> Result<(), ReconstructError> {
    let font_ref: Option<ObjectId> = {
        let res = doc.get_object(resources_id).map_err(lop_err)?;
        let dict = res.as_dict().map_err(lop_err)?;
        match dict.get(b"Font") {
            Ok(Object::Reference(id)) => Some(*id),
            _ => None,
        }
    };

    if let Some(font_ref_id) = font_ref {
        let font_dict = doc
            .get_object_mut(font_ref_id)
            .map_err(lop_err)?
            .as_dict_mut()
            .map_err(lop_err)?;
        font_dict.set(resource_name, Object::Reference(font_id));
        return Ok(());
    }

    {
        let res = doc.get_object_mut(resources_id).map_err(lop_err)?;
        let dict = res.as_dict_mut().map_err(lop_err)?;
        if dict.get(b"Font").is_err() {
            dict.set("Font", Dictionary::new());
        }
    }

    let res = doc.get_object_mut(resources_id).map_err(lop_err)?;
    let dict = res.as_dict_mut().map_err(lop_err)?;
    let font_dict = dict
        .get_mut(b"Font")
        .map_err(lop_err)?
        .as_dict_mut()
        .map_err(lop_err)?;
    font_dict.set(resource_name, Object::Reference(font_id));
    Ok(())
}

// ═════════════════════════════════════════════════════════════════════════════
// Page-content replacement  (mirrors reference replace_page_content)
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "pdf")]
fn replace_page_content(
    doc: &mut Document,
    page_id: ObjectId,
    content: Vec<u8>,
) -> Result<(), ReconstructError> {
    let stream_id = doc.add_object(Object::Stream(Stream::new(Dictionary::new(), content)));
    let page_dict = doc
        .get_object_mut(page_id)
        .map_err(lop_err)?
        .as_dict_mut()
        .map_err(lop_err)?;
    page_dict.set("Contents", Object::Reference(stream_id));
    Ok(())
}

// ═════════════════════════════════════════════════════════════════════════════
// Text-state helpers
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "pdf")]
fn parse_tf(op: &lopdf::content::Operation) -> Option<(Vec<u8>, f32)> {
    if op.operands.len() < 2 {
        return None;
    }
    let name = match op.operands.get(0) {
        Some(Object::Name(n)) => n.clone(),
        _ => return None,
    };
    let size = object_to_f32(op.operands.get(1)?)?;
    Some((name, size))
}

#[cfg(feature = "pdf")]
fn object_to_f32(obj: &Object) -> Option<f32> {
    match obj {
        Object::Real(v) => Some(*v),
        Object::Integer(v) => Some(*v as f32),
        _ => None,
    }
}

#[cfg(feature = "pdf")]
fn push_font_switch(
    ops: &mut Vec<lopdf::content::Operation>,
    name: &[u8],
    size: f32,
    state: &mut TextState,
) {
    ops.push(ContentOp::new(
        "Tf",
        vec![Object::Name(name.to_vec()), Object::Real(size)],
    ));
    state.font_name = Some(name.to_vec());
    state.font_size = Some(size);
}

#[cfg(feature = "pdf")]
fn push_line_break(ops: &mut Vec<lopdf::content::Operation>, state: &mut TextState) {
    if state.leading.unwrap_or(0.0) > 0.0 {
        ops.push(ContentOp::new("T*", vec![]));
        return;
    }
    let size = state.font_size.unwrap_or(12.0);
    let leading = (size * 1.2).max(1.0);
    ops.push(ContentOp::new("TL", vec![Object::Real(leading)]));
    state.leading = Some(leading);
    ops.push(ContentOp::new("T*", vec![]));
}

#[cfg(feature = "pdf")]
fn is_space_adjustment(value: f32) -> bool {
    value.abs() >= 100.0
}

#[cfg(feature = "pdf")]
fn needs_fallback_font(text: &str) -> bool {
    text.chars().any(|c| c > '\u{007F}')
}

// ═════════════════════════════════════════════════════════════════════════════
// Reflow helpers  (mirrors reference should_reflow / reflow_text)
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "pdf")]
fn reflow_lines(original: &str, translated: &str) -> Vec<String> {
    let orig_len = original.chars().count().max(1);
    let trans_len = translated.chars().count();
    let should =
        orig_len >= REFLOW_MIN_CHARS && (trans_len as f32 / orig_len as f32) > REFLOW_RATIO;

    if should {
        reflow_text(translated, orig_len.max(REFLOW_MIN_LINE_CHARS))
    } else {
        vec![translated.to_string()]
    }
}

#[cfg(feature = "pdf")]
fn reflow_text(text: &str, max_chars: usize) -> Vec<String> {
    if text.trim().is_empty() {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let wlen = word.chars().count();
        let clen = current.chars().count();
        if current.is_empty() {
            if wlen <= max_chars {
                current.push_str(word);
            } else {
                for chunk in split_word(word, max_chars.max(1)) {
                    lines.push(chunk);
                }
            }
            continue;
        }
        if clen + 1 + wlen <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = String::new();
            if wlen <= max_chars {
                current.push_str(word);
            } else {
                for chunk in split_word(word, max_chars.max(1)) {
                    lines.push(chunk);
                }
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        vec![text.to_string()]
    } else {
        lines
    }
}

#[cfg(feature = "pdf")]
fn split_word(word: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return vec![word.to_string()];
    }
    word.chars()
        .collect::<Vec<_>>()
        .chunks(max_chars)
        .map(|c| c.iter().collect())
        .collect()
}

// ─── TJ redistribution for non-fallback multi-part arrays ────────────────────

#[cfg(feature = "pdf")]
fn redistribute_by_length(translated: &str, original_parts: &[String]) -> Vec<String> {
    if original_parts.is_empty() {
        return vec![];
    }
    if original_parts.len() == 1 {
        return vec![translated.to_string()];
    }
    let original_lens: Vec<usize> = original_parts.iter().map(|s| s.chars().count()).collect();
    let total_original: usize = original_lens.iter().sum();
    let t_chars: Vec<char> = translated.chars().collect();
    let total_t = t_chars.len();
    if total_original == 0 {
        return vec![String::new(); original_parts.len()];
    }
    let mut result = Vec::with_capacity(original_parts.len());
    let mut char_offset = 0usize;
    for (idx, _) in original_lens.iter().enumerate() {
        let portion = if idx == original_lens.len() - 1 {
            total_t.saturating_sub(char_offset)
        } else {
            let frac = original_lens[idx] as f64 / total_original as f64;
            (frac * total_t as f64).round() as usize
        };
        let part: String = t_chars.iter().skip(char_offset).take(portion).collect();
        char_offset += portion;
        result.push(part);
    }
    result
}

// ═════════════════════════════════════════════════════════════════════════════
// Encoding helpers  (mirrors reference decode_pdf_string / encode helpers)
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "pdf")]
fn decode_pdf_string(bytes: &[u8]) -> (String, EncodingHint) {
    let hint = detect_encoding_hint(bytes);
    let text = match hint {
        EncodingHint::Utf16Be => decode_utf16be(bytes),
        EncodingHint::Utf16Le => decode_utf16le(bytes),
        EncodingHint::SingleByte => decode_single_byte(bytes),
    };
    (text, hint)
}

#[cfg(feature = "pdf")]
fn detect_encoding_hint(bytes: &[u8]) -> EncodingHint {
    if bytes.len() >= 2 {
        if bytes[0] == 0xFE && bytes[1] == 0xFF {
            return EncodingHint::Utf16Be;
        }
        if bytes[0] == 0xFF && bytes[1] == 0xFE {
            return EncodingHint::Utf16Le;
        }
    }
    if looks_like_utf16(bytes, true) {
        return EncodingHint::Utf16Be;
    }
    if looks_like_utf16(bytes, false) {
        return EncodingHint::Utf16Le;
    }
    EncodingHint::SingleByte
}

#[cfg(feature = "pdf")]
fn looks_like_utf16(bytes: &[u8], big_endian: bool) -> bool {
    if bytes.len() < 4 || bytes.len() % 2 != 0 {
        return false;
    }
    let (mut ze, mut zo) = (0usize, 0usize);
    for (idx, b) in bytes.iter().enumerate() {
        if *b == 0 {
            if idx % 2 == 0 {
                ze += 1;
            } else {
                zo += 1;
            }
        }
    }
    let pairs = bytes.len() / 2;
    let er = ze as f32 / pairs as f32;
    let or_ = zo as f32 / pairs as f32;
    if big_endian {
        er > 0.4 && or_ < 0.2
    } else {
        or_ > 0.4 && er < 0.2
    }
}

#[cfg(feature = "pdf")]
fn decode_utf16be(bytes: &[u8]) -> String {
    let start = if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        2
    } else {
        0
    };
    let u16s: Vec<u16> = bytes[start..]
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&u16s).to_string()
}

#[cfg(feature = "pdf")]
fn decode_utf16le(bytes: &[u8]) -> String {
    let start = if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        2
    } else {
        0
    };
    let u16s: Vec<u16> = bytes[start..]
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&u16s).to_string()
}

#[cfg(feature = "pdf")]
fn decode_single_byte(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    let (cow, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
    cow.into_owned()
}

#[cfg(feature = "pdf")]
fn encode_with_hint(text: &str, hint: EncodingHint) -> Vec<u8> {
    let ascii_only = text.chars().all(|c| c <= '\u{007F}');
    if ascii_only && matches!(hint, EncodingHint::SingleByte) {
        return text.as_bytes().to_vec();
    }
    match hint {
        EncodingHint::Utf16Le => encode_utf16le(text),
        _ => encode_utf16be_bytes(text),
    }
}

#[cfg(feature = "pdf")]
fn encode_utf16be_bytes(text: &str) -> Vec<u8> {
    let mut out = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

#[cfg(feature = "pdf")]
fn encode_utf16le(text: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

// ═════════════════════════════════════════════════════════════════════════════
// Font name helpers  (exact mirror of reference face_best_name / sanitize_pdf_name)
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "pdf")]
fn face_best_name(face: &ttf_parser::Face) -> Option<String> {
    use ttf_parser::name_id;
    for target in [
        name_id::POST_SCRIPT_NAME,
        name_id::FULL_NAME,
        name_id::FAMILY,
    ] {
        for name in face.names() {
            if name.name_id == target {
                if let Some(v) = name.to_string() {
                    return Some(v);
                }
            }
        }
    }
    None
}

#[cfg(feature = "pdf")]
fn sanitize_pdf_name(name: &str, fallback: &str) -> String {
    let out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Error conversion helper
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "pdf")]
fn lop_err(e: lopdf::Error) -> ReconstructError {
    ReconstructError::Pdf {
        message: e.to_string(),
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Tests
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(all(test, feature = "pdf"))]
mod tests {
    use super::*;

    #[test]
    fn utf16be_hex_round_trip() {
        assert_eq!(encode_utf16be_hex_str("न"), "0928");
        assert_eq!(encode_utf16be_hex_str("नम"), "0928092E");
    }

    #[test]
    fn normalize_lookup_key_collapses_whitespace() {
        assert_eq!(normalize_lookup_key("  hello   world  "), "hello world");
    }
}
