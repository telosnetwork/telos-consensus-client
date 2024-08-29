use alloy::hex;
use alloy::hex::FromHex;
use alloy::primitives::Parity::Eip155;
use alloy::primitives::{Address, B256, U256};
use alloy_consensus::TxLegacy;
use antelope::chain::checksum::Checksum256;
use telos_translator_rs::rlp::telos_rlp_decode::TelosTxDecodable;
use telos_translator_rs::transaction::make_unique_vrs;

#[test]
fn test_unsigned_trx() {
    let raw = hex::decode(
        "e7808082520894d80744e16d62c62c5fa2a04b92da3fe6b9efb5238b52e00fde054bb73290000080",
    )
    .unwrap();

    let tx = TxLegacy::decode_telos_signed_fields(
        &mut raw.as_slice(),
        make_unique_vrs(
            Checksum256::from_hex(
                "00000032f9ff3095950dbef8701acc5f0eb193e3c2d089da0e2237659048d62b",
            )
            .unwrap(),
            Address::ZERO,
            0,
        ),
    );
    if tx.is_err() {
        println!(
            "Failed to decode unsigned transaction: {:?}",
            tx.clone().err()
        );
        panic!("Failed to decode unsigned transaction");
    }
    assert_eq!(
        tx.unwrap().hash(),
        &B256::from_hex("8d8c62a8bc0762f66ec0be70db1a2e8b9adb6504f4c9bdd2cf794611ebeab87b")
            .unwrap()
    );
}
