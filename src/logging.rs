use tracing_subscriber::EnvFilter;

pub fn init_logging(verbose: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        if verbose {
            EnvFilter::new("hackathon=debug,reqwest=warn")
        } else {
            EnvFilter::new("hackathon=info,reqwest=warn")
        }
    });

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
