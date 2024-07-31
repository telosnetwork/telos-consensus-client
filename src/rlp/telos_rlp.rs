use crate::rlp::decode::TelosDecodable;
use alloy_rlp::{Error, Header};
use ruint::Uint;

/// Allows a [`Uint`] to be deserialized from RLP.
///
/// See <https://ethereum.org/en/developers/docs/data-structures-and-encoding/rlp/>
impl<const BITS: usize, const LIMBS: usize> TelosDecodable for Uint<BITS, LIMBS> {
    #[inline]
    fn decode_telos(buf: &mut &[u8]) -> Result<Self, Error> {
        let bytes = Header::decode_bytes(buf, false)?;

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

        Self::try_from_be_slice(bytes).ok_or(Error::Overflow)
    }
}
