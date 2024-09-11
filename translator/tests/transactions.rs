use antelope::api::client::APIClient;
use antelope::chain::asset::{Asset, Symbol};
use antelope::chain::checksum::Checksum256;
use antelope::chain::name::Name;
use antelope::util::hex_to_bytes;
use telos_translator_rs::transaction::TelosEVMTransaction;
use telos_translator_rs::types::evm_types::{TransferAction, WithdrawAction};
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
            memo: "0xb4b01216a5bc8f1c8a33cd990a1239030e60c905".to_string(),
        },
        &NameToAddressCache::new(APIClient::default()),
    )
    .await;

    assert_eq!(
        trx.hash().to_string(),
        "0xdb81c0fe3f904e4637d7679ac200db43984336d0ea42cea4dd5dd22dec28541b"
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
        "0x2cac6ea0102c2eb6e3ad4288853c0a2d457643d162ff56d1b381bcb8de1fe9e9"
    );
}
