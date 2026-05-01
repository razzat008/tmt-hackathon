use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("missing API token; set TMT_API_TOKEN or pass --api-token")]
    MissingApiToken,

    #[error("invalid argument: {message}")]
    InvalidArgument { message: String },

    #[error("input file not found: {path}")]
    InputNotFound { path: PathBuf },

    #[error("input path is not a file: {path}")]
    InputNotFile { path: PathBuf },

    #[error("input file exceeds size limit: {size} bytes (max {max})")]
    FileTooLarge { size: u64, max: u64 },

    #[error("unsupported file format: {ext}")]
    UnsupportedFormat { ext: String },

    #[error("output format must match input format: input={input_ext}, output={output_ext}")]
    MismatchedOutputFormat {
        input_ext: String,
        output_ext: String,
    },

    #[error("request text too large: {len} bytes (max {max})")]
    RequestTooLarge { len: usize, max: usize },

    #[error("rate limited after {attempts} attempts")]
    RateLimitExceeded { attempts: u32 },

    #[error("translation failed: {message}")]
    TranslationFailed { message: String },

    #[error("network error: {message}")]
    Network { message: String },

    #[error("io error: {message}")]
    Io { message: String },

    #[error("csv error: {message}")]
    Csv { message: String },

    #[error("pdf error: {message}")]
    Pdf { message: String },

    #[error("not implemented: {feature}")]
    NotImplemented { feature: String },
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::Io {
            message: error.to_string(),
        }
    }
}

impl From<csv::Error> for AppError {
    fn from(error: csv::Error) -> Self {
        Self::Csv {
            message: error.to_string(),
        }
    }
}
