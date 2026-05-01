use clap::Parser;
use hackathon::{app, cli::Cli, init_logging};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    init_logging(cli.verbose);
    if let Err(err) = app::run(cli).await {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
