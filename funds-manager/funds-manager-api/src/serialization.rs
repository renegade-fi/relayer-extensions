//! Serialization helpers for the funds manager API

/// A module for serializing and deserializing addresses as strings
pub(crate) mod address_string_serialization {
    use std::str::FromStr;

    use ethers::types::Address;
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    /// Serialize an address to a string
    pub fn serialize<S: Serializer>(address: &Address, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{address:#x}"))
    }

    /// Deserialize a string to an address
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Address, D::Error> {
        let s = String::deserialize(d)?;
        Address::from_str(&s).map_err(|_| D::Error::custom("Invalid address"))
    }
}

/// A module for serializing and deserializing U256 as strings
pub(crate) mod u256_string_serialization {
    use ethers::types::U256;
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    /// Serialize a U256 to a string
    pub fn serialize<S: Serializer>(value: &U256, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&value.to_string())
    }

    /// Deserialize a string to a U256
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<U256, D::Error> {
        let s = String::deserialize(d)?;
        U256::from_dec_str(&s).map_err(|_| D::Error::custom("Invalid U256 value"))
    }
}

/// A module for serializing and deserializing bytes from a hex string
pub(crate) mod bytes_string_serialization {
    use ethers::types::Bytes;
    use hex::FromHex;
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    /// Serialize bytes to a hex string
    pub fn serialize<S: Serializer>(value: &Bytes, s: S) -> Result<S::Ok, S::Error> {
        let hex = format!("{value:#x}");
        s.serialize_str(&hex)
    }

    /// Deserialize a hex string to bytes
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Bytes, D::Error> {
        let s = String::deserialize(d)?;
        Bytes::from_hex(s).map_err(|_| D::Error::custom("Invalid bytes value"))
    }
}

#[cfg(test)]
mod tests {
    use ethers::types::{Address, Bytes, U256};
    use rand::{thread_rng, Rng};

    /// Test serialization and deserialization of an address
    #[test]
    fn test_address_serialization() {
        let addr = Address::random();
        let serialized = serde_json::to_string(&addr).unwrap();
        let deserialized: Address = serde_json::from_str(&serialized).unwrap();
        assert_eq!(addr, deserialized);
    }

    /// Test serialization and deserialization of a U256
    #[test]
    fn test_u256_serialization() {
        let mut rng = thread_rng();
        let mut bytes = [0u8; 32];
        rng.fill(&mut bytes);
        let value = U256::from(bytes);

        let serialized = serde_json::to_string(&value).unwrap();
        let deserialized: U256 = serde_json::from_str(&serialized).unwrap();
        assert_eq!(value, deserialized);
    }

    /// Test serialization and deserialization of bytes
    #[test]
    fn test_bytes_serialization() {
        const N: usize = 32;
        let mut rng = thread_rng();
        let bytes: Bytes = (0..N).map(|_| rng.gen_range(0..=u8::MAX)).collect();

        let serialized = serde_json::to_string(&bytes).unwrap();
        let deserialized: Bytes = serde_json::from_str(&serialized).unwrap();
        assert_eq!(bytes, deserialized);
    }
}
