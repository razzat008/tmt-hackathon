use std::path::Path;

use crate::{config::RuntimeConfig, error::AppError, translate::TranslationService};

pub async fn translate(
    _input: &Path,
    _output: &Path,
    _src_lang: &str,
    _tgt_lang: &str,
    _service: &TranslationService,
    _config: &RuntimeConfig,
) -> Result<(), AppError> {
    Err(AppError::NotImplemented {
        feature: "DOCX translation".to_string(),
    })
}
