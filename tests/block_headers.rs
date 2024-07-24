use alloy::{hex::FromHex, primitives::FixedBytes};
use antelope::{api::client::{APIClient, DefaultProvider}, chain::{checksum::Checksum256, name::Name, Encoder}};
use telos_translator_rs::{block::Block, types::{ship_types::{BlockHeader, BlockPosition, GetBlocksResultV0, SignedBlock, SignedBlockHeader}, types::NameToAddressCache}};


async fn generate_block(
    chain_id: u64,
    http_endpoint: String,
    block_num: u32,
) -> Block {
    let api_client: APIClient<DefaultProvider> = APIClient::<DefaultProvider>::default_provider(
        http_endpoint.clone()
    ).expect("Failed to create API client");

    let block = api_client.v1_chain.get_block(
        block_num.to_string()
    ).await.expect("Failed to fetch block");

    let block_pos = BlockPosition {
        block_num,
        block_id: Checksum256::from_bytes(&block.id.bytes).expect("Failed to parse block id")
    };

    let signed_block = SignedBlock {
        header: SignedBlockHeader {
            header: BlockHeader {
                timestamp: (block.time_point.elapsed / 1_000_000) as u32,
                producer: block.producer,
                confirmed: block.confirmed,
                previous: Checksum256::from_bytes(&block.previous.bytes).unwrap(),
                transaction_mroot: block.transaction_mroot,
                action_mroot: block.action_mroot,
                schedule_version: block.schedule_version,
                new_producers: None,
                header_extensions: vec![],
            },
            producer_signature: block.producer_signature
        },
        transactions: vec![],
        block_extensions: vec![]
    };

    let block_bytes = Encoder::pack(&signed_block);

    Block::new(
        chain_id,
        block_num as u64,
        block_pos.block_id,
        GetBlocksResultV0 {
            head: block_pos.clone(),
            last_irreversible: block_pos.clone(),
            this_block: Some(block_pos.clone()),
            prev_block: None,
            block: Some(block_bytes),
            traces: Some(vec![]),
            deltas: Some(vec![]),
        },
    )
}


#[tokio::test]
async fn genesis_mainnet() {
    let evm_chain_id_mainnet = 40;
    let http_endpoint = "https://mainnet.telos.net".to_string();

    let native_to_evm_cache = NameToAddressCache::new(
        APIClient::<DefaultProvider>::default_provider(http_endpoint.clone()).expect("Failed to create API client"));
    let zero_bytes = FixedBytes::from_slice(
        &vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    let mut block = generate_block(evm_chain_id_mainnet, http_endpoint, 36).await;

    block.deserialize();

    let evm_genesis = block.generate_evm_data(
        zero_bytes.clone(),
        &native_to_evm_cache
    ).await;

    println!("genesis: {:#?}", evm_genesis);
    println!("hash: {:#?}", evm_genesis.hash_slow());

    assert_eq!(
        evm_genesis.hash_slow(),
        FixedBytes::from_hex("36fe7024b760365e3970b7b403e161811c1e626edd68460272fcdfa276272563").unwrap()
    );
}
