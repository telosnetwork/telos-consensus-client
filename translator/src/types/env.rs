use std::str::FromStr;

use alloy::primitives::FixedBytes;
use lazy_static::lazy_static;

use crate::translator::{default_channel_size, TranslatorConfig};

pub const ANTELOPE_EPOCH_MS: u64 = 946684800000;
pub const ANTELOPE_INTERVAL_MS: u64 = 500;

pub const ZERO_HASH_HEX: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub const DEFAULT_GAS_LIMIT: u128 = 0x7fffffff;

lazy_static! {
    pub static ref ZERO_HASH: FixedBytes<32> = FixedBytes::from_str(ZERO_HASH_HEX).unwrap();
    pub static ref MAINNET_GENESIS_CONFIG: TranslatorConfig = TranslatorConfig {
        chain_id: 40.into(),

        skip_raw_tx_until: None,
        evm_start_block: 37,
        evm_stop_block: None,

        prev_hash: ZERO_HASH_HEX.to_string(),
        validate_hash: Some(
            "36fe7024b760365e3970b7b403e161811c1e626edd68460272fcdfa276272563".to_string()
        ),

        http_endpoint: String::from("http://127.0.0.1:8888"),
        ship_endpoint: String::from("ws://127.0.0.1:29999"),

        raw_message_channel_size: default_channel_size(),
        block_message_channel_size: default_channel_size(),
        final_message_channel_size: default_channel_size()
    };
    pub static ref MAINNET_DEPLOY_CONFIG: TranslatorConfig = TranslatorConfig {
        chain_id: 40.into(),

        skip_raw_tx_until: None,
        evm_start_block: 180698860,
        evm_stop_block: None,

        prev_hash: "757720a8e51c63ef1d4f907d6569dacaa965e91c2661345902de18af11f81063".to_string(),
        validate_hash: Some(
            "ed58397aca4c7ce2117fae8093bdced8f01d47855a46bb5ad6e4df4a93e8ee27".to_string()
        ),

        http_endpoint: String::from("http://127.0.0.1:8888"),
        ship_endpoint: String::from("ws://127.0.0.1:29999"),

        raw_message_channel_size: default_channel_size(),
        block_message_channel_size: default_channel_size(),
        final_message_channel_size: default_channel_size()
    };
    pub static ref TESTNET_GENESIS_CONFIG: TranslatorConfig = TranslatorConfig {
        chain_id: 41.into(),

        // TODO: Figure out this number
        skip_raw_tx_until: Some(1234),
        evm_start_block: 58,
        evm_stop_block: None,

        prev_hash: ZERO_HASH_HEX.to_string(),
        validate_hash: Some(
            "1f42e34c53aa45b4bb0a8fc20cb98ba1f0663ef1d581995c56f9f2314b837a35".to_string()
        ),

        http_endpoint: String::from("http://127.0.0.1:8888"),
        ship_endpoint: String::from("ws://127.0.0.1:29999"),

        raw_message_channel_size: default_channel_size(),
        block_message_channel_size: default_channel_size(),
        final_message_channel_size: default_channel_size()
    };
    // pub static ref TESTNET_DEPLOY_CONFIG: TranslatorConfig = TranslatorConfig {
    //     chain_id: 41.into(),
    //
    //     // TODO: Figure out this number, likely on testnet we need to also start much earlier than 136mil
    //     last_legacy_raw_tx: Some(1234),
    //     evm_start_block: 136393814,
    //     evm_stop_block: None,
    //
    //     prev_hash: "8e149fd918bad5a4adfe6f17478e46643f7db7292a2b7b9247f48dc85bdeec94".to_string(),
    //     validate_hash: None,
    //
    //     http_endpoint: String::from("http://127.0.0.1:8888"),
    //     ship_endpoint: String::from("ws://127.0.0.1:29999"),
    //
    //     raw_message_channel_size: default_channel_size(),
    //     block_message_channel_size: default_channel_size(),
    //     final_message_channel_size: default_channel_size()
    // };
}
