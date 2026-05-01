use tokio::task::JoinSet;
use tracing::debug;

use crate::{
    formats::pdf::{
        PdfTextLine, TranslatedLine, error::TranslationError, utils::fix_devanagari_clusters,
    },
    translate::TranslationService,
};

pub async fn translate_lines(
    lines: Vec<PdfTextLine>,
    src_lang: &str,
    tgt_lang: &str,
    service: &TranslationService,
) -> Result<Vec<TranslatedLine>, TranslationError> {
    let mut join_set = JoinSet::new();
    let src_lang = src_lang.to_string();
    let tgt_lang = tgt_lang.to_string();

    for (idx, line) in lines.into_iter().enumerate() {
        let service = service.clone();
        let src_lang = src_lang.clone();
        let tgt_lang = tgt_lang.clone();

        join_set.spawn(async move {
            let translated = service
                .translate_text(&line.text, &src_lang, &tgt_lang)
                .await
                .map_err(|err| TranslationError::Service {
                    message: err.to_string(),
                })?;
            Ok::<_, TranslationError>((
                idx,
                TranslatedLine {
                    source: line,
                    translated_text: fix_devanagari_clusters(&translated),
                },
            ))
        });
    }

    let mut ordered: Vec<Option<TranslatedLine>> = Vec::new();
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok((idx, line))) => {
                if ordered.len() <= idx {
                    ordered.resize_with(idx + 1, || None);
                }
                ordered[idx] = Some(line);
            }
            Ok(Err(err)) => return Err(err),
            Err(join_err) => {
                debug!(error = %join_err, "pdf translate task failed to join");
                return Err(TranslationError::Service {
                    message: join_err.to_string(),
                });
            }
        }
    }

    Ok(ordered.into_iter().flatten().collect())
}
