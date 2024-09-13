use clap::Parser;
use std::fs;
use telos_translator_rs::translator::{Translator, TranslatorConfig};
use tracing::error;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "config.toml")]
    config: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    tracing_subscriber::fmt::init();

    let config_contents = fs::read_to_string(args.config).expect("Could not read config file");
    let config: TranslatorConfig =
        toml::from_str(&config_contents).expect("Could not parse config as toml");

    if let Err(e) = Translator::new(config).launch(None).await {
        error!("Failed to launch translator: {e:?}");
    }
}
