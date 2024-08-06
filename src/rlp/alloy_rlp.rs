use alloy::primitives::private::alloy_rlp::{Decodable, Error, Header};
use alloy::primitives::{Bytes, Signature, TxKind, U256};
use alloy_consensus::{SignableTransaction, Signed, TxLegacy};

use alloy_rlp::Result;

fn decode_fields(data: &mut &[u8]) -> Result<TxLegacy> {
    let nonce = u64::decode(data).expect("Failed to decode nonce");
    let gas_price = u128::decode(data).expect("Failed to decode gas price");
    let gas_limit = u128::decode(data).expect("Failed to decode gas limit");
    let to = TxKind::decode(data).expect("Failed to decode to");
    let value = decode_telos_u256(data).expect("Failed to decode value");
    let input = Bytes::decode(data).expect("Failed to decode input");

    Ok(TxLegacy {
        chain_id: None,
        nonce,
        gas_price,
        gas_limit,
        to,
        value,
        input,
    })
}

pub trait TelosTxDecodable {
    fn decode_telos_signed_fields(
        buf: &mut &[u8],
        sig: Signature,
    ) -> alloy::primitives::private::alloy_rlp::Result<Signed<Self>>
    where
        Self: Sized;
}

impl TelosTxDecodable for TxLegacy {
    fn decode_telos_signed_fields(
        buf: &mut &[u8],
        sig: Signature,
    ) -> alloy::primitives::private::alloy_rlp::Result<Signed<Self>, Error> {
        let header = Header::decode(buf)?;
        if !header.list {
            return Err(Error::UnexpectedString);
        }

        // record original length so we can check encoding
        let original_len = buf.len();

        let mut tx = decode_fields(buf).map_err(|_| Error::Custom("Failed to decode fields"))?;

        let signature = Signature::decode_rlp_vrs(buf)?;

        // extract chain id from signature
        let v = signature.v();
        let r = signature.r();
        let s = signature.s();
        if v.to_u64() != 0 || r != U256::ZERO || s != U256::ZERO {
            return Err(Error::Custom("Invalid signature"));
        }

        tx.chain_id = sig.v().chain_id();

        let signed = tx.into_signed(sig);
        if buf.len() + header.payload_length != original_len {
            return Err(Error::ListLengthMismatch {
                expected: header.payload_length,
                got: original_len - buf.len(),
            });
        }

        Ok(signed)
    }
}
// TODO is this needed?
//
// impl TelosDecodable for TxLegacy {
//     fn decode_telos(data: &mut &[u8]) -> Result<Self> {
//         let header = Header::decode(data).expect("Failed to decode header");
//         let remaining_len = data.len();
//
//         let transaction_payload_len = header.payload_length;
//
//         if transaction_payload_len > remaining_len {
//             return Err(InputTooShort);
//         }
//
//         let mut transaction = decode_fields(data).expect("Failed to decode fields");
//
//         // If we still have data, it should be an eip-155 encoded chain_id
//         if !data.is_empty() {
//             transaction.chain_id =
//                 Some(TelosDecodable::decode_telos(data).expect("Failed to decode chain id"));
//             let _r: U256 = TelosDecodable::decode_telos(data).expect("Failed to decode r value");
//             let _s: U256 = TelosDecodable::decode_telos(data).expect("Failed to decode s value");
//         }
//
//         let decoded = remaining_len - data.len();
//         if decoded != transaction_payload_len {
//             return Err(UnexpectedLength);
//         }
//
//         Ok(transaction)
//     }
// }

// decode_telos_u256 decodes rlp value of u256 but without a check if the first bytes are zero
// which is a rlp specification requirement.
// Note: legacy transactions signed by the native network who's RLP value field is encoded as bytes and has a leading zeroes.
fn decode_telos_u256(buf: &mut &[u8]) -> Result<U256> {
    let bytes = Header::decode_bytes(buf, false).expect("Failed to decode bytes");

    // The RLP spec states that deserialized positive integers with leading zeroes
    // get treated as invalid.
    //
    // See:
    // https://ethereum.org/en/developers/docs/data-structures-and-encoding/rlp/
    //
    // To check this, we only need to check if the first byte is zero to make sure
    // there are no leading zeros
    // if !bytes.is_empty() && bytes[0] == 0 {
    //     return Err(Error::LeadingZero);
    // }

    Ok(U256::try_from_be_slice(bytes).expect("Failed to decode U256 from bytes"))
}
