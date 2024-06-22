use alloy::primitives::private::alloy_rlp::{Decodable, EMPTY_STRING_CODE, Error, Header};
use alloy::primitives::{Address, Bytes, TxKind, U256};
use alloy_consensus::TxLegacy;
use alloy_rlp::Result;
use alloy_rlp::Error::{InputTooShort, UnexpectedLength};
use bytes::{Buf};
use crate::rlp::decode::TelosDecodable;

fn decode_fields(data: &mut &[u8]) -> Result<TxLegacy> {
    let nonce = u64::decode_telos(data).expect("Failed to decode nonce");
    let gas_price = u128::decode_telos(data).expect("Failed to decode gas price");
    let gas_limit = u128::decode_telos(data).expect("Failed to decode gas limit");
    let to = TxKind::decode_telos(data).expect("Failed to decode to");
    let value_bytes = U256::decode_telos(data).expect("Failed to decode value");
    let input_bytes = U256::decode_telos(data).expect("Failed to decode input");

    let value = U256::ZERO;
    let input = Bytes::new();

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

impl TelosDecodable for TxLegacy {
    fn decode_telos(data: &mut &[u8]) -> Result<Self> {
        let header = Header::decode(data).expect("Failed to decode header");
        let remaining_len = data.len();

        let transaction_payload_len = header.payload_length;

        if transaction_payload_len > remaining_len {
            return Err(InputTooShort);
        }

        let mut transaction = decode_fields(data).expect("Failed to decode fields");

        // If we still have data, it should be an eip-155 encoded chain_id
        if !data.is_empty() {
            transaction.chain_id = Some(TelosDecodable::decode_telos(data).expect("Failed to decode chain id"));
            let r: U256 = TelosDecodable::decode_telos(data).expect("Failed to decode r value"); // r
            let s: U256 = TelosDecodable::decode_telos(data).expect("Failed to decode s value"); // s
        }

        let decoded = remaining_len - data.len();
        if decoded != transaction_payload_len {
            return Err(UnexpectedLength);
        }

        Ok(transaction)
    }
}

impl TelosDecodable for TxKind {
    fn decode_telos(buf: &mut &[u8]) -> Result<TxKind, alloy_rlp::Error> {
        if let Some(&first) = buf.first() {
            if first == EMPTY_STRING_CODE {
                buf.advance(1);
                Ok(TxKind::Create)
            } else {
                let addr = <Address as Decodable>::decode(buf).expect("Failed to decode address");
                Ok(TxKind::Call(addr))
            }
        } else {
            Err(InputTooShort)
        }
    }
}

impl TelosDecodable for U256 {
    fn decode_telos(buf: &mut &[u8]) -> Result<Self> {
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

        Ok(Self::try_from_be_slice(bytes).expect("Failed to decode U256 from bytes"))

        //
        // if let Some(&first) = buf.first() {
        //     if first == EMPTY_STRING_CODE {
        //         buf.advance(1);
        //         Ok(U256::ZERO)
        //     } else {
        //         let u256 = U256::from_be_slice(&buf[0..32]);
        //         buf.advance(32);
        //         Ok(u256)
        //     }
        // } else {
        //     Err(InputTooShort)
        // }
    }
}