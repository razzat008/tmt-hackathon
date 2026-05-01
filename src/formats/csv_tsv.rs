use std::path::Path;

use csv::{ReaderBuilder, WriterBuilder};
use futures::stream::{self, StreamExt};
use tracing::{debug, info};

use crate::{config::RuntimeConfig, error::AppError, translate::TranslationService};

pub async fn translate(
    input: &Path,
    output: &Path,
    src_lang: &str,
    tgt_lang: &str,
    service: &TranslationService,
    _config: &RuntimeConfig,
) -> Result<(), AppError> {
    let delimiter = delimiter_for(input)?;

    let mut reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .from_path(input)?;

    let mut rows: Vec<Vec<String>> = Vec::new();
    for record in reader.records() {
        let record = record?;
        rows.push(record.iter().map(|cell| cell.to_string()).collect());
    }

    let total_rows = rows.len();
    let total_cells = rows.iter().map(|row| row.len()).sum::<usize>();
    let non_empty_cells = rows
        .iter()
        .flatten()
        .filter(|cell| !cell.trim().is_empty())
        .count();

    info!(
        rows = total_rows,
        cells = total_cells,
        non_empty_cells,
        "loaded CSV/TSV input"
    );

    let mut translated_rows = rows.clone();

    let src_lang = src_lang.to_string();
    let tgt_lang = tgt_lang.to_string();

    let jobs: Vec<(usize, usize, String)> = rows
        .iter()
        .enumerate()
        .flat_map(|(row_idx, row)| {
            row.iter()
                .enumerate()
                .filter(|(_, cell)| !cell.trim().is_empty())
                .map(move |(col_idx, cell)| (row_idx, col_idx, cell.clone()))
        })
        .collect();

    let results: Vec<(usize, usize, Result<String, AppError>)> = stream::iter(jobs)
        .map(|(row_idx, col_idx, cell)| {
            let service = service.clone();
            let src_lang = src_lang.clone();
            let tgt_lang = tgt_lang.clone();
            async move {
                let translated = service.translate_text(&cell, &src_lang, &tgt_lang).await;
                (row_idx, col_idx, translated)
            }
        })
        .buffer_unordered(1)
        .collect()
        .await;

    let mut first_error: Option<AppError> = None;
    for (row_idx, col_idx, result) in results {
        match result {
            Ok(text) => translated_rows[row_idx][col_idx] = text,
            Err(err) => {
                debug!(row = row_idx, col = col_idx, "cell translation failed");
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
    }

    if let Some(err) = first_error {
        return Err(err);
    }

    let mut writer = WriterBuilder::new()
        .delimiter(delimiter)
        .from_path(output)?;

    for row in translated_rows {
        writer.write_record(&row)?;
    }

    writer.flush()?;

    Ok(())
}

fn delimiter_for(path: &Path) -> Result<u8, AppError> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .ok_or_else(|| AppError::InvalidArgument {
            message: format!("file has no extension: {}", path.display()),
        })?;

    Ok(match ext.as_str() {
        "tsv" => b'\t',
        _ => b',',
    })
}
