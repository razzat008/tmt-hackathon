use std::fs;
use std::path::{Path, PathBuf};

use crate::{
    cli::Cli,
    config::{MAX_FILE_SIZE_BYTES, RuntimeConfig},
    error::AppError,
    formats,
};
use tracing::{debug, info};

pub async fn run(cli: Cli) -> Result<(), AppError> {
    let config = RuntimeConfig::try_from(&cli)?;

    info!(
        input = %cli.input.display(),
        output = %cli.output.display(),
        src_lang = %cli.src_lang,
        tgt_lang = %cli.tgt_lang,
        "starting translation"
    );

    validate_input_file(&cli.input)?;
    validate_output_format(&cli.input, &cli.output)?;
    ensure_output_parent(&cli.output)?;

    debug!(
        concurrency = config.concurrency,
        rate_limit_ms = ?config.rate_limit_ms,
        max_retries = config.max_retries,
        "runtime configuration"
    );

    formats::translate_file(
        &cli.input,
        &cli.output,
        &cli.src_lang,
        &cli.tgt_lang,
        &config,
    )
    .await
}

fn validate_input_file(path: &Path) -> Result<(), AppError> {
    if !path.exists() {
        return Err(AppError::InputNotFound {
            path: path.to_path_buf(),
        });
    }

    if !path.is_file() {
        return Err(AppError::InputNotFile {
            path: path.to_path_buf(),
        });
    }

    let size = fs::metadata(path)?.len();
    if size > MAX_FILE_SIZE_BYTES {
        return Err(AppError::FileTooLarge {
            size,
            max: MAX_FILE_SIZE_BYTES,
        });
    }

    Ok(())
}

fn validate_output_format(input: &Path, output: &Path) -> Result<(), AppError> {
    let input_ext = extension(input)?;
    let output_ext = extension(output)?;

    if input_ext != output_ext {
        return Err(AppError::MismatchedOutputFormat {
            input_ext,
            output_ext,
        });
    }

    Ok(())
}

fn ensure_output_parent(path: &Path) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    Ok(())
}

fn extension(path: &Path) -> Result<String, AppError> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("file has no extension: {}", path.display()),
        })
}

#[allow(dead_code)]
fn normalize_output_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}
