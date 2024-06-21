use alloy::hex;
use alloy_consensus::TxLegacy;
use telos_translator_rs::rlp::decode::TelosDecodable;

#[test]
fn test_unsigned_trx() {
    let raw = hex::decode("f78212aa8575a1c379a28307a120947282835cf78a5e88a52fc701f09d1614635be4b8900000000000000000000000000000000080808080").unwrap();

    let tx = TxLegacy::decode_telos(&mut raw.as_slice());
    if tx.is_err() {
        println!("Failed to decode unsigned transaction: {:?}", tx.err());
        assert!(false, "Failed to decode unsigned transaction");
    }
}