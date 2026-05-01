use std::path::Path;

use tracing::info;

use crate::{config::RuntimeConfig, error::AppError, translate::TranslationService};

pub mod error;
pub mod fonts;
pub mod parser;
pub mod reconstructor;
pub mod renderer;
pub mod translator;
pub mod types;
pub mod utils;

pub use error::PdfTranslateError;
pub use types::{FontStyle, FontVariants, PdfTextLine, TranslateConfig, TranslatedLine};

pub async fn translate(
    input: &Path,
    output: &Path,
    src_lang: &str,
    tgt_lang: &str,
    service: &TranslationService,
    config: &RuntimeConfig,
) -> Result<(), AppError> {
    let translate_config = TranslateConfig {
        input_path: input.to_path_buf(),
        output_path: output.to_path_buf(),
        src_lang: src_lang.to_string(),
        tgt_lang: tgt_lang.to_string(),
        font_path: config.font_path.clone(),
        concurrency: config.concurrency,
        dpi: config.pdf_dpi,
        jpeg_quality: config.pdf_jpeg_quality,
    };

    translate_pdf(&translate_config, service)
        .await
        .map_err(|err| AppError::Pdf {
            message: err.to_string(),
        })
}

async fn translate_pdf(
    config: &TranslateConfig,
    service: &TranslationService,
) -> Result<(), PdfTranslateError> {
    if requires_complex_script_font(&config.tgt_lang) && config.font_path.is_none() {
        return Err(PdfTranslateError::ComplexScriptFontRequired {
            lang: config.tgt_lang.clone(),
        });
    }

    if let Some(path) = &config.font_path {
        if !path.exists() {
            return Err(PdfTranslateError::FontNotFound { path: path.clone() });
        }
    }

    info!("parsing PDF metadata");
    let lines = parser::parse_metadata(&config.input_path)?;

    info!(line_count = lines.len(), "translating PDF lines");
    let translated =
        translator::translate_lines(lines, &config.src_lang, &config.tgt_lang, service).await?;

    let font_variants = config
        .font_path
        .as_ref()
        .map(|path| fonts::build_font_variants(path));

    reconstructor::reconstruct_pdf(
        &config.input_path,
        &config.output_path,
        &translated,
        font_variants.as_ref(),
        config,
    )?;

    Ok(())
}

fn requires_complex_script_font(lang: &str) -> bool {
    matches!(
        lang.to_ascii_lowercase().as_str(),
        "ne" | "nep" | "nepali" | "tmg" | "tamang"
    )
}
