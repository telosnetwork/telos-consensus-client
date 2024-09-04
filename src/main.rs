use std::fs;
use clap::Parser;
use tokio::sync::mpsc;
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
    let (stop_tx, stop_rx) = mpsc::channel::<()>(1);

    if let Err(e) = Translator::new(config).launch(None, stop_tx, stop_rx).await {
        error!("Failed to launch translator: {e:?}");
    }
}
