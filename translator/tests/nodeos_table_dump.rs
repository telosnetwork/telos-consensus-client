use std::fs::File;
use std::io::Write;
use telos_translator_rs::injection::dump_storage;

#[tokio::test]
pub async fn dump_it() {
    let telos_rpc = "http://127.0.0.1:8889";
    let block_delta = 57;

    let json_state = dump_storage(telos_rpc, block_delta).await;
    let json_str = serde_json::to_string_pretty(&json_state).unwrap();

    // Write the JSON to a file
    let mut file = File::create("evm-state.json").unwrap();
    file.write_all(json_str.as_bytes()).unwrap();
}
