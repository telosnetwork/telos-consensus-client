use alloy::hex;
use alloy::primitives::Parity::Eip155;
use alloy::primitives::U256;
use alloy_consensus::TxLegacy;
use telos_translator_rs::rlp::telos_rlp_decode::TelosTxDecodable;
use telos_translator_rs::transaction::make_unique_vrs;

#[test]
fn test_unsigned_trx() {
    let raw = hex::decode("f78212aa8575a1c379a28307a120947282835cf78a5e88a52fc701f09d1614635be4b8900000000000000000000000000000000080808080").unwrap();

    let tx = TxLegacy::decode_telos_signed_fields(
        &mut raw.as_slice(),
        make_unique_vrs(Default::default(), Default::default(), Default::default()),
    );
    if tx.is_err() {
        println!(
            "Failed to decode unsigned transaction: {:?}",
            tx.clone().err()
        );
        panic!("Failed to decode unsigned transaction");
    }
    let (tx, sig, _hash) = tx.unwrap().into_parts();
    assert_eq!(tx.value, U256::ZERO);
    assert_eq!(sig.v(), Eip155(42));
}
