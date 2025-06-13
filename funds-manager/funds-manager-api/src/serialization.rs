//! Serialization helpers for the funds manager API

/// A module for serializing and deserializing U256 as strings
pub(crate) mod u256_string_serialization {
    use alloy_primitives::U256;
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    /// Serialize a U256 to a string
    pub fn serialize<S: Serializer>(value: &U256, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&value.to_string())
    }

    /// Deserialize a string to a U256
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<U256, D::Error> {
        let s = String::deserialize(d)?;
        U256::from_str_radix(&s, 10).map_err(|_| D::Error::custom("Invalid U256 value"))
    }
}

/// A module for serializing and deserializing f64 as strings
pub(crate) mod f64_string_serialization {
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    /// Serialize an f64 to a string
    pub fn serialize<S: Serializer>(value: &f64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&value.to_string())
    }

    /// Deserialize a string to an f64
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        let s = String::deserialize(d)?;
        s.parse::<f64>().map_err(|_| D::Error::custom("Invalid f64 value"))
    }
}

#[cfg(test)]
mod tests {
    use super::u256_string_serialization;
    use alloy_primitives::U256;
    use rand::{thread_rng, Rng};
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
    struct TestU256 {
        #[serde(with = "u256_string_serialization")]
        value: U256,
    }

    /// Test serialization and deserialization of a U256
    #[test]
    fn test_u256_serialization() {
        let mut rng = thread_rng();
        let mut bytes = [0u8; 32];
        rng.fill(&mut bytes);
        let value = U256::from_be_bytes(bytes);
        let test_value = TestU256 { value };

        let serialized = serde_json::to_string(&test_value).unwrap();
        let deserialized: TestU256 = serde_json::from_str(&serialized).unwrap();
        assert_eq!(test_value, deserialized);
    }
}
