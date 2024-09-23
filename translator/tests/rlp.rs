use alloy::hex;
use alloy::hex::FromHex;
use alloy::primitives::{Address, B256};
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
        Some(make_unique_vrs(
            Checksum256::from_hex(
                "00000032f9ff3095950dbef8701acc5f0eb193e3c2d089da0e2237659048d62b",
            )
            .unwrap(),
            Address::ZERO,
            0,
        )),
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
        &B256::from_hex("ede91f8a618cd49907d9a90fe2bf0443848f5ff549369eac42d1978b4fb8eccc")
            .unwrap()
    );
}

#[test]
fn test_unsigned_trx2() {
    let raw = hex::decode(
        "f78212aa8575a1c379a28307a120947282835cf78a5e88a52fc701f09d1614635be4b8900000000000000000000000000000000080808080",
    )
        .unwrap();

    let tx = TxLegacy::decode_telos_signed_fields(
        &mut raw.as_slice(),
        Some(make_unique_vrs(
            Checksum256::from_hex(
                "00000032f9ff3095950dbef8701acc5f0eb193e3c2d089da0e2237659048d62b",
            )
            .unwrap(),
            Address::ZERO,
            0,
        )),
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
        &B256::from_hex("3f5cba81e5f45971c4743f86644103328479a6d1640e78b0bc1aa286f0da91a2")
            .unwrap()
    );
}

#[test]
fn test_signed_trx() {
    let raw = hex::decode(
        "f8aa11857a307efa8083023fa09479f5a8bd0d6a00a41ea62cda426cef0115117a6180b844e2bbb1580000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000073a0b40ec08b01a351dcbf5e86eeb15262bf7033dc7b99a054dfb198487636a79c5fa000b64d6775ba737738ccff7f1c0a29c287cbb91f2eb17e1d0b74ffb73d9daa85",
    ).unwrap();

    let signed_legacy =
        TxLegacy::decode_telos_signed_fields(&mut raw.as_slice(), None);
    
    assert!(signed_legacy.is_ok());
}