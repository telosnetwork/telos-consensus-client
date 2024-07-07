use alloy_rlp::{Error, Header, Result};
use bytes::{Bytes, BytesMut};
use core::marker::{PhantomData, PhantomPinned};
use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

/// A type that can be decoded from an RLP blob.
pub trait TelosDecodable: Sized {
    /// Decodes the blob into the appropriate type. `buf` must be advanced past
    /// the decoded object.
    fn decode_telos(buf: &mut &[u8]) -> Result<Self>;
}

/// An active RLP decoder, with a specific slice of a payload.
pub struct TelosRlp<'a> {
    payload_view: &'a [u8],
}

impl<'a> TelosRlp<'a> {
    /// Instantiate an RLP decoder with a payload slice.
    pub fn new(mut payload: &'a [u8]) -> Result<Self> {
        let payload_view = Header::decode_bytes(&mut payload, true)?;
        Ok(Self { payload_view })
    }

    /// Decode the next item from the buffer.
    #[inline]
    pub fn get_next<T: TelosDecodable>(&mut self) -> Result<Option<T>> {
        if self.payload_view.is_empty() {
            Ok(None)
        } else {
            T::decode_telos(&mut self.payload_view).map(Some)
        }
    }
}

impl<T: ?Sized> TelosDecodable for PhantomData<T> {
    fn decode_telos(_buf: &mut &[u8]) -> Result<Self> {
        Ok(Self)
    }
}

impl TelosDecodable for PhantomPinned {
    fn decode_telos(_buf: &mut &[u8]) -> Result<Self> {
        Ok(Self)
    }
}

impl TelosDecodable for bool {
    #[inline]
    fn decode_telos(buf: &mut &[u8]) -> Result<Self> {
        Ok(match u8::decode_telos(buf)? {
            0 => false,
            1 => true,
            _ => return Err(Error::Custom("invalid bool value, must be 0 or 1")),
        })
    }
}

impl<const N: usize> TelosDecodable for [u8; N] {
    #[inline]
    fn decode_telos(from: &mut &[u8]) -> Result<Self> {
        let bytes = Header::decode_bytes(from, false)?;
        Self::try_from(bytes).map_err(|_| Error::UnexpectedLength)
    }
}

macro_rules! decode_integer {
    ($($t:ty),+ $(,)?) => {$(
        impl TelosDecodable for $t {
            #[inline]
            fn decode_telos(buf: &mut &[u8]) -> Result<Self> {
                let bytes = Header::decode_bytes(buf, false)?;
                static_left_pad(bytes).map(<$t>::from_be_bytes)
            }
        }
    )+};
}

decode_integer!(u8, u16, u32, u64, usize, u128);

impl TelosDecodable for Bytes {
    #[inline]
    fn decode_telos(buf: &mut &[u8]) -> Result<Self> {
        Header::decode_bytes(buf, false).map(|x| Self::from(x.to_vec()))
    }
}

impl TelosDecodable for BytesMut {
    #[inline]
    fn decode_telos(buf: &mut &[u8]) -> Result<Self> {
        Header::decode_bytes(buf, false).map(Self::from)
    }
}

impl TelosDecodable for String {
    #[inline]
    fn decode_telos(buf: &mut &[u8]) -> Result<Self> {
        Header::decode_str(buf).map(Into::into)
    }
}

impl<T: TelosDecodable> TelosDecodable for Vec<T> {
    #[inline]
    fn decode_telos(buf: &mut &[u8]) -> Result<Self> {
        let mut bytes = Header::decode_bytes(buf, true)?;
        let mut vec = Self::new();
        let payload_view = &mut bytes;
        while !payload_view.is_empty() {
            vec.push(T::decode_telos(payload_view)?);
        }
        Ok(vec)
    }
}

macro_rules! wrap_impl {
    ($($(#[$attr:meta])* [$($gen:tt)*] <$t:ty>::$new:ident($t2:ty)),+ $(,)?) => {$(
        $(#[$attr])*
        impl<$($gen)*> TelosDecodable for $t {
            #[inline]
            fn decode_telos(buf: &mut &[u8]) -> Result<Self> {
                <$t2 as TelosDecodable>::decode_telos(buf).map(<$t>::$new)
            }
        }
    )+};
}

wrap_impl! {
    #[cfg(feature = "arrayvec")]
    [const N: usize] <arrayvec::ArrayVec<u8, N>>::from([u8; N]),
    [T: ?Sized + TelosDecodable] <Box<T>>::new(T),
    [T: ?Sized + TelosDecodable] <Rc<T>>::new(T),
    [T: ?Sized + TelosDecodable] <Arc<T>>::new(T),
}

impl<T: ?Sized + ToOwned> TelosDecodable for Cow<'_, T>
where
    T::Owned: TelosDecodable,
{
    #[inline]
    fn decode_telos(buf: &mut &[u8]) -> Result<Self> {
        T::Owned::decode_telos(buf).map(Self::Owned)
    }
}

#[cfg(feature = "std")]
mod std_impl {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    impl TelosDecodable for IpAddr {
        fn decode(buf: &mut &[u8]) -> Result<Self> {
            let bytes = Header::decode_bytes(buf, false)?;
            match bytes.len() {
                4 => Ok(Self::V4(Ipv4Addr::from(
                    slice_to_array::<4>(bytes).expect("infallible"),
                ))),
                16 => Ok(Self::V6(Ipv6Addr::from(
                    slice_to_array::<16>(bytes).expect("infallible"),
                ))),
                _ => Err(Error::UnexpectedLength),
            }
        }
    }

    impl TelosDecodable for Ipv4Addr {
        #[inline]
        fn decode(buf: &mut &[u8]) -> Result<Self> {
            let bytes = Header::decode_bytes(buf, false)?;
            slice_to_array::<4>(bytes).map(Self::from)
        }
    }

    impl TelosDecodable for Ipv6Addr {
        #[inline]
        fn decode(buf: &mut &[u8]) -> Result<Self> {
            let bytes = Header::decode_bytes(buf, false)?;
            slice_to_array::<16>(bytes).map(Self::from)
        }
    }
}

/// Left-pads a slice to a statically known size array.
///
/// # Errors
///
/// Returns an error if the slice is too long or if the first byte is 0.
#[inline]
pub(crate) fn static_left_pad<const N: usize>(data: &[u8]) -> Result<[u8; N]> {
    if data.len() > N {
        return Err(Error::Overflow);
    }

    let mut v = [0; N];

    if data.is_empty() {
        return Ok(v);
    }

    if data[0] == 0 {
        return Err(Error::LeadingZero);
    }

    // SAFETY: length checked above
    unsafe { v.get_unchecked_mut(N - data.len()..) }.copy_from_slice(data);
    Ok(v)
}

#[cfg(feature = "std")]
#[inline]
fn slice_to_array<const N: usize>(slice: &[u8]) -> Result<[u8; N]> {
    slice.try_into().map_err(|_| Error::UnexpectedLength)
}
