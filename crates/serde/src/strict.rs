use std::{collections::BTreeMap, marker::PhantomData};

/// Require a JSON object and carry strict object decoding into nested values.
pub struct MapOnly<D>(pub D);

// The private newtype marker lets an existing Strict boundary reuse its adapter
// instead of nesting Strict<Strict<...>> at every declared child object.
const OBJECT_MARKER: &str = "$meshway::map-only";

struct ObjectVisitor<V>(V);
impl<'de, V: serde::de::Visitor<'de>> serde::de::Visitor<'de> for ObjectVisitor<V> {
    type Value = V::Value;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an object")
    }
    fn visit_newtype_struct<D: serde::Deserializer<'de>>(
        self,
        d: D,
    ) -> Result<Self::Value, D::Error> {
        d.deserialize_map(self)
    }
    fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
        self.0.visit_map(StrictMap(map, Default::default()))
    }
}

impl<'de, D: serde::Deserializer<'de>> serde::Deserializer<'de> for MapOnly<D> {
    type Error = D::Error;

    fn deserialize_any<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.0
            .deserialize_newtype_struct(OBJECT_MARKER, ObjectVisitor(visitor))
    }

    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

// Carry object-only struct decoding through nested maps and arrays. Serde's
// default struct visitor accepts positional arrays, contrary to our schemas.
struct Strict<D>(D);
impl<'de, D: serde::Deserializer<'de>> serde::Deserializer<'de> for Strict<D> {
    type Error = D::Error;
    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }
    fn deserialize_any<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.deserialize_any(StrictVisitor(visitor))
    }
    fn deserialize_struct<V: serde::de::Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.deserialize_map(StrictVisitor(visitor))
    }
    fn deserialize_map<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.deserialize_map(StrictVisitor(visitor))
    }
    fn deserialize_enum<V: serde::de::Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.deserialize_enum(name, variants, visitor)
    }
    fn deserialize_option<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.deserialize_option(StrictVisitor(visitor))
    }
    fn deserialize_newtype_struct<V: serde::de::Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        if name == OBJECT_MARKER {
            self.0.deserialize_map(visitor)
        } else {
            self.0
                .deserialize_newtype_struct(name, StrictVisitor(visitor))
        }
    }
    serde::forward_to_deserialize_any! {bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf unit unit_struct seq tuple tuple_struct identifier ignored_any}
}
struct StrictSeed<S>(S);
impl<'de, S: serde::de::DeserializeSeed<'de>> serde::de::DeserializeSeed<'de> for StrictSeed<S> {
    type Value = S::Value;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        self.0.deserialize(Strict(deserializer))
    }
}
struct StrictMap<M>(M, std::collections::BTreeSet<String>);
impl<'de, M: serde::de::MapAccess<'de>> serde::de::MapAccess<'de> for StrictMap<M> {
    type Error = M::Error;
    fn next_key_seed<K: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        let Some(key) = self.0.next_key::<String>()? else {
            return Ok(None);
        };
        if !self.1.insert(key.clone()) {
            return Err(serde::de::Error::custom("duplicate object key"));
        }
        seed.deserialize(Strict(
            serde::de::value::StringDeserializer::<Self::Error>::new(key),
        ))
        .map(Some)
    }
    fn next_value_seed<V: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.next_value_seed(StrictSeed(seed))
    }
    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}
struct StrictSeq<S>(S);
impl<'de, S: serde::de::SeqAccess<'de>> serde::de::SeqAccess<'de> for StrictSeq<S> {
    type Error = S::Error;
    fn next_element_seed<T: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        self.0.next_element_seed(StrictSeed(seed))
    }
    fn size_hint(&self) -> Option<usize> {
        self.0.size_hint()
    }
}
struct StrictVisitor<V>(V);
macro_rules! visit_scalar {
    ($($method:ident($ty:ty)),* $(,)?) => {
        $(
            fn $method<E: serde::de::Error>(self, value: $ty) -> Result<Self::Value, E> {
                self.0.$method(value)
            }
        )*
    };
}
impl<'de, V: serde::de::Visitor<'de>> serde::de::Visitor<'de> for StrictVisitor<V> {
    type Value = V::Value;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.expecting(f)
    }
    visit_scalar! {
        visit_bool(bool), visit_i8(i8), visit_i16(i16), visit_i32(i32), visit_i64(i64), visit_i128(i128),
        visit_u8(u8), visit_u16(u16), visit_u32(u32), visit_u64(u64), visit_u128(u128),
        visit_f32(f32), visit_f64(f64), visit_char(char), visit_str(&str),
        visit_borrowed_str(&'de str), visit_string(String), visit_bytes(&[u8]),
        visit_borrowed_bytes(&'de [u8]), visit_byte_buf(Vec<u8>),
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        self.0.visit_none()
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        self.0.visit_unit()
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        self.0.visit_some(Strict(d))
    }
    fn visit_newtype_struct<D: serde::Deserializer<'de>>(
        self,
        d: D,
    ) -> Result<Self::Value, D::Error> {
        self.0.visit_newtype_struct(Strict(d))
    }
    fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
        self.0.visit_map(StrictMap(map, Default::default()))
    }
    fn visit_seq<S: serde::de::SeqAccess<'de>>(self, seq: S) -> Result<Self::Value, S::Error> {
        self.0.visit_seq(StrictSeq(seq))
    }
}

/// Reject both duplicate JSON keys and collisions after typed key decoding.
pub fn unique_map<'de, D, K, V>(d: D) -> Result<BTreeMap<K, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    K: serde::Deserialize<'de> + Ord,
    V: serde::Deserialize<'de>,
{
    struct Entries<K, V>(PhantomData<(K, V)>);
    impl<'de, K: serde::Deserialize<'de> + Ord, V: serde::Deserialize<'de>> serde::de::Visitor<'de>
        for Entries<K, V>
    {
        type Value = BTreeMap<K, V>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an object with unique keys")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut values = BTreeMap::new();
            while let Some((key, value)) = map.next_entry()? {
                if values.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate map key"));
                }
            }
            Ok(values)
        }
    }
    serde::Deserializer::deserialize_map(MapOnly(d), Entries(PhantomData))
}
