use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct PdfTextLine {
    pub page_index: usize,
    pub line_index: usize,
    pub text: String,
    pub x0: f32,
    pub top: f32,
    pub x1: f32,
    pub bottom: f32,
    pub font_size: f32,
    pub font_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TranslatedLine {
    pub source: PdfTextLine,
    pub translated_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontStyle {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

#[derive(Debug, Clone)]
pub struct FontVariants {
    pub regular: PathBuf,
    pub bold: PathBuf,
    pub italic: PathBuf,
    pub bold_italic: PathBuf,
}

#[derive(Debug, Clone)]
pub struct TranslateConfig {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub src_lang: String,
    pub tgt_lang: String,
    pub font_path: Option<PathBuf>,
    pub concurrency: usize,
    pub dpi: u32,
    pub jpeg_quality: u8,
}
