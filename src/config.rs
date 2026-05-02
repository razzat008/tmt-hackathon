use std::env;
use std::path::PathBuf;

use crate::{cli::Cli, error::AppError};

pub const DEFAULT_BASE_URL: &str = "https://tmt.ilprl.ku.edu.np/lang-translate";
pub const MAX_FILE_SIZE_BYTES: u64 = 1_000_000;
pub const MAX_REQUEST_TEXT_BYTES: usize = 5_000;

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub base_url: String,
    pub api_token: String,
    pub concurrency: usize,
    pub rate_limit_ms: Option<u64>,
    pub max_retries: u32,
    pub font_path: Option<PathBuf>,
    pub pdf_dpi: u32,
    pub pdf_jpeg_quality: u8,
    pub debug_bboxes: bool,
}

impl TryFrom<&Cli> for RuntimeConfig {
    type Error = AppError;

    fn try_from(cli: &Cli) -> Result<Self, Self::Error> {
        let token = match cli
            .api_token
            .clone()
            .or_else(|| env::var("TMT_API_TOKEN").ok())
        {
            Some(token) if !token.trim().is_empty() => token,
            _ => return Err(AppError::MissingApiToken),
        };

        if cli.concurrency == 0 {
            return Err(AppError::InvalidArgument {
                message: "concurrency must be >= 1".to_string(),
            });
        }

        if cli.max_retries == 0 {
            return Err(AppError::InvalidArgument {
                message: "max-retries must be >= 1".to_string(),
            });
        }

        if cli.src_lang.eq_ignore_ascii_case(&cli.tgt_lang) {
            return Err(AppError::InvalidArgument {
                message: "src_lang and tgt_lang must differ".to_string(),
            });
        }

        if cli.dpi == 0 {
            return Err(AppError::InvalidArgument {
                message: "dpi must be >= 1".to_string(),
            });
        }

        if !(1..=100).contains(&cli.jpeg_quality) {
            return Err(AppError::InvalidArgument {
                message: "jpeg-quality must be between 1 and 100".to_string(),
            });
        }

        if let Some(path) = &cli.font_path {
            if !path.exists() {
                return Err(AppError::InvalidArgument {
                    message: format!("font path does not exist: {}", path.display()),
                });
            }
            if !path.is_file() {
                return Err(AppError::InvalidArgument {
                    message: format!("font path is not a file: {}", path.display()),
                });
            }
        }

        Ok(Self {
            base_url: cli.base_url.clone(),
            api_token: token,
            concurrency: cli.concurrency,
            rate_limit_ms: cli.rate_limit_ms,
            max_retries: cli.max_retries,
            font_path: cli.font_path.clone(),
            pdf_dpi: cli.dpi,
            pdf_jpeg_quality: cli.jpeg_quality,
            debug_bboxes: cli.debug_bboxes,
        })
    }
}
