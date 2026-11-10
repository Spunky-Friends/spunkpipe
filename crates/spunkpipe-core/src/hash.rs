use crate::{Error, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::io::Read;

/// A BLAKE3 content hash.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hash([u8; 32]);

impl Hash {
    pub fn of(bytes: &[u8]) -> Self {
        Hash(*blake3::hash(bytes).as_bytes())
    }

    pub fn from_reader<R: Read>(reader: R) -> std::io::Result<Self> {
        let mut hasher = blake3::Hasher::new();
        hasher.update_reader(reader)?;
        Ok(Hash(*hasher.finalize().as_bytes()))
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Hash(bytes)
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        let mut out = [0u8; 32];
        if s.len() != 64 {
            return Err(Error::BadHash(s.to_string()));
        }
        hex::decode_to_slice(s, &mut out).map_err(|_| Error::BadHash(s.to_string()))?;
        Ok(Hash(out))
    }

    /// Where the compressed blob for this hash lives, relative to the store root.
    pub fn blob_path(&self) -> String {
        let hex = self.to_hex();
        format!("blobs/{}/{}.zst", &hex[..2], hex)
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", &self.to_hex()[..12])
    }
}

impl Serialize for Hash {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Hash::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let h = Hash::of(b"hello");
        assert_eq!(Hash::from_hex(&h.to_hex()).unwrap(), h);
        assert!(Hash::from_hex("zz").is_err());
        assert!(Hash::from_hex(&"g".repeat(64)).is_err());
    }

    #[test]
    fn reader_matches_bytes() {
        let data = vec![7u8; 100_000];
        assert_eq!(Hash::from_reader(&data[..]).unwrap(), Hash::of(&data));
    }

    #[test]
    fn blob_path_is_sharded() {
        let h = Hash::of(b"x");
        let p = h.blob_path();
        assert!(p.starts_with(&format!("blobs/{}/", &h.to_hex()[..2])));
        assert!(p.ends_with(".zst"));
    }
}
