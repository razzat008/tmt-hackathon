use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("pdf parsing is disabled; build with --features pdf")]
    FeatureDisabled,

    #[error("pdf parse error: {message}")]
    Pdf { message: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum TranslationError {
    #[error("translation error: {message}")]
    Service { message: String },
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("pdf rendering is disabled; build with --features pdf")]
    FeatureDisabled,

    #[error("font error: {message}")]
    Font { message: String },

    #[error("image error: {message}")]
    Image { message: String },
}

#[derive(Debug, Error)]
pub enum ReconstructError {
    #[error("pdf reconstruction is disabled; build with --features pdf")]
    FeatureDisabled,

    #[error("pdf reconstruct error: {message}")]
    Pdf { message: String },

    #[error("page not found: {page_index}")]
    PageNotFound { page_index: usize },

    #[error("render error: {0}")]
    Render(#[from] RenderError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum PdfTranslateError {
    #[error("Parse error: {0}")]
    Parse(#[from] ParseError),

    #[error("Translation error: {0}")]
    Translation(#[from] TranslationError),

    #[error("Reconstruction error: {0}")]
    Reconstruction(#[from] ReconstructError),

    #[error("Font not found: {path}. Install via: sudo apt install fonts-noto-core")]
    FontNotFound { path: PathBuf },

    #[error("Language '{lang}' requires a complex-script font (--font-path)")]
    ComplexScriptFontRequired { lang: String },
}
