use std::path::Path;

#[cfg(feature = "pdf")]
use std::collections::BTreeMap;

use crate::formats::pdf::{FontVariants, TranslateConfig, TranslatedLine, error::ReconstructError};

#[cfg(feature = "pdf")]
use crate::formats::pdf::{
    error::RenderError, fonts::pick_variant, renderer, utils::escape_pdf_string,
};

#[cfg(feature = "pdf")]
pub fn reconstruct_pdf(
    input_path: &Path,
    output_path: &Path,
    translated_lines: &[TranslatedLine],
    font_variants: Option<&FontVariants>,
    config: &TranslateConfig,
) -> Result<(), ReconstructError> {
    use lopdf::Document;

    if translated_lines.is_empty() {
        std::fs::copy(input_path, output_path)?;
        return Ok(());
    }

    let mut doc = Document::load(input_path).map_err(|err| ReconstructError::Pdf {
        message: err.to_string(),
    })?;

    let pages = doc.get_pages();
    let by_page = group_by_page(translated_lines);

    for (page_index, lines) in by_page {
        let page_id = pages
            .values()
            .nth(page_index)
            .ok_or(ReconstructError::PageNotFound { page_index })?;

        let (_, page_height) = page_dimensions(&doc, *page_id)?;

        redact_regions(&mut doc, *page_id, &lines, page_height)?;

        for line in lines {
            let rect = source_rect(&line.source, page_height);
            if rect.width() <= 0.0 || rect.height() <= 0.0 {
                continue;
            }

            let font_path = match font_variants {
                Some(variants) => pick_variant(variants, line.source.font_name.as_deref()),
                None => config.font_path.clone().ok_or_else(|| {
                    ReconstructError::Render(RenderError::Font {
                        message: "font path required for PDF rendering".to_string(),
                    })
                })?,
            };

            let font_data = std::fs::read(&font_path)?;
            let upem = font_upem(&font_data)?;

            let script = script_for_lang(&config.tgt_lang);
            let glyphs =
                renderer::shape_text(&font_data, &line.translated_text, script, &config.tgt_lang)?;

            let scale = config.dpi as f32 / 72.0;
            let width_px = (rect.width() * scale).ceil().max(1.0) as u32;
            let height_px = (rect.height() * scale).ceil().max(1.0) as u32;
            let font_size_px = (line.source.font_size * scale).ceil().max(1.0) as u32;

            let img = renderer::render_glyphs(
                &font_path,
                &glyphs,
                font_size_px,
                upem,
                width_px,
                height_px,
            )?;
            let jpeg = renderer::to_jpeg(img, config.jpeg_quality)?;

            insert_image(&mut doc, *page_id, &rect, width_px, height_px, &jpeg)?;

            insert_invisible_text(
                &mut doc,
                *page_id,
                &rect,
                &line.translated_text,
                line.source.font_size,
            )?;
        }
    }

    doc.save(output_path).map_err(|err| ReconstructError::Pdf {
        message: err.to_string(),
    })?;

    Ok(())
}

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

#[cfg(feature = "pdf")]
fn group_by_page(lines: &[TranslatedLine]) -> BTreeMap<usize, Vec<&TranslatedLine>> {
    let mut map: BTreeMap<usize, Vec<&TranslatedLine>> = BTreeMap::new();
    for line in lines {
        map.entry(line.source.page_index).or_default().push(line);
    }
    map
}

#[cfg(feature = "pdf")]
#[derive(Debug, Clone, Copy)]
struct Rect {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

#[cfg(feature = "pdf")]
impl Rect {
    fn width(self) -> f32 {
        (self.x1 - self.x0).max(0.0)
    }

    fn height(self) -> f32 {
        (self.y1 - self.y0).max(0.0)
    }
}

#[cfg(feature = "pdf")]
fn source_rect(line: &crate::formats::pdf::PdfTextLine, page_height: f32) -> Rect {
    Rect {
        x0: line.x0,
        y0: page_height - line.bottom,
        x1: line.x1,
        y1: page_height - line.top,
    }
}

#[cfg(feature = "pdf")]
fn page_dimensions(
    doc: &lopdf::Document,
    page_id: lopdf::ObjectId,
) -> Result<(f32, f32), ReconstructError> {
    let page = doc
        .get_object(page_id)
        .map_err(|err| ReconstructError::Pdf {
            message: err.to_string(),
        })?;
    let dict = page.as_dict().map_err(|err| ReconstructError::Pdf {
        message: err.to_string(),
    })?;
    let media_box = dict.get(b"MediaBox").map_err(|err| ReconstructError::Pdf {
        message: err.to_string(),
    })?;

    let array = media_box.as_array().map_err(|err| ReconstructError::Pdf {
        message: err.to_string(),
    })?;
    if array.len() < 4 {
        return Err(ReconstructError::Pdf {
            message: "MediaBox missing coordinates".to_string(),
        });
    }

    let x0 = object_to_f32(&array[0])?;
    let y0 = object_to_f32(&array[1])?;
    let x1 = object_to_f32(&array[2])?;
    let y1 = object_to_f32(&array[3])?;

    Ok((x1 - x0, y1 - y0))
}

#[cfg(feature = "pdf")]
fn object_to_f32(obj: &lopdf::Object) -> Result<f32, ReconstructError> {
    match obj {
        lopdf::Object::Integer(value) => Ok(*value as f32),
        lopdf::Object::Real(value) => Ok(*value as f32),
        _ => Err(ReconstructError::Pdf {
            message: "expected numeric MediaBox entry".to_string(),
        }),
    }
}

#[cfg(feature = "pdf")]
fn redact_regions(
    doc: &mut lopdf::Document,
    page_id: lopdf::ObjectId,
    lines: &[&TranslatedLine],
    page_height: f32,
) -> Result<(), ReconstructError> {
    let mut content = String::new();
    for line in lines {
        let rect = source_rect(&line.source, page_height);
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            continue;
        }
        content.push_str(&format!(
            "q\n1 1 1 rg\n{:.2} {:.2} {:.2} {:.2} re\nf\nQ\n",
            rect.x0,
            rect.y0,
            rect.width(),
            rect.height()
        ));
    }

    append_to_page_content(doc, page_id, content.as_bytes())
}

#[cfg(feature = "pdf")]
fn insert_image(
    doc: &mut lopdf::Document,
    page_id: lopdf::ObjectId,
    rect: &Rect,
    width_px: u32,
    height_px: u32,
    jpeg: &[u8],
) -> Result<(), ReconstructError> {
    use lopdf::{Dictionary, Object, Stream};

    let mut dict = Dictionary::new();
    dict.set("Type", "XObject");
    dict.set("Subtype", "Image");
    dict.set("Width", width_px as i64);
    dict.set("Height", height_px as i64);
    dict.set("ColorSpace", "DeviceRGB");
    dict.set("BitsPerComponent", 8);
    dict.set("Filter", "DCTDecode");

    let stream = Stream::new(dict, jpeg.to_vec());
    let image_id = doc.add_object(stream);
    let image_name = format!("Im{}", image_id.0);

    let page = doc
        .get_object_mut(page_id)
        .map_err(|err| ReconstructError::Pdf {
            message: err.to_string(),
        })?;
    let page_dict = page.as_dict_mut().map_err(|err| ReconstructError::Pdf {
        message: err.to_string(),
    })?;

    let resources = get_or_create_dict(page_dict, b"Resources")?;
    let xobjects = get_or_create_dict(resources, b"XObject")?;
    xobjects.set(image_name.as_bytes(), Object::Reference(image_id));

    let content = format!(
        "q\n{:.2} 0 0 {:.2} {:.2} {:.2} cm\n/{} Do\nQ\n",
        rect.width(),
        rect.height(),
        rect.x0,
        rect.y0,
        image_name
    );
    append_to_page_content(doc, page_id, content.as_bytes())
}

#[cfg(feature = "pdf")]
fn insert_invisible_text(
    doc: &mut lopdf::Document,
    page_id: lopdf::ObjectId,
    rect: &Rect,
    text: &str,
    font_size: f32,
) -> Result<(), ReconstructError> {
    // Check if F1 font needs to be added (shared borrow, no conflict)
    let need_font = {
        let page = doc
            .get_object(page_id)
            .map_err(|err| ReconstructError::Pdf {
                message: err.to_string(),
            })?;
        let page_dict = page.as_dict().map_err(|err| ReconstructError::Pdf {
            message: err.to_string(),
        })?;
        fonts_need_f1(page_dict)
    };

    if need_font {
        let mut font_dict = lopdf::Dictionary::new();
        font_dict.set("Type", "Font");
        font_dict.set("Subtype", "Type1");
        font_dict.set("BaseFont", "Helvetica");

        // add_object takes &mut doc but doesn't touch page_id's dict,
        // so we can do this before re-borrowing page_dict.
        let font_id = doc.add_object(lopdf::Object::Dictionary(font_dict));

        let page = doc
            .get_object_mut(page_id)
            .map_err(|err| ReconstructError::Pdf {
                message: err.to_string(),
            })?;
        let page_dict = page.as_dict_mut().map_err(|err| ReconstructError::Pdf {
            message: err.to_string(),
        })?;
        let resources = get_or_create_dict(page_dict, b"Resources")?;
        let fonts = get_or_create_dict(resources, b"Font")?;
        fonts.set("F1", lopdf::Object::Reference(font_id));
    }

    let content = format!(
        "BT\n3 Tr\n/F1 {size} Tf\n{x:.2} {y:.2} Td\n({text}) Tj\nET\n",
        x = rect.x0,
        y = rect.y0,
        size = font_size,
        text = escape_pdf_string(text),
    );
    append_to_page_content(doc, page_id, content.as_bytes())
}

#[cfg(feature = "pdf")]
fn ensure_builtin_font(
    doc: &mut lopdf::Document,
    page_dict: &mut lopdf::Dictionary,
) -> Result<(), ReconstructError> {
    use lopdf::{Dictionary, Object};

    let resources = get_or_create_dict(page_dict, b"Resources")?;
    let fonts = get_or_create_dict(resources, b"Font")?;

    if fonts.get(b"F1").is_err() {
        let mut font_dict = Dictionary::new();
        font_dict.set("Type", "Font");
        font_dict.set("Subtype", "Type1");
        font_dict.set("BaseFont", "Helvetica");

        let font_id = doc.add_object(Object::Dictionary(font_dict));
        fonts.set("F1", Object::Reference(font_id));
    }

    Ok(())
}

#[cfg(feature = "pdf")]
fn append_to_page_content(
    doc: &mut lopdf::Document,
    page_id: lopdf::ObjectId,
    content: &[u8],
) -> Result<(), ReconstructError> {
    use lopdf::{Dictionary, Object, Stream};

    let stream = Stream::new(Dictionary::new(), content.to_vec());
    let stream_id = doc.add_object(stream);

    let page = doc
        .get_object_mut(page_id)
        .map_err(|err| ReconstructError::Pdf {
            message: err.to_string(),
        })?;
    let page_dict = page.as_dict_mut().map_err(|err| ReconstructError::Pdf {
        message: err.to_string(),
    })?;

    // get_mut returns Result<&mut Object, Error>, so match Ok variants
    match page_dict.get_mut(b"Contents") {
        Ok(Object::Reference(reference)) => {
            // (u32, u16) is Copy; no deref needed, just copy it
            let old_ref = *reference;
            let mut array = Vec::new();
            array.push(Object::Reference(old_ref));
            array.push(Object::Reference(stream_id));
            page_dict.set("Contents", Object::Array(array));
        }
        Ok(Object::Array(items)) => {
            items.push(Object::Reference(stream_id));
        }
        Ok(Object::Stream(_)) => {
            // get() also returns Result; map to Option for the ok() chain
            let existing_stream = page_dict
                .get(b"Contents")
                .ok() // Result -> Option
                .and_then(|obj| obj.as_stream().ok()) // Option<Object> -> Option<&Stream>
                .ok_or_else(|| ReconstructError::Pdf {
                    message: "invalid Contents stream".to_string(),
                })?
                .clone();
            let existing_id = doc.add_object(Object::Stream(existing_stream));
            let page = doc
                .get_object_mut(page_id)
                .map_err(|err| ReconstructError::Pdf {
                    message: err.to_string(),
                })?;
            let page_dict = page.as_dict_mut().map_err(|err| ReconstructError::Pdf {
                message: err.to_string(),
            })?;
            page_dict.set(
                "Contents",
                Object::Array(vec![
                    Object::Reference(existing_id),
                    Object::Reference(stream_id),
                ]),
            );
        }
        _ => {
            page_dict.set("Contents", Object::Reference(stream_id));
        }
    }

    Ok(())
}

#[cfg(feature = "pdf")]
fn get_or_create_dict<'a>(
    dict: &'a mut lopdf::Dictionary,
    key: &[u8],
) -> Result<&'a mut lopdf::Dictionary, ReconstructError> {
    use lopdf::{Dictionary, Object};

    if dict.get(key).is_err() {
        dict.set(key, Object::Dictionary(Dictionary::new()));
    }

    dict.get_mut(key)
        .map_err(|err| ReconstructError::Pdf {
            message: err.to_string(),
        })?
        .as_dict_mut()
        .map_err(|err| ReconstructError::Pdf {
            message: err.to_string(),
        })
}

#[cfg(feature = "pdf")]
fn font_upem(font_data: &[u8]) -> Result<i32, ReconstructError> {
    use rustybuzz::Face;
    let face = Face::from_slice(font_data, 0).ok_or_else(|| {
        ReconstructError::Render(RenderError::Font {
            message: "invalid font data".to_string(),
        })
    })?;
    let upem = face.units_per_em() as i32;
    if upem <= 0 {
        return Err(ReconstructError::Render(RenderError::Font {
            message: "font has invalid units-per-em".to_string(),
        }));
    }
    Ok(upem)
}

#[cfg(feature = "pdf")]
fn script_for_lang(lang: &str) -> &'static str {
    match lang.to_ascii_lowercase().as_str() {
        "ne" | "nep" | "nepali" | "tmg" | "tamang" => "Deva",
        _ => "Latn",
    }
}

/// Returns true if the page's Font dict is missing F1 (or Resources/Font don't exist yet).
/// Takes only a shared ref so it doesn't conflict with the subsequent mutable borrows.
#[cfg(feature = "pdf")]
fn fonts_need_f1(page_dict: &lopdf::Dictionary) -> bool {
    page_dict
        .get(b"Resources")
        .ok()
        .and_then(|r| r.as_dict().ok())
        .and_then(|r| r.get(b"Font").ok())
        .and_then(|f| f.as_dict().ok())
        .map(|fonts| fonts.get(b"F1").is_err())
        .unwrap_or(true)
}
