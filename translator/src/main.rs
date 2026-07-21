use clap::Parser;
use eyre::{Context, Result};
use std::fs;
use std::process::ExitCode;
use telos_translator_rs::translator::{Translator, TranslatorConfig};
use tracing::error;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "translator-config.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!("Translator stopped after a fatal error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt::init();

    let config_contents = fs::read_to_string(&args.config)
        .wrap_err_with(|| format!("Could not read config file {}", args.config))?;
    let config: TranslatorConfig =
        toml::from_str(&config_contents).wrap_err("Could not parse config as TOML")?;

    Translator::new(config)
        .launch(None)
        .await
        .wrap_err("Failed to launch translator")
}
