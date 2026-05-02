use std::cmp::Ordering;
use std::path::Path;

use crate::formats::pdf::{PdfTextLine, error::ParseError, utils::fix_devanagari_clusters};

#[derive(Debug, Clone)]
struct PdfWord {
    text: String,
    x0: f32,
    top: f32,
    x1: f32,
    bottom: f32,
    font_size: f32,
    font_name: Option<String>,
}

const Y_TOLERANCE: f32 = 3.0;
#[allow(dead_code)]
const X_TOLERANCE: f32 = 2.0;

#[cfg(feature = "pdf")]
pub fn parse_metadata(path: &Path) -> Result<Vec<PdfTextLine>, ParseError> {
    use lopdf::content::Content;
    use lopdf::Document;

    let doc = Document::load(path).map_err(|e| ParseError::Pdf {
        message: e.to_string(),
    })?;

    let pages = doc.get_pages();
    let mut all_lines = Vec::new();

    // pages is BTreeMap<u32, ObjectId> keyed by 1-based page number
    for (page_num, &page_id) in &pages {
        let page_index = (*page_num as usize).saturating_sub(1);
        let page_height = get_page_height(&doc, page_id).unwrap_or(792.0);

        let content_data = match doc.get_page_content(page_id) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let content = match Content::decode(&content_data) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let words = extract_words_from_content(&content, page_height);
        if words.is_empty() {
            continue;
        }
        let mut lines = group_into_lines(words, page_index, Y_TOLERANCE);
        all_lines.append(&mut lines);
    }

    Ok(all_lines)
}

#[cfg(not(feature = "pdf"))]
pub fn parse_metadata(_path: &Path) -> Result<Vec<PdfTextLine>, ParseError> {
    Err(ParseError::FeatureDisabled)
}

#[cfg(feature = "pdf")]
fn get_page_height(doc: &lopdf::Document, page_id: lopdf::ObjectId) -> Result<f32, ParseError> {
    let page = doc.get_object(page_id).map_err(|e| ParseError::Pdf {
        message: e.to_string(),
    })?;
    let dict = page.as_dict().map_err(|e| ParseError::Pdf {
        message: e.to_string(),
    })?;
    let media_box = dict.get(b"MediaBox").map_err(|e| ParseError::Pdf {
        message: e.to_string(),
    })?;
    let array = media_box.as_array().map_err(|e| ParseError::Pdf {
        message: e.to_string(),
    })?;
    if array.len() < 4 {
        return Err(ParseError::Pdf {
            message: "MediaBox missing coordinates".to_string(),
        });
    }
    let y0 = obj_f32(&array[1])?;
    let y1 = obj_f32(&array[3])?;
    Ok(y1 - y0)
}

#[cfg(feature = "pdf")]
fn obj_f32(obj: &lopdf::Object) -> Result<f32, ParseError> {
    match obj {
        lopdf::Object::Integer(v) => Ok(*v as f32),
        lopdf::Object::Real(v) => Ok(*v as f32),
        _ => Err(ParseError::Pdf {
            message: "expected numeric PDF object".to_string(),
        }),
    }
}

#[cfg(feature = "pdf")]
fn obj_f32_safe(obj: &lopdf::Object) -> Option<f32> {
    match obj {
        lopdf::Object::Integer(v) => Some(*v as f32),
        lopdf::Object::Real(v) => Some(*v as f32),
        _ => None,
    }
}

/// Decode PDF string bytes to a Rust String.
/// Handles UTF-16BE (BOM 0xFE 0xFF) and falls back to Latin-1/PDFDocEncoding.
/// Control characters (< 0x20) are dropped — in custom-encoded PDFs they represent
/// ligature glyphs (ff, fi, fl, …) that we can't reconstruct without a ToUnicode CMap.
#[cfg(feature = "pdf")]
fn decode_pdf_string(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let utf16: Vec<u16> = bytes[2..]
            .chunks(2)
            .map(|c| {
                if c.len() == 2 {
                    u16::from_be_bytes([c[0], c[1]])
                } else {
                    0
                }
            })
            .collect();
        return String::from_utf16_lossy(&utf16)
            .chars()
            .filter(|&c| c >= ' ')
            .collect();
    }
    // PDFDocEncoding: printable range 0x20-0xFF maps 1:1 to Latin-1.
    // Bytes below 0x20 are ligatures / undefined in standard encodings — drop them.
    bytes.iter().filter(|&&b| b >= 0x20).map(|&b| b as char).collect()
}

/// Effective font height in PDF user-space points.
///
/// Many PDF generators (LaTeX, Word, Gemini) write `Tf /F1 1` (normalized size 1)
/// and encode the actual point size in the text matrix as `12 0 0 12 tx ty Tm`.
/// The true rendered height is font_size × ‖(c, d)‖ where [a,b,c,d,e,f] = tm.
#[cfg(feature = "pdf")]
fn effective_font_size_pts(font_size: f32, tm: &[f32; 6]) -> f32 {
    // Vertical scale from text matrix columns 2&3 (c, d) — handles shear/rotation too
    let vert_scale = (tm[2] * tm[2] + tm[3] * tm[3]).sqrt();
    let eff = if vert_scale > 0.01 { font_size * vert_scale } else { font_size };
    eff.max(1.0)
}

/// Walk a decoded content stream and collect text words with their positions.
///
/// Tracks the PDF text state machine: text matrix (tm), line matrix (lm),
/// current font name and size, and leading. All positions are converted from
/// PDF coordinate space (origin bottom-left, y up) to screen space (y down).
#[cfg(feature = "pdf")]
fn extract_words_from_content(
    content: &lopdf::content::Content,
    page_height: f32,
) -> Vec<PdfWord> {
    let mut words = Vec::new();
    // text matrix: [a, b, c, d, tx, ty] — current position is (tx, ty) = (tm[4], tm[5])
    let mut tm = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut lm = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut font_size = 12.0f32;
    let mut font_name: Option<String> = None;
    let mut leading = 0.0f32;
    let mut in_text = false;

    for op in &content.operations {
        match op.operator.as_str() {
            "BT" => {
                in_text = true;
                tm = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                lm = tm;
            }
            "ET" => {
                in_text = false;
            }
            "Tf" if in_text => {
                if let Some(name) = op.operands.first().and_then(|o| o.as_name_str().ok()) {
                    font_name = Some(name.to_string());
                }
                if let Some(sz) = op.operands.get(1).and_then(obj_f32_safe) {
                    font_size = sz.abs().max(1.0);
                }
            }
            "TL" => {
                if let Some(v) = op.operands.first().and_then(obj_f32_safe) {
                    leading = v;
                }
            }
            "Tm" if in_text => {
                if op.operands.len() >= 6 {
                    for (i, o) in op.operands.iter().enumerate().take(6) {
                        tm[i] = obj_f32_safe(o).unwrap_or(0.0);
                    }
                    lm = tm;
                }
            }
            "Td" | "TD" if in_text => {
                if op.operands.len() >= 2 {
                    let tx = obj_f32_safe(&op.operands[0]).unwrap_or(0.0);
                    let ty = obj_f32_safe(&op.operands[1]).unwrap_or(0.0);
                    if op.operator == "TD" {
                        leading = -ty;
                    }
                    // translate line matrix by (tx, ty)
                    lm[4] += tx * lm[0] + ty * lm[2];
                    lm[5] += tx * lm[1] + ty * lm[3];
                    tm = lm;
                }
            }
            "T*" if in_text => {
                lm[4] += (-leading) * lm[2];
                lm[5] += (-leading) * lm[3];
                tm = lm;
            }
            // show text string
            "Tj" | "'" if in_text => {
                if let Some(bytes) = op.operands.first().and_then(|o| o.as_str().ok()) {
                    let text = decode_pdf_string(bytes);
                    let eff_size = effective_font_size_pts(font_size, &tm);
                    push_word(&text, &mut tm, eff_size, &font_name, page_height, &mut words);
                }
            }
            // set word/char spacing, move to next line, show string
            "\"" if in_text => {
                if let Some(bytes) = op.operands.get(2).and_then(|o| o.as_str().ok()) {
                    let text = decode_pdf_string(bytes);
                    let eff_size = effective_font_size_pts(font_size, &tm);
                    push_word(&text, &mut tm, eff_size, &font_name, page_height, &mut words);
                }
            }
            // show text with individual glyph positioning
            "TJ" if in_text => {
                if let Some(array) = op.operands.first().and_then(|o| o.as_array().ok()) {
                    let start_x = tm[4];
                    let eff_size = effective_font_size_pts(font_size, &tm);
                    let mut text_buf = String::new();
                    let mut kern_advance = 0.0f32;
                    // Many PDFs encode word spaces as a large negative kern number.
                    // Kern is in thousandths of font_size (text space), which we scale
                    // to user space by using eff_size. Threshold: >15% of eff_size = space.
                    let space_threshold = eff_size * 0.15;

                    for element in array {
                        match element {
                            lopdf::Object::String(bytes, _) => {
                                text_buf.push_str(&decode_pdf_string(bytes));
                            }
                            lopdf::Object::Integer(k) => {
                                // PDF spec: position -= k/1000 * font_size (text space)
                                // → user-space advance = -k/1000 * eff_size
                                let advance = -(*k as f32 / 1000.0) * eff_size;
                                kern_advance += advance;
                                if advance > space_threshold && !text_buf.ends_with(' ') {
                                    text_buf.push(' ');
                                }
                            }
                            lopdf::Object::Real(k) => {
                                let advance = -(*k as f32 / 1000.0) * eff_size;
                                kern_advance += advance;
                                if advance > space_threshold && !text_buf.ends_with(' ') {
                                    text_buf.push(' ');
                                }
                            }
                            _ => {}
                        }
                    }

                    let trimmed = text_buf.trim().to_string();
                    if !trimmed.is_empty() {
                        let eff_size = effective_font_size_pts(font_size, &tm);
                        let char_width = estimate_text_width(&trimmed, eff_size);
                        let total_width = (char_width + kern_advance).max(1.0);
                        let y = tm[5];
                        words.push(PdfWord {
                            text: trimmed,
                            x0: start_x,
                            top: page_height - y - eff_size,
                            x1: start_x + total_width,
                            bottom: page_height - y,
                            font_size: eff_size,
                            font_name: font_name.clone(),
                        });
                        tm[4] = start_x + total_width;
                    }
                }
            }
            _ => {}
        }
    }

    words
}

#[cfg(feature = "pdf")]
fn push_word(
    text: &str,
    tm: &mut [f32; 6],
    font_size: f32,
    font_name: &Option<String>,
    page_height: f32,
    words: &mut Vec<PdfWord>,
) {
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return;
    }
    let x = tm[4];
    let y = tm[5];
    let width = estimate_text_width(&trimmed, font_size);
    words.push(PdfWord {
        text: trimmed,
        x0: x,
        top: page_height - y - font_size,
        x1: x + width,
        bottom: page_height - y,
        font_size,
        font_name: font_name.clone(),
    });
    tm[4] += width;
}

/// Rough proportional-font advance estimate: ~0.55 × font_size per character.
/// Sufficient for bounding-box redaction; actual glyph metrics aren't needed.
#[cfg(feature = "pdf")]
fn estimate_text_width(text: &str, font_size: f32) -> f32 {
    text.chars().count() as f32 * font_size * 0.55
}

fn group_into_lines(words: Vec<PdfWord>, page_index: usize, y_tolerance: f32) -> Vec<PdfTextLine> {
    let mut words = words;
    words.sort_by(|a, b| {
        a.top
            .partial_cmp(&b.top)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.x0.partial_cmp(&b.x0).unwrap_or(Ordering::Equal))
    });

    let mut lines: Vec<Vec<PdfWord>> = Vec::new();
    for word in words {
        match lines.last_mut() {
            Some(current) => {
                let current_top = current.first().map(|w| w.top).unwrap_or(word.top);
                if (word.top - current_top).abs() <= y_tolerance {
                    current.push(word);
                } else {
                    lines.push(vec![word]);
                }
            }
            None => lines.push(vec![word]),
        }
    }

    let mut result = Vec::with_capacity(lines.len());
    for (line_index, mut line_words) in lines.into_iter().enumerate() {
        line_words.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(Ordering::Equal));

        let text = line_words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        let x0 = line_words.iter().map(|w| w.x0).fold(f32::INFINITY, f32::min);
        let top = line_words.iter().map(|w| w.top).fold(f32::INFINITY, f32::min);
        let x1 = line_words
            .iter()
            .map(|w| w.x1)
            .fold(f32::NEG_INFINITY, f32::max);
        let bottom = line_words
            .iter()
            .map(|w| w.bottom)
            .fold(f32::NEG_INFINITY, f32::max);
        let font_size = line_words.iter().map(|w| w.font_size).fold(0.0, f32::max);
        let font_name = line_words.iter().find_map(|w| w.font_name.clone());

        result.push(PdfTextLine {
            page_index,
            line_index,
            text: fix_devanagari_clusters(&text),
            x0,
            top,
            x1,
            bottom,
            font_size,
            font_name,
        });
    }

    result
}
