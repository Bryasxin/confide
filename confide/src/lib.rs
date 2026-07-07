pub use confide_impl::confide;
pub use humantime_serde;

pub mod bytesize_serde {
    use bytesize::ByteSize;
    use serde::{Deserializer, Serialize, Serializer, de};
    use std::fmt;

    #[doc(hidden)]
    pub trait BytesizeConvert: Copy {
        fn to_u64(self) -> u64;
        fn from_u64(v: u64) -> Self;
    }

    macro_rules! impl_bytesize_convert {
        ($($ty:ty),*) => {
            $(impl BytesizeConvert for $ty {
                #[inline]
                fn to_u64(self) -> u64 { self as u64 }
                #[inline]
                fn from_u64(v: u64) -> Self { v as Self }
            })*
        };
    }

    impl_bytesize_convert!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

    pub fn serialize<T: BytesizeConvert, S: Serializer>(val: &T, s: S) -> Result<S::Ok, S::Error> {
        ByteSize::b(val.to_u64())
            .display()
            .iec()
            .to_string()
            .serialize(s)
    }

    pub fn deserialize<'de, T: BytesizeConvert, D: Deserializer<'de>>(d: D) -> Result<T, D::Error> {
        struct V;
        impl<'de2> de::Visitor<'de2> for V {
            type Value = u64;

            fn expecting(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
                fmt.write_str("a byte size string like '1MiB' or an integer")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<u64, E> {
                Ok(v)
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<u64, E> {
                u64::try_from(v).map_err(|_| {
                    E::invalid_value(de::Unexpected::Signed(v), &"non-negative integer")
                })
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<u64, E> {
                v.parse::<ByteSize>()
                    .map(|bs| bs.as_u64())
                    .map_err(|_| E::invalid_value(de::Unexpected::Str(v), &self))
            }
        }

        let val = if d.is_human_readable() {
            d.deserialize_any(V)?
        } else {
            d.deserialize_u64(V)?
        };
        Ok(T::from_u64(val))
    }
}
