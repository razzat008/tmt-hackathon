use std::cmp::Ordering;
use std::path::Path;

use crate::formats::pdf::{PdfTextLine, error::ParseError, utils::fix_devanagari_clusters};

#[allow(dead_code)]
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

#[cfg(feature = "pdf")]
pub fn parse_metadata(_path: &Path) -> Result<Vec<PdfTextLine>, ParseError> {
    Err(ParseError::Pdf {
        message: "pdfium extraction not yet implemented; see parser.rs".to_string(),
    })
}

#[cfg(not(feature = "pdf"))]
pub fn parse_metadata(_path: &Path) -> Result<Vec<PdfTextLine>, ParseError> {
    Err(ParseError::FeatureDisabled)
}

#[allow(dead_code)]
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

        let x0 = line_words
            .iter()
            .map(|w| w.x0)
            .fold(f32::INFINITY, f32::min);
        let top = line_words
            .iter()
            .map(|w| w.top)
            .fold(f32::INFINITY, f32::min);
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

#[allow(dead_code)]
const Y_TOLERANCE: f32 = 3.0;
#[allow(dead_code)]
const X_TOLERANCE: f32 = 2.0;
