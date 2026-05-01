use std::path::PathBuf;

use hackathon::{cli::Cli, config::RuntimeConfig};

fn base_cli() -> Cli {
    Cli {
        input: PathBuf::from("input.csv"),
        output: PathBuf::from("output.csv"),
        src_lang: "en".to_string(),
        tgt_lang: "ne".to_string(),
        base_url: "https://example.com".to_string(),
        api_token: Some("team_test".to_string()),
        concurrency: 2,
        rate_limit_ms: None,
        max_retries: 3,
        font_path: None,
        dpi: 96,
        jpeg_quality: 85,
        verbose: false,
    }
}

#[test]
fn config_rejects_zero_concurrency() {
    let mut cli = base_cli();
    cli.concurrency = 0;
    assert!(RuntimeConfig::try_from(&cli).is_err());
}

#[test]
fn config_rejects_same_language() {
    let mut cli = base_cli();
    cli.tgt_lang = "en".to_string();
    assert!(RuntimeConfig::try_from(&cli).is_err());
}

#[test]
fn config_rejects_invalid_dpi() {
    let mut cli = base_cli();
    cli.dpi = 0;
    assert!(RuntimeConfig::try_from(&cli).is_err());
}

#[test]
fn config_rejects_invalid_jpeg_quality() {
    let mut cli = base_cli();
    cli.jpeg_quality = 0;
    assert!(RuntimeConfig::try_from(&cli).is_err());

    let mut cli = base_cli();
    cli.jpeg_quality = 101;
    assert!(RuntimeConfig::try_from(&cli).is_err());
}
