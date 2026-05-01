use std::path::Path;

use crate::{
    config::RuntimeConfig, error::AppError, tmt::TmtClient, translate::TranslationService,
};
use tracing::info;

pub mod csv_tsv;
pub mod docx;
pub mod pdf;

pub async fn translate_file(
    input: &Path,
    output: &Path,
    src_lang: &str,
    tgt_lang: &str,
    config: &RuntimeConfig,
) -> Result<(), AppError> {
    let ext = extension(input)?;

    info!(format = %ext, "dispatching format handler");

    let client = TmtClient::new(&config.base_url, &config.api_token);
    let service = TranslationService::new(client, config)?;

    match ext.as_str() {
        "csv" | "tsv" => {
            csv_tsv::translate(input, output, src_lang, tgt_lang, &service, config).await
        }
        "docx" => docx::translate(input, output, src_lang, tgt_lang, &service, config).await,
        "pdf" => pdf::translate(input, output, src_lang, tgt_lang, &service, config).await,
        _ => Err(AppError::UnsupportedFormat { ext }),
    }
}

fn extension(path: &Path) -> Result<String, AppError> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("file has no extension: {}", path.display()),
        })
}
