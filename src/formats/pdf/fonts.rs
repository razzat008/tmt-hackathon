use std::path::{Path, PathBuf};

use crate::formats::pdf::FontVariants;

pub fn build_font_variants(base: &Path) -> FontVariants {
    let parent = base.parent().unwrap_or_else(|| Path::new(""));
    let stem = base
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let ext = base.extension().and_then(|s| s.to_str()).unwrap_or("");

    let base_name = stem.rsplit_once('-').map(|(b, _)| b).unwrap_or(stem);

    let resolve = |weight: &str| -> PathBuf {
        let candidate = if ext.is_empty() {
            parent.join(format!("{base_name}-{weight}"))
        } else {
            parent.join(format!("{base_name}-{weight}.{ext}"))
        };
        if candidate.exists() {
            candidate
        } else {
            base.to_path_buf()
        }
    };

    FontVariants {
        regular: base.to_path_buf(),
        bold: resolve("Bold"),
        italic: base.to_path_buf(),
        bold_italic: resolve("Bold"),
    }
}

pub fn pick_variant(variants: &FontVariants, font_name: Option<&str>) -> PathBuf {
    let Some(name) = font_name else {
        return variants.regular.clone();
    };
    let name = name.split('+').last().unwrap_or(name).to_lowercase();

    let is_bold = ["bold", "heavy", "black", "demi", "semibold", "medium"]
        .iter()
        .any(|w| name.contains(w));
    let is_italic = ["italic", "oblique", "slant"]
        .iter()
        .any(|w| name.contains(w));

    match (is_bold, is_italic) {
        (true, true) => variants.bold_italic.clone(),
        (true, false) => variants.bold.clone(),
        (false, true) => variants.italic.clone(),
        _ => variants.regular.clone(),
    }
}
