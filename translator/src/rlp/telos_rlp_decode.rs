use alloy::primitives::private::alloy_rlp::{Decodable, Error, Header};
use alloy::primitives::{Bytes, Parity, Signature, TxKind, U256};
use alloy_consensus::{SignableTransaction, Signed, TxLegacy};

use alloy_rlp::Result;
use bytes::Buf;

fn decode_fields(data: &mut &[u8]) -> Result<TxLegacy, Error> {
    let nonce = u64::decode(data).map_err(|_| Error::Custom("Failed to decode nonce"))?;
    let gas_price = u128::decode(data).map_err(|_| Error::Custom("Failed to decode gas price"))?;
    let gas_limit = u128::decode(data).map_err(|_| Error::Custom("Failed to decode gas limit"))?;
    let to = TxKind::decode(data).map_err(|_| Error::Custom("Failed to decode to"))?;
    let value = decode_telos_u256(data).map_err(|_| Error::Custom("Failed to decode value"))?;
    let input = Bytes::decode(data).map_err(|_| Error::Custom("Failed to decode input"))?;

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
        sig: Option<Signature>,
    ) -> Result<Signed<Self>, Error>
    where
        Self: Sized;
}

impl TelosTxDecodable for TxLegacy {
    fn decode_telos_signed_fields(
        buf: &mut &[u8],
        provided_sig: Option<Signature>,
    ) -> Result<Signed<Self>, Error> {
        let header = Header::decode(buf)?;
        if !header.list {
            return Err(Error::Custom("Not a list."));
        }

        // record original length so we can check encoding
        let original_len = buf.len();

        let mut tx = decode_fields(buf).expect("Failed to decode fields");
        let mut v = Parity::Parity(false);
        let mut r = U256::ZERO;
        let mut s = U256::ZERO;

        let sig = match provided_sig.is_some() {
            true => {
                if !buf.is_empty() {
                    // There are some native signed transactions which were RLP encoded with 0 values for signature
                    //   in RLP encoding these 0 values are encoded as [128, 128, 128], so we need purge them
                    //   from the buffer but leave signature value as zeros
                    if buf == &[128, 128, 128] {
                        buf.advance(3);
                    } else {
                        let decoded_signature = Signature::decode_rlp_vrs(buf)?;
                        v = decoded_signature.v();
                        r = decoded_signature.r();
                        s = decoded_signature.s();
                        if v.to_u64() != 0 || r != U256::ZERO || s != U256::ZERO {
                            return Err(Error::Custom(
                                "Unsigned Telos Native trx with signature data",
                            ));
                        }
                    }
                }
                provided_sig.unwrap()
            }
            false => {
                if buf.is_empty() {
                    return Err(Error::Custom("Trx without signature"));
                }

                let parity: Parity = Decodable::decode(buf)?;
                let r = decode_telos_u256(buf)?;
                let s = decode_telos_u256(buf)?;

                Signature::from_rs_and_parity(r, s, parity)
                    .map_err(|_| Error::Custom("attempted to decode invalid field element"))?
            }
        };

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
