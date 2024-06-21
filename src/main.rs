use clap::Parser;
use tracing::{error, info};
use telos_translator_rs::translator::Translator;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "http://localhost:8888")]
    http_endpoint: String,
    #[arg(short, long, default_value = "ws://localhost:18999")]
    ship_endpoint: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    tracing_subscriber::fmt::init();
    let mut translator = Translator::new(args.http_endpoint, args.ship_endpoint).await.unwrap();
    match translator.launch().await {
        Ok(_) => info!("Translator launched successfully"),
        Err(e) => error!("Failed to launch translator: {:?}", e),
    }
}
