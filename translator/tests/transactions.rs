use antelope::api::client::APIClient;
use antelope::chain::asset::{Asset, Symbol};
use antelope::chain::checksum::{Checksum160, Checksum256};
use antelope::chain::name::Name;
use antelope::util::hex_to_bytes;
use reth_primitives::BloomInput::Raw;
use telos_translator_rs::transaction::TelosEVMTransaction;
use telos_translator_rs::types::evm_types::{
    PrintedReceipt, RawAction, TransferAction, WithdrawAction,
};
use telos_translator_rs::types::translator_types::NameToAddressCache;

#[tokio::test]
async fn test_deposit() {
    let trx = TelosEVMTransaction::from_transfer(
        40,
        0,
        Checksum256::from_bytes(&hex_to_bytes(
            "11e1a6c5c637681588383e401479054882c2a168f1ea766bedcc75a9ca4ce6b8",
        ))
        .unwrap(),
        TransferAction {
            from: Name::new("exrsrv.tf"),
            to: Name::new("eosio.evm"),
            quantity: Asset::new(654507, Symbol::new("TLOS", 4)),
            memo: Vec::from("0xb4b01216a5bc8f1c8a33cd990a1239030e60c905".to_string()),
        },
        &NameToAddressCache::new(APIClient::default()),
    )
    .await
    .unwrap();

    assert_eq!(
        trx.hash().to_string(),
        "0xc92303ea310408950f009ac1466d4c4d534cfd02a4bc630881e94e28da7b377e"
    );
}

#[tokio::test]
async fn test_withdraw() {
    let from = "0x87bC2200f5066DFc22e987DAb486b979Cd254F4B"
        .parse()
        .unwrap();
    let trx = TelosEVMTransaction::from_withdraw_no_cache(
        40,
        0,
        Checksum256::from_bytes(&hex_to_bytes(
            "1203ee37cfc4130ea7bf4885f3cbbf1fe85a55ce64709fe7357dcdc453e0ba1f",
        ))
        .unwrap(),
        WithdrawAction {
            to: Name::new("steferretto"),
            quantity: Asset::new(37000000, Symbol::new("TLOS", 4)),
        },
        from,
    )
    .await;

    assert_eq!(
        trx.hash().to_string(),
        "0x38f3f0600ea400119b34289d198c048623520be84f0e9f98941bf033a8e1c49c"
    );
}

//

#[tokio::test]
async fn test_single_zero_sig() {
    // as found in testnet evm block #179647915
    let trx = TelosEVMTransaction::from_raw_action(
        41,
        0,
        Checksum256::from_bytes(&hex_to_bytes(
            "0b3339c3594d86694d0fb78756f22e0d782a7223af0d0d59a51b7bc8b80d8440",
        )).unwrap(),
        RawAction {
            ram_payer: Name::new_from_str("eosio"),
            tx: hex_to_bytes("f901481985746050fb56831e8480949a469d1e668425907548228ea525a661ff3bfa2b80b90124b069bcc30000000000000000000000000000000000000000000000000000000000000060000000000000000000000000111111111111111111111111111111111111111100000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000043300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000220000000000000000000000000000003000000000000000000000000000000000"),
            estimate_gas: false,
            sender: Some(Checksum160::from_hex("a81803217967f4a8e5306057ef75c452685613ab").unwrap()),
        },
        PrintedReceipt {
            charged_gas: "".to_string(),
            trx_index: 0,
            block: 0,
            status: 0,
            epoch: 0,
            createdaddr: "".to_string(),
            gasused: "".to_string(),
            logs: vec![],
            output: "".to_string(),
            errors: None,
        }
        ).await.unwrap();

    println!("{:#?}", trx);
}

#[tokio::test]
async fn test_failed_decode() {
    // as found in testnet evm block #179647915
    let trx = TelosEVMTransaction::from_raw_action(
        41,
        0,
        Checksum256::from_bytes(&hex_to_bytes(
            "0ac6654876b6f061fc165a8aa54e36b1d1704898d26ce33a9c9322068332960e",
        )).unwrap(),
        RawAction {
            ram_payer: Name::new_from_str("eosio"),
            tx: hex_to_bytes("f8570785746050fb56831e848094a763a9333de157a9009f5eecd778b799e3c6d74e80b5b069bcc3000000000000000000000000000000000000000000000000000000006c33bdd2622e59fd10b411ff8d8d8d4dc5caf6ce0000000000000000000000000000000000000000000000000000000017"),
            estimate_gas: false,
            sender: Some(Checksum160::from_hex("6c33bdd2622e59fd10b411ff8d8d8d4dc5caf6ce").unwrap()),
        },
        PrintedReceipt {
            charged_gas: "".to_string(),
            trx_index: 0,
            block: 0,
            status: 0,
            epoch: 0,
            createdaddr: "".to_string(),
            gasused: "".to_string(),
            logs: vec![],
            output: "".to_string(),
            errors: None,
        }
    ).await.unwrap();

    println!("{:#?}", trx);
}
