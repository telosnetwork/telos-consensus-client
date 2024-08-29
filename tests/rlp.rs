use alloy::hex;
use alloy::primitives::Parity::Eip155;
use alloy::primitives::U256;
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
                "4242424242424242424242424242424242424242424242424242424242424242",
            )
            .unwrap(),
            Default::default(),
            Default::default(),
        ),
    );
    if tx.is_err() {
        println!(
            "Failed to decode unsigned transaction: {:?}",
            tx.clone().err()
        );
        panic!("Failed to decode unsigned transaction");
    }
    let (tx, sig, _hash) = tx.unwrap().into_parts();
    assert_eq!(
        tx.value,
        U256::from_str_radix("52e00fde054bb732900000", 16).unwrap()
    );
    assert_eq!(sig.v(), Eip155(42));
}
