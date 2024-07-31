use std::fs;

use clap::Parser;
use tracing::{error, info};
use telos_translator_rs::translator::{Translator, TranslatorConfig};

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

    let config_contents = fs::read_to_string(args.config)
        .expect("Could not read config file");
    let config: TranslatorConfig = toml::from_str(&config_contents)
        .expect("Could not parse config as toml");

    let mut translator = Translator::new(config).await.unwrap();
    match translator.launch(None).await {
        Ok(_) => info!("Translator launched successfully"),
        Err(e) => error!("Failed to launch translator: {:?}", e),
    }

    // Keep the main thread alive
    loop {
        tokio::task::yield_now().await;
    }
}
