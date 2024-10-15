use alloy::{hex::FromHex, primitives::FixedBytes};
use antelope::{
    api::client::{APIClient, DefaultProvider},
    chain::{checksum::Checksum256, Encoder},
};
use telos_translator_rs::{
    block::ProcessingEVMBlock,
    types::{
        env::{ANTELOPE_EPOCH_MS, ANTELOPE_INTERVAL_MS, MAINNET_DEPLOY_CONFIG},
        ship_types::{
            BlockHeader, BlockPosition, GetBlocksResultV0, SignedBlock, SignedBlockHeader,
        },
        translator_types::{ChainId, NameToAddressCache},
    },
};
use tracing::info;
use telos_translator_rs::block::ProcessingEVMBlockArgs;

async fn generate_block(
    chain_id: u64,
    http_endpoint: String,
    block_num: u32,
) -> ProcessingEVMBlock {
    let api_client: APIClient<DefaultProvider> =
        APIClient::<DefaultProvider>::default_provider(http_endpoint.clone(), Some(10))
            .expect("Failed to create API client");

    let block = api_client
        .v1_chain
        .get_block(block_num.to_string())
        .await
        .expect("Failed to fetch block");

    let block_pos = BlockPosition {
        block_num,
        block_id: Checksum256::from_bytes(&block.id.bytes).expect("Failed to parse block id"),
    };

    let time_slot = ((block.time_point.elapsed / 1000) - ANTELOPE_EPOCH_MS) / ANTELOPE_INTERVAL_MS;

    let signed_block = SignedBlock {
        header: SignedBlockHeader {
            header: BlockHeader {
                timestamp: time_slot as u32,
                producer: block.producer,
                confirmed: block.confirmed,
                previous: Checksum256::from_bytes(&block.previous.bytes).unwrap(),
                transaction_mroot: block.transaction_mroot,
                action_mroot: block.action_mroot,
                schedule_version: block.schedule_version,
                new_producers: None,
                header_extensions: vec![],
            },
            producer_signature: block.producer_signature,
        },
        transactions: vec![],
        block_extensions: vec![],
    };

    let block_bytes = Encoder::pack(&signed_block);

    info!("block_pos.block_id {}", block_pos.block_id);

    ProcessingEVMBlock::new(
        ProcessingEVMBlockArgs {
            chain_id: chain_id,
            block_num: block_num,
            block_hash: block_pos.block_id,
            prev_block_hash: None,
            // Block is always final
            lib_num: block_num,
            lib_hash: block_pos.block_id,
            result: GetBlocksResultV0 {
                head: block_pos.clone(),
                last_irreversible: block_pos.clone(),
                this_block: Some(block_pos.clone()),
                prev_block: None,
                block: Some(block_bytes),
                traces: Some(vec![]),
                deltas: Some(vec![]),
            },
            use_legacy_raw_action: false,
    })
}

#[tokio::test]
async fn genesis_mainnet() {
    let evm_chain_id_mainnet = ChainId(40);
    let http_endpoint = "https://mainnet.telos.net".to_string();

    let native_to_evm_cache = NameToAddressCache::new(
        APIClient::<DefaultProvider>::default_provider(http_endpoint.clone(), Some(10))
            .expect("Failed to create API client"),
    );
    let zero_bytes = FixedBytes::from_slice(&[
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0,
    ]);

    let mut block = generate_block(evm_chain_id_mainnet.0, http_endpoint, 36).await;

    block.deserialize();

    if let Ok((header, payload)) = block
        .generate_evm_data(
            zero_bytes,
            evm_chain_id_mainnet.block_delta(),
            &native_to_evm_cache,
        )
        .await
    {
        println!("genesis: {:#?}", header);
        println!("hash: {:#?}", payload.block_hash);

        assert_eq!(
            payload.block_hash,
            FixedBytes::from_hex(
                "36fe7024b760365e3970b7b403e161811c1e626edd68460272fcdfa276272563"
            )
            .unwrap()
        );
    } else {
        panic!("Failed to generate evm data");
    }
}

#[tokio::test]
async fn deploy_mainnet() {
    let evm_chain_id_mainnet = ChainId(40);
    let http_endpoint = "https://mainnet.telos.net".to_string();

    let native_to_evm_cache = NameToAddressCache::new(
        APIClient::<DefaultProvider>::default_provider(http_endpoint.clone(), Some(10))
            .expect("Failed to create API client"),
    );
    let parent_hash = FixedBytes::from_hex(&MAINNET_DEPLOY_CONFIG.prev_hash).unwrap();

    let mut block = generate_block(
        evm_chain_id_mainnet.0,
        http_endpoint,
        MAINNET_DEPLOY_CONFIG.evm_start_block,
    )
    .await;

    block.deserialize();

    if let Ok((header, payload)) = block
        .generate_evm_data(
            parent_hash,
            evm_chain_id_mainnet.block_delta(),
            &native_to_evm_cache,
        )
        .await
    {
        println!("genesis: {:#?}", header);
        println!("hash: {:#?}", payload.block_hash);

        assert_eq!(
            payload.block_hash,
            FixedBytes::from_hex(MAINNET_DEPLOY_CONFIG.validate_hash.clone().unwrap()).unwrap()
        );
    } else {
        panic!("Failed to generate evm data");
    }
}
