use std::borrow::Cow;
use std::path::Path;

use docx_rust::document::{
    BodyContent, ParagraphContent, RunContent, TableCellContent, TableRowContent, TextSpace,
};
use docx_rust::DocxFile;
use futures::stream::{self, StreamExt};
use tracing::info;

use crate::{config::RuntimeConfig, error::AppError, translate::TranslationService};

// ── paragraph collection ──────────────────────────────────────────────────────

struct ParaRecord {
    text: String,
    run_originals: Vec<String>,
}

fn collect_paragraph(para: &docx_rust::document::Paragraph) -> ParaRecord {
    let mut run_originals: Vec<String> = Vec::new();
    for pc in &para.content {
        if let ParagraphContent::Run(run) = pc {
            let text: String = run
                .content
                .iter()
                .filter_map(|rc| {
                    if let RunContent::Text(t) = rc {
                        Some(t.text.as_ref().to_string())
                    } else {
                        None
                    }
                })
                .collect();
            run_originals.push(text);
        }
    }
    let text = run_originals.concat();
    ParaRecord { text, run_originals }
}

fn collect_all(body: &docx_rust::document::Body) -> Vec<ParaRecord> {
    let mut records = Vec::new();
    for bc in &body.content {
        match bc {
            BodyContent::Paragraph(p) => records.push(collect_paragraph(p)),
            BodyContent::Table(t) => {
                for row in &t.rows {
                    for cell_content in &row.cells {
                        if let TableRowContent::TableCell(cell) = cell_content {
                            for tcc in &cell.content {
                                let TableCellContent::Paragraph(p) = tcc;
                                records.push(collect_paragraph(p));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    records
}

// ── translation distribution ──────────────────────────────────────────────────

/// Distribute `translated` across `run_originals` proportionally by original
/// character count.  The last run absorbs any rounding remainder.
fn distribute(translated: &str, run_originals: &[String]) -> Vec<String> {
    if run_originals.is_empty() {
        return vec![];
    }
    if run_originals.len() == 1 {
        return vec![translated.to_string()];
    }
    let orig_lens: Vec<usize> = run_originals.iter().map(|s| s.chars().count()).collect();
    let total_orig: usize = orig_lens.iter().sum();
    let t_chars: Vec<char> = translated.chars().collect();
    let total_t = t_chars.len();

    if total_orig == 0 {
        return vec![String::new(); run_originals.len()];
    }

    let mut result = Vec::with_capacity(run_originals.len());
    let mut offset = 0usize;

    for (idx, &len) in orig_lens.iter().enumerate() {
        let portion = if idx == orig_lens.len() - 1 {
            total_t.saturating_sub(offset)
        } else {
            let frac = len as f64 / total_orig as f64;
            (frac * total_t as f64).round() as usize
        };
        let part: String = t_chars.iter().skip(offset).take(portion).collect();
        offset += portion;
        result.push(part);
    }
    result
}

// ── mutation ──────────────────────────────────────────────────────────────────

fn apply_to_paragraph(
    para: &mut docx_rust::document::Paragraph,
    translated: &str,
    run_originals: &[String],
) {
    if run_originals.is_empty() {
        return;
    }
    let distributed = distribute(translated, run_originals);
    let mut run_idx = 0usize;

    for pc in para.content.iter_mut() {
        if let ParagraphContent::Run(run) = pc {
            if run_idx >= distributed.len() {
                break;
            }
            let new_text = &distributed[run_idx];
            let orig_empty = run_originals[run_idx].is_empty();

            if !orig_empty {
                for rc in run.content.iter_mut() {
                    if let RunContent::Text(t) = rc {
                        t.text = Cow::Owned(new_text.clone());
                        t.space = Some(TextSpace::Preserve);
                        break;
                    }
                }
            }
            run_idx += 1;
        }
    }
}

fn apply_all(
    body: &mut docx_rust::document::Body,
    records: &[ParaRecord],
    translations: &[Option<String>],
    idx: &mut usize,
) {
    for bc in body.content.iter_mut() {
        match bc {
            BodyContent::Paragraph(p) => {
                if let Some(Some(translated)) = translations.get(*idx) {
                    apply_to_paragraph(p, translated, &records[*idx].run_originals);
                }
                *idx += 1;
            }
            BodyContent::Table(t) => {
                for row in t.rows.iter_mut() {
                    for cell_content in row.cells.iter_mut() {
                        if let TableRowContent::TableCell(cell) = cell_content {
                            for tcc in cell.content.iter_mut() {
                                let TableCellContent::Paragraph(p) = tcc;
                                if let Some(Some(translated)) = translations.get(*idx) {
                                    apply_to_paragraph(
                                        p,
                                        translated,
                                        &records[*idx].run_originals,
                                    );
                                }
                                *idx += 1;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

// ── public entry point ────────────────────────────────────────────────────────

pub async fn translate(
    input: &Path,
    output: &Path,
    src_lang: &str,
    tgt_lang: &str,
    service: &TranslationService,
    config: &RuntimeConfig,
) -> Result<(), AppError> {
    // Step 1 — load.  docx_file must stay alive until write_file completes.
    let docx_file = DocxFile::from_file(input)
        .map_err(|e| AppError::Docx { message: e.to_string() })?;
    let mut docx = docx_file
        .parse()
        .map_err(|e| AppError::Docx { message: e.to_string() })?;

    // Step 2 — collect paragraph text and per-run metadata.
    let records = collect_all(&docx.document.body);
    info!(count = records.len(), "collected paragraphs from docx");

    // Step 3 — translate concurrently.
    let src = src_lang.to_string();
    let tgt = tgt_lang.to_string();

    let jobs: Vec<(usize, String)> = records
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.text.trim().is_empty())
        .map(|(i, r)| (i, r.text.clone()))
        .collect();

    let results: Vec<(usize, Result<String, AppError>)> = stream::iter(jobs)
        .map(|(para_idx, text)| {
            let svc = service.clone();
            let s = src.clone();
            let t = tgt.clone();
            async move {
                let r = svc.translate_text(&text, &s, &t).await;
                (para_idx, r)
            }
        })
        .buffer_unordered(config.concurrency)
        .collect()
        .await;

    let mut translations: Vec<Option<String>> = vec![None; records.len()];
    for (i, result) in results {
        match result {
            Ok(t) => translations[i] = Some(t),
            Err(e) => return Err(e),
        }
    }

    // Step 4 — write translated text back into the document structure.
    let mut para_idx = 0usize;
    apply_all(
        &mut docx.document.body,
        &records,
        &translations,
        &mut para_idx,
    );

    // Step 5 — save.
    docx.write_file(output)
        .map_err(|e| AppError::Docx { message: e.to_string() })?;

    Ok(())
}
