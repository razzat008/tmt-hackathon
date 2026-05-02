use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about = "TMT file translation tool")]
pub struct Cli {
    /// Input file path (.pdf, .docx, .csv, .tsv)
    #[arg(short, long)]
    pub input: PathBuf,

    /// Output file path (must match input format)
    #[arg(short, long)]
    pub output: PathBuf,

    /// Source language code (en/ne/tmg)
    #[arg(short = 's', long)]
    pub src_lang: String,

    /// Target language code (en/ne/tmg)
    #[arg(short = 't', long)]
    pub tgt_lang: String,

    /// API base URL
    #[arg(long, default_value = crate::config::DEFAULT_BASE_URL)]
    pub base_url: String,

    /// API token (overrides TMT_API_TOKEN env var)
    #[arg(long)]
    pub api_token: Option<String>,

    /// Max concurrent in-flight requests
    #[arg(long, default_value_t = 2)]
    pub concurrency: usize,

    /// Optional delay (ms) between requests
    #[arg(long)]
    pub rate_limit_ms: Option<u64>,

    /// Max retries for non-rate-limit failures per sentence
    #[arg(long, default_value_t = 4)]
    pub max_retries: u32,

    /// PDF font path for complex scripts (required for PDF output)
    #[arg(long)]
    pub font_path: Option<PathBuf>,

    /// PDF render DPI (default 96)
    #[arg(long, default_value_t = 96)]
    pub dpi: u32,

    /// PDF JPEG quality (1-100, default 85)
    #[arg(long, default_value_t = 85)]
    pub jpeg_quality: u8,

    /// Verbose logging
    #[arg(long, default_value_t = false)]
    pub verbose: bool,
}
