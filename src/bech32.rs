// LNP/BP Core Library implementing LNPBP specifications & standards
// Written in 2021 by
//     Dr. Maxim Orlovsky <orlovsky@pandoracore.com>
//
// To the extent possible under law, the author(s) have dedicated all
// copyright and related and neighboring rights to this software to
// the public domain worldwide. This software is distributed without
// any warranty.
//
// You should have received a copy of the MIT License
// along with this software.
// If not, see <https://opensource.org/licenses/MIT>.

//! Types that need to have `data1...` and `z1...` bech 32 implementation
//! according to LNPBP-39 must implement [`ToBech32Payload`] and
//! [`FromBech32Payload`] traits.
//!
//! Bech32 `id1...` representation is provided automatically only for hash types
//! implementing [`bitcoin::hashes::Hash`] trait

#[cfg(feature = "zip")]
use deflate::{write::DeflateEncoder, Compression};
use serde::{
    de::{Error as SerdeError, Unexpected, Visitor},
    Deserializer, Serializer,
};
use std::convert::{Infallible, TryFrom};
use std::fmt;
use std::str::FromStr;

use bech32::{FromBase32, ToBase32};
use bitcoin::hashes::{sha256t, Hash};

pub const HRP_ID: &'static str = "id";
pub const HRP_DATA: &'static str = "data";
#[cfg(feature = "zip")]
pub const HRP_ZIP: &'static str = "z";

#[cfg(feature = "zip")]
pub const RAW_DATA_ENCODING_DEFLATE: u8 = 1u8;

// TODO: Derive more traits once `bech32::Error` will support them
/// Errors generated by Bech32 conversion functions (both parsing and
/// type-specific conversion errors)
#[derive(Clone, PartialEq, Debug, Display, From, Error)]
#[display(doc_comments)]
pub enum Error {
    /// Bech32 string parse error: {0}
    #[from]
    Bech32Error(::bech32::Error),

    /// Payload data parse error: {0}
    #[from(strict_encoding::Error)]
    #[from(bitcoin::consensus::encode::Error)]
    #[from(bitcoin::hashes::Error)]
    #[from(Infallible)]
    WrongData,

    /// Requested object type does not match used Bech32 HRP
    WrongPrefix,

    /// Provided raw data use unknown encoding version {0}
    UnknownRawDataEncoding(u8),

    /// Can not encode raw data with DEFLATE algorithm
    DeflateEncoding,

    /// Error inflating compressed data from payload: {0}
    InflateError(String),
}

/// Type for wrapping Vec<u8> data in cases you need to do a convenient
/// enum variant display derives with `#[display(inner)]`
#[derive(
    Wrapper,
    Clone,
    Ord,
    PartialOrd,
    Eq,
    PartialEq,
    Hash,
    Default,
    Debug,
    Display,
    From,
    StrictEncode,
    StrictDecode,
)]
#[wrap(
    Index,
    IndexMut,
    IndexRange,
    IndexFull,
    IndexFrom,
    IndexTo,
    IndexInclusive
)]
#[display(Vec::bech32_data_string)]
// We get `(To)Bech32DataString` and `FromBech32DataString` for free b/c
// the wrapper creates `From<Vec<u8>>` impl for us, which with rust stdlib
// implies `TryFrom<Vec<u8>>`, for which we have auto trait derivation
// `FromBech32Payload`, for which the traits above are automatically derived
pub struct Blob(Vec<u8>);

impl FromStr for Blob {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Blob::from_bech32_data_str(s)
    }
}

pub trait ToBech32Payload {
    fn to_bech32_payload(&self) -> Vec<u8>;
}

pub trait AsBech32Payload {
    fn as_bech32_payload(&self) -> &[u8];
}

impl<T> AsBech32Payload for T
where
    T: AsRef<[u8]>,
{
    fn as_bech32_payload(&self) -> &[u8] {
        self.as_ref()
    }
}

pub trait FromBech32Payload
where
    Self: Sized,
{
    fn from_bech32_payload(payload: Vec<u8>) -> Result<Self, Error>;
}

impl<T> FromBech32Payload for T
where
    T: TryFrom<Vec<u8>>,
    Error: From<T::Error>,
{
    fn from_bech32_payload(payload: Vec<u8>) -> Result<T, Error> {
        Ok(T::try_from(payload)?)
    }
}

// -- Common (non-LNPBP-39) traits

pub trait ToBech32String {
    fn to_bech32_string(&self) -> String;
}

pub trait FromBech32Str {
    const HRP: &'static str;

    fn from_bech32_str(s: &str) -> Result<Self, Error>
    where
        Self: Sized;
}

pub mod strategies {
    use super::*;
    use amplify::{Holder, Wrapper};
    use strict_encoding::{StrictDecode, StrictEncode};

    pub struct UsingStrictEncoding;
    pub struct Wrapped;
    #[cfg(feature = "zip")]
    pub struct CompressedStrictEncoding;

    pub trait Strategy {
        const HRP: &'static str;
        type Strategy;
    }

    impl<T> ToBech32String for T
    where
        T: Strategy + Clone,
        Holder<T, <T as Strategy>::Strategy>: ToBech32String,
    {
        #[inline]
        fn to_bech32_string(&self) -> String {
            Holder::new(self.clone()).to_bech32_string()
        }
    }

    impl<T> FromBech32Str for T
    where
        T: Strategy,
        Holder<T, <T as Strategy>::Strategy>: FromBech32Str,
    {
        const HRP: &'static str = T::HRP;

        #[inline]
        fn from_bech32_str(s: &str) -> Result<Self, Error> {
            Ok(Holder::from_bech32_str(s)?.into_inner())
        }
    }

    impl<T> ToBech32String for Holder<T, Wrapped>
    where
        T: Wrapper,
        T::Inner: ToBech32String,
    {
        #[inline]
        fn to_bech32_string(&self) -> String {
            self.as_inner().as_inner().to_bech32_string()
        }
    }

    impl<T> FromBech32Str for Holder<T, Wrapped>
    where
        T: Wrapper + Strategy,
        T::Inner: FromBech32Str,
    {
        const HRP: &'static str = T::HRP;

        #[inline]
        fn from_bech32_str(s: &str) -> Result<Self, Error> {
            Ok(Self::new(T::from_inner(T::Inner::from_bech32_str(s)?)))
        }
    }

    impl<T> ToBech32String for Holder<T, UsingStrictEncoding>
    where
        T: StrictEncode + Strategy,
    {
        #[inline]
        fn to_bech32_string(&self) -> String {
            let data = self
                .as_inner()
                .strict_serialize()
                .expect("in-memory strict encoding failure");
            ::bech32::encode(T::HRP, data.to_base32())
                .unwrap_or(s!("Error: wrong bech32 prefix"))
        }
    }

    impl<T> FromBech32Str for Holder<T, UsingStrictEncoding>
    where
        T: StrictDecode + Strategy,
    {
        const HRP: &'static str = T::HRP;

        #[inline]
        fn from_bech32_str(s: &str) -> Result<Self, Error> {
            let (hrp, data) = ::bech32::decode(s)?;
            if hrp.as_str() != Self::HRP {
                return Err(Error::WrongPrefix);
            }
            Ok(Self::new(T::strict_deserialize(Vec::<u8>::from_base32(
                &data,
            )?)?))
        }
    }
}
pub use strategies::Strategy;

// -- Sealed traits & their implementation

/// Special trait for preventing implementation of [`FromBech32DataStr`] and
/// others from outside of this crate. For details see
/// <https://rust-lang.github.io/api-guidelines/future-proofing.html#sealed-traits-protect-against-downstream-implementations-c-sealed>
mod sealed {
    use super::*;
    use amplify::Wrapper;

    pub trait HashType<Tag>: Wrapper<Inner = sha256t::Hash<Tag>>
    where
        Tag: sha256t::Tag,
    {
    }
    pub trait ToPayload: ToBech32Payload {}
    pub trait AsPayload: AsBech32Payload {}
    pub trait FromPayload: FromBech32Payload {}

    impl<T, Tag> HashType<Tag> for T
    where
        T: Wrapper<Inner = sha256t::Hash<Tag>>,
        Tag: sha256t::Tag,
    {
    }
    impl<T> ToPayload for T where T: ToBech32Payload {}
    impl<T> AsPayload for T where T: AsBech32Payload {}
    impl<T> FromPayload for T where T: FromBech32Payload {}
}

pub trait ToBech32DataString: sealed::ToPayload {
    fn to_bech32_data_string(&self) -> String {
        ::bech32::encode(HRP_DATA, self.to_bech32_payload().to_base32())
            .expect("HRP is hardcoded and can't fail")
    }
}

impl<T> ToBech32DataString for T where T: sealed::ToPayload {}

pub trait Bech32DataString: sealed::AsPayload {
    fn bech32_data_string(&self) -> String {
        ::bech32::encode(HRP_DATA, self.as_bech32_payload().to_base32())
            .expect("HRP is hardcoded and can't fail")
    }
}

impl<T> Bech32DataString for T where T: sealed::AsPayload {}

pub trait FromBech32DataStr
where
    Self: Sized + sealed::FromPayload,
{
    fn from_bech32_data_str(s: &str) -> Result<Self, Error> {
        let (hrp, data) = bech32::decode(&s)?;
        if &hrp != HRP_DATA {
            return Err(Error::WrongPrefix);
        }
        Self::from_bech32_payload(Vec::<u8>::from_base32(&data)?)
    }
}

impl<T> FromBech32DataStr for T where T: sealed::FromPayload {}

#[cfg(feature = "zip")]
pub mod zip {
    use super::*;
    use amplify::Holder;
    use strict_encoding::{StrictDecode, StrictEncode};

    fn payload_to_bech32_zip_string(hrp: &str, payload: &[u8]) -> String {
        use std::io::Write;

        // We initialize writer with a version byte, indicating deflation
        // algorithm used
        let writer = vec![RAW_DATA_ENCODING_DEFLATE];
        let mut encoder = DeflateEncoder::new(writer, Compression::Best);
        encoder
            .write(payload)
            .expect("in-memory strict encoder failure");
        let data = encoder.finish().expect("zip algorithm failure");

        ::bech32::encode(hrp, data.to_base32())
            .expect("HRP is hardcoded and can't fail")
    }

    fn bech32_zip_str_to_payload(hrp: &str, s: &str) -> Result<Vec<u8>, Error> {
        use bitcoin::consensus::encode::ReadExt;

        let (prefix, data) = bech32::decode(&s)?;
        if &prefix != hrp {
            return Err(Error::WrongPrefix);
        }
        let data = Vec::<u8>::from_base32(&data)?;
        let mut reader: &[u8] = data.as_ref();
        match reader.read_u8()? {
            RAW_DATA_ENCODING_DEFLATE => {
                let decoded = inflate::inflate_bytes(&mut reader)
                    .map_err(|e| Error::InflateError(e))?;
                Ok(decoded)
            }
            unknown_ver => Err(Error::UnknownRawDataEncoding(unknown_ver))?,
        }
    }

    pub trait ToBech32ZipString: sealed::ToPayload {
        fn to_bech32_zip_string(&self) -> String {
            payload_to_bech32_zip_string(HRP_ZIP, &self.to_bech32_payload())
        }
    }

    impl<T> ToBech32ZipString for T where T: sealed::ToPayload {}

    pub trait Bech32ZipString: sealed::AsPayload {
        fn bech32_zip_string(&self) -> String {
            payload_to_bech32_zip_string(HRP_ZIP, &self.as_bech32_payload())
        }
    }

    impl<T> Bech32ZipString for T where T: sealed::AsPayload {}

    pub trait FromBech32ZipStr: sealed::FromPayload {
        fn from_bech32_zip_str(s: &str) -> Result<Self, Error> {
            Self::from_bech32_payload(bech32_zip_str_to_payload(HRP_ZIP, s)?)
        }
    }

    impl<T> FromBech32ZipStr for T where T: sealed::FromPayload {}

    impl<T> ToBech32String for Holder<T, strategies::CompressedStrictEncoding>
    where
        T: StrictEncode + Strategy,
    {
        #[inline]
        fn to_bech32_string(&self) -> String {
            let data = self
                .as_inner()
                .strict_serialize()
                .expect("in-memory strict encoding failure");
            payload_to_bech32_zip_string(T::HRP, &data)
        }
    }

    impl<T> FromBech32Str for Holder<T, strategies::CompressedStrictEncoding>
    where
        T: StrictDecode + Strategy,
    {
        const HRP: &'static str = T::HRP;

        #[inline]
        fn from_bech32_str(s: &str) -> Result<Self, Error> {
            Ok(Self::new(T::strict_deserialize(
                bech32_zip_str_to_payload(Self::HRP, s)?,
            )?))
        }
    }
}
use std::fmt::Formatter;
use std::marker::PhantomData;
#[cfg(feature = "zip")]
pub use zip::*;

/// Trait representing given bitcoin hash type as a Bech32 `id1...` value
pub trait ToBech32IdString<Tag>
where
    Self: sealed::HashType<Tag>,
    Tag: sha256t::Tag,
{
    /// Returns Bech32-encoded string in form of `id1...` representing the type
    fn to_bech32_id_string(&self) -> String;
}

/// Trait that can generate the type from a given Bech32 `id1...` value
pub trait FromBech32IdStr<Tag>
where
    Self: sealed::HashType<Tag> + Sized,
    Tag: sha256t::Tag,
{
    fn from_bech32_id_str(s: &str) -> Result<Self, Error>;
}

impl<T, Tag> ToBech32IdString<Tag> for T
where
    Self: sealed::HashType<Tag>,
    Tag: sha256t::Tag,
{
    fn to_bech32_id_string(&self) -> String {
        ::bech32::encode(HRP_ID, self.to_inner().to_base32())
            .expect("HRP is hardcoded and can't fail")
    }
}

impl<T, Tag> FromBech32IdStr<Tag> for T
where
    Self: sealed::HashType<Tag>,
    Tag: sha256t::Tag,
{
    fn from_bech32_id_str(s: &str) -> Result<T, Error> {
        let (hrp, id) = ::bech32::decode(&s)?;
        if &hrp != HRP_ID {
            return Err(Error::WrongPrefix);
        }
        let vec = Vec::<u8>::from_base32(&id)?;
        Ok(Self::from_inner(Self::Inner::from_slice(&vec)?))
    }
}

pub fn serialize<T, S>(data: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: ToBech32String,
{
    serializer.serialize_str(&data.to_bech32_string())
}

pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromBech32Str,
{
    deserializer.deserialize_str(Bech32Visitor::<T>(PhantomData))
}

struct Bech32Visitor<Value>(PhantomData<Value>);

impl<'de, ValueT> Visitor<'de> for Bech32Visitor<ValueT>
where
    ValueT: FromBech32Str,
{
    type Value = ValueT;

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str("a bech32m-encoded string")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: SerdeError,
    {
        Self::Value::from_bech32_str(v).map_err(|_| {
            E::invalid_value(Unexpected::Str(v), &"valid bech32 string")
        })
    }
}