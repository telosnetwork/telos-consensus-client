use alloy::{hex::FromHex, primitives::FixedBytes};
use antelope::{
    api::client::{APIClient, DefaultProvider},
    chain::{checksum::Checksum256, name::Name, signature::Signature, Encoder},
};
use chrono::NaiveDateTime;
use serde::Deserialize;
use telos_translator_rs::{
    block::{ProcessingEVMBlock, ProcessingEVMBlockArgs},
    types::{
        env::{ANTELOPE_EPOCH_MS, ANTELOPE_INTERVAL_MS, MAINNET_DEPLOY_CONFIG},
        ship_types::{
            BlockHeader, BlockPosition, GetBlocksResultV0, SignedBlock, SignedBlockHeader,
            TableDelta, TransactionTrace,
        },
        translator_types::{ChainId, NameToAddressCache},
    },
};

#[derive(Deserialize)]
struct PinnedNativeBlockHeader {
    block_num: u32,
    block_id: String,
    timestamp: String,
    producer: String,
    confirmed: u16,
    previous: String,
    transaction_mroot: String,
    action_mroot: String,
    schedule_version: u32,
    producer_signature: String,
}

fn pinned_block(block_num: u32) -> PinnedNativeBlockHeader {
    let fixtures: Vec<PinnedNativeBlockHeader> =
        serde_json::from_str(include_str!("fixtures/mainnet-native-block-headers.json"))
            .expect("mainnet native block fixtures must be valid JSON");
    fixtures
        .into_iter()
        .find(|fixture| fixture.block_num == block_num)
        .unwrap_or_else(|| panic!("missing mainnet native block fixture {block_num}"))
}

fn checksum(value: &str) -> Checksum256 {
    Checksum256::from_hex(value).expect("fixture checksum must be valid")
}

fn timestamp_slot(value: &str) -> u32 {
    let unix_ms = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.3f")
        .expect("fixture timestamp must be valid")
        .and_utc()
        .timestamp_millis();
    let elapsed_ms = unix_ms
        .checked_sub(ANTELOPE_EPOCH_MS as i64)
        .expect("fixture timestamp must follow the Antelope epoch");
    assert_eq!(
        elapsed_ms % ANTELOPE_INTERVAL_MS as i64,
        0,
        "fixture timestamp must fall on an Antelope block slot"
    );
    u32::try_from(elapsed_ms / ANTELOPE_INTERVAL_MS as i64)
        .expect("fixture timestamp slot must fit u32")
}

fn generate_block(chain_id: u64, fixture: PinnedNativeBlockHeader) -> ProcessingEVMBlock {
    let block_pos = BlockPosition {
        block_num: fixture.block_num,
        block_id: checksum(&fixture.block_id),
    };
    let signed_block = SignedBlock {
        header: SignedBlockHeader {
            header: BlockHeader {
                timestamp: timestamp_slot(&fixture.timestamp),
                producer: Name::new_from_str(&fixture.producer),
                confirmed: fixture.confirmed,
                previous: checksum(&fixture.previous),
                transaction_mroot: checksum(&fixture.transaction_mroot),
                action_mroot: checksum(&fixture.action_mroot),
                schedule_version: fixture.schedule_version,
                new_producers: None,
                header_extensions: vec![],
            },
            producer_signature: Signature::from_string(&fixture.producer_signature)
                .expect("fixture signature must be valid"),
        },
        transactions: vec![],
        block_extensions: vec![],
    };

    ProcessingEVMBlock::new(ProcessingEVMBlockArgs {
        chain_id,
        block_num: fixture.block_num,
        block_hash: block_pos.block_id,
        prev_block_hash: None,
        // The pinned header fixture represents an irreversible historical block.
        lib_num: fixture.block_num,
        lib_hash: block_pos.block_id,
        result: GetBlocksResultV0 {
            head: block_pos.clone(),
            last_irreversible: block_pos.clone(),
            this_block: Some(block_pos),
            prev_block: None,
            block: Some(Encoder::pack(&signed_block)),
            // Header generation does not need EVM events, but strict SHIP decoding requires each
            // requested component to be present as a valid encoded collection.
            traces: Some(Encoder::pack(&Vec::<TransactionTrace>::new())),
            deltas: Some(Encoder::pack(&Vec::<TableDelta>::new())),
        },
        skip_events: false,
    })
}

fn offline_name_cache() -> NameToAddressCache {
    NameToAddressCache::new(
        APIClient::<DefaultProvider>::default_provider("http://127.0.0.1:1".to_string(), Some(1))
            .expect("offline test endpoint must be a valid URL"),
    )
}

#[tokio::test]
async fn genesis_mainnet() {
    let chain_id = ChainId(40);
    let mut block = generate_block(chain_id.0, pinned_block(36));
    block
        .deserialize()
        .expect("fixture must be valid SHIP data");

    let (_, payload) = block
        .generate_evm_data(
            FixedBytes::ZERO,
            chain_id.block_delta(),
            &offline_name_cache(),
        )
        .await
        .expect("fixture must generate EVM data");

    assert_eq!(
        payload.block_hash,
        FixedBytes::from_hex("36fe7024b760365e3970b7b403e161811c1e626edd68460272fcdfa276272563")
            .unwrap()
    );
}

#[tokio::test]
async fn deploy_mainnet() {
    let chain_id = ChainId(40);
    let parent_hash = FixedBytes::from_hex(&MAINNET_DEPLOY_CONFIG.prev_hash).unwrap();
    let mut block = generate_block(
        chain_id.0,
        pinned_block(MAINNET_DEPLOY_CONFIG.evm_start_block),
    );
    block
        .deserialize()
        .expect("fixture must be valid SHIP data");

    let (_, payload) = block
        .generate_evm_data(parent_hash, chain_id.block_delta(), &offline_name_cache())
        .await
        .expect("fixture must generate EVM data");

    assert_eq!(
        payload.block_hash,
        FixedBytes::from_hex(MAINNET_DEPLOY_CONFIG.validate_hash.clone().unwrap()).unwrap()
    );
}
