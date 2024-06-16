use clap::Parser;
use telos_translator_rs::translator::Translator;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "ws://localhost:18999")]
    ship_endpoint: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let mut translator = Translator::new(args.ship_endpoint).await.unwrap();
    translator.launch().await.unwrap();
}
