//! A formatter which stringifies all numbers in a response

use std::{
    fmt::Display,
    io::{self},
    str::FromStr,
};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{
    Number, Value,
    ser::{CompactFormatter, Formatter, Serializer},
};

use crate::error::AuthServerError;

/// Keys to ignore when converting stringified numbers in a deserialization
const IGNORED_KEYS: [&str; 1] = ["price"];

// --------------
// | Serializer |
// --------------

/// Serialize a value to json, possibly stringifying all numbers
pub(crate) fn json_serialize<T: Serialize>(
    value: &T,
    stringify: bool,
) -> Result<Vec<u8>, AuthServerError> {
    if stringify {
        let mut buf = Vec::new();
        let mut ser = Serializer::with_formatter(&mut buf, StringifyNumbersFormatter::default());
        value.serialize(&mut ser).map_err(AuthServerError::serde)?;
        Ok(buf)
    } else {
        serde_json::to_vec(&value).map_err(AuthServerError::serde)
    }
}

/// A helper to write an escaped string to a writer
fn write_escaped_string<W, T>(writer: &mut W, value: T) -> io::Result<()>
where
    T: Display,
    W: ?Sized + io::Write,
{
    write!(writer, "\"{}\"", value)
}

/// A formatter which stringifies all numbers in a response
struct StringifyNumbersFormatter<F: Formatter = CompactFormatter>(F);
impl<F: Formatter> Formatter for StringifyNumbersFormatter<F> {
    // --- Number Types --- //
    fn write_i8<W>(&mut self, writer: &mut W, value: i8) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        write_escaped_string(writer, value)
    }

    fn write_i16<W>(&mut self, writer: &mut W, value: i16) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        write_escaped_string(writer, value)
    }

    fn write_i32<W>(&mut self, writer: &mut W, value: i32) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        write_escaped_string(writer, value)
    }

    fn write_i64<W>(&mut self, writer: &mut W, value: i64) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        write_escaped_string(writer, value)
    }

    fn write_i128<W>(&mut self, writer: &mut W, value: i128) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        write_escaped_string(writer, value)
    }

    fn write_u8<W>(&mut self, writer: &mut W, value: u8) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        write_escaped_string(writer, value)
    }

    fn write_u16<W>(&mut self, writer: &mut W, value: u16) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        write_escaped_string(writer, value)
    }

    fn write_u32<W>(&mut self, writer: &mut W, value: u32) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        write_escaped_string(writer, value)
    }

    fn write_u64<W>(&mut self, writer: &mut W, value: u64) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        write_escaped_string(writer, value)
    }

    fn write_u128<W>(&mut self, writer: &mut W, value: u128) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        write_escaped_string(writer, value)
    }

    fn write_f32<W>(&mut self, writer: &mut W, value: f32) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        write_escaped_string(writer, value)
    }

    fn write_f64<W>(&mut self, writer: &mut W, value: f64) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        write_escaped_string(writer, value)
    }

    // --- JSON Passthrough Types --- //
    fn write_null<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        self.0.write_null(writer)
    }

    fn write_bool<W>(&mut self, writer: &mut W, value: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        self.0.write_bool(writer, value)
    }

    fn begin_string<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        self.0.begin_string(writer)
    }

    fn end_string<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        self.0.end_string(writer)
    }
}

impl Default for StringifyNumbersFormatter {
    fn default() -> Self {
        Self(CompactFormatter)
    }
}

// ----------------
// | Deserializer |
// ----------------

/// Deserialize a value from json, possibly parsing stringified numbers
pub(crate) fn json_deserialize<T: DeserializeOwned>(
    buf: &[u8],
    stringify: bool,
) -> Result<T, AuthServerError> {
    if stringify {
        let mut val: Value = serde_json::from_slice(buf).map_err(AuthServerError::serde)?;
        convert_stringified_numbers(&mut val)?;
        serde_json::from_value(val).map_err(AuthServerError::serde)
    } else {
        serde_json::from_slice(buf).map_err(AuthServerError::serde)
    }
}

/// Convert all the stringified numbers in a struct into numbers
fn convert_stringified_numbers(val: &mut Value) -> Result<(), AuthServerError> {
    match val {
        // If we see a string, check if that string represents a number.
        // If it does, we convert it to a number. Under the hood, `serde_json` uses
        // a string type to represent arbitrary precision numbers, so we don't actually need
        // the parsed value, we just need to annotate it as a number, and the deserializer will
        // handle it correctly.
        Value::String(s) => {
            // Try parsing a number
            if is_numeric(s) {
                let num = Number::from_str(s).map_err(AuthServerError::serde)?;
                *val = Value::Number(num);
            }
        },

        // Recurse into objects and arrays
        Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                if should_ignore_key(key.as_str(), value) {
                    continue;
                }

                convert_stringified_numbers(value)?;
            }
        },
        Value::Array(arr) => {
            for value in arr.iter_mut() {
                convert_stringified_numbers(value)?;
            }
        },
        _ => {},
    }
    Ok(())
}

/// Returns whether the given string represents a number
///
/// This can be an integer or floating point value
fn is_numeric(s: &str) -> bool {
    s.parse::<f64>().is_ok()
}

/// Whether a key should be ignored when converting stringified numbers in a
/// deserialization
fn should_ignore_key(key: &str, value: &Value) -> bool {
    // Only ignore keys which directly correspond to a possibly stringified number
    if value.is_object() || value.is_array() {
        return false;
    }

    IGNORED_KEYS.contains(&key)
}

#[cfg(test)]
mod test {
    use eyre::Result;
    use rand::{Rng, SeedableRng, rngs::StdRng};
    use serde::Deserialize;

    use super::*;

    /// Number of test iterations to run
    const NUM_TEST_ITERATIONS: u64 = 1000;

    // --------------
    // | Test Types |
    // --------------

    /// An embedded struct used for testing
    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct EmbeddedStruct {
        /// A u128
        a: u128,
        /// A bool
        b: bool,
        /// An f64
        c: f64,
        /// An array of u128s
        d: [u128; 10],
    }

    /// The embedded struct with all values converted to strings
    #[derive(Deserialize)]
    struct StringifiedEmbeddedStruct {
        /// A u128
        a: String,
        /// A bool
        b: bool,
        /// An f64
        c: String,
        /// An array of u128s
        d: [String; 10],
    }

    /// A struct used for testing
    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct TestStruct {
        /// A u128
        a: u128,
        /// A u64
        b: u64,
        /// A floating point
        c: f64,
        /// A bool
        d: bool,
        /// A string
        e: String,
        /// An array of u128s
        f: Vec<u128>,
        /// A struct
        g: EmbeddedStruct,
    }

    /// The stringified version of the test struct
    #[derive(Deserialize)]
    struct StringifiedTestStruct {
        /// A u128
        a: String,
        /// A u64
        b: String,
        /// A floating point
        c: String,
        /// A bool
        d: bool,
        /// A string
        e: String,
        /// An array of u128s
        f: Vec<String>,
        /// A struct
        g: StringifiedEmbeddedStruct,
    }

    impl TestStruct {
        /// Create a new pseudorandom test struct from a seed
        fn new(seed: u64) -> Self {
            let mut rng = StdRng::seed_from_u64(seed);
            Self {
                a: rng.r#gen(),
                b: rng.r#gen(),
                c: rng.r#gen(),
                d: rng.r#gen(),
                e: "test".to_string(),
                f: (0..10).map(|_| rng.r#gen()).collect(),
                g: EmbeddedStruct {
                    a: rng.r#gen(),
                    b: rng.r#gen(),
                    c: rng.r#gen(),
                    d: [rng.r#gen(); 10],
                },
            }
        }
    }

    // ---------
    // | Tests |
    // ---------

    /// Tests serializing a struct without stringifying the numbers
    #[test]
    fn test_json_serialize() -> Result<()> {
        for seed in 0..NUM_TEST_ITERATIONS {
            let test_struct = TestStruct::new(seed);
            let json_buf = json_serialize(&test_struct, false /* stringify */)?;
            let deser: TestStruct = serde_json::from_slice(&json_buf)?;

            assert_eq!(test_struct, deser);
        }
        Ok(())
    }

    /// Tests stringifying the numbers in a response
    #[test]
    fn test_stringify_numbers_formatter() -> Result<()> {
        for seed in 0..NUM_TEST_ITERATIONS {
            let test_struct = TestStruct::new(seed);
            let json_buf = json_serialize(&test_struct, true /* stringify */)?;
            let stringified_deser: StringifiedTestStruct = serde_json::from_slice(&json_buf)?;

            assert_eq!(test_struct.a, stringified_deser.a.parse::<u128>().unwrap());
            assert_eq!(test_struct.b, stringified_deser.b.parse::<u64>().unwrap());
            assert_eq!(test_struct.c, stringified_deser.c.parse::<f64>().unwrap());
            assert_eq!(test_struct.d, stringified_deser.d);
            assert_eq!(test_struct.e, stringified_deser.e);

            let g_parsed: Vec<u128> =
                stringified_deser.f.iter().map(|s| s.parse::<u128>().unwrap()).collect();
            assert_eq!(test_struct.f.clone(), g_parsed);
            assert_eq!(test_struct.g.a, stringified_deser.g.a.parse::<u128>().unwrap());
            assert_eq!(test_struct.g.b, stringified_deser.g.b);
            assert_eq!(test_struct.g.c, stringified_deser.g.c.parse::<f64>().unwrap());

            let gd_parsed: Vec<u128> =
                stringified_deser.g.d.iter().map(|s| s.parse::<u128>().unwrap()).collect();
            let gd_parsed_array: [u128; 10] = gd_parsed.try_into().unwrap();
            assert_eq!(test_struct.g.d, gd_parsed_array);
        }

        Ok(())
    }

    /// Tests deserializing a struct without stringifying the numbers
    #[test]
    fn test_json_deserialize() -> Result<()> {
        for seed in 0..NUM_TEST_ITERATIONS {
            let test_struct = TestStruct::new(seed);
            let json_buf = json_serialize(&test_struct, false /* stringify */)?;
            let deser: TestStruct = json_deserialize(&json_buf, false /* stringify */)?;
            assert_eq!(test_struct, deser);
        }
        Ok(())
    }

    /// Tests deserializing a struct with stringified numbers
    #[test]
    fn test_json_deserialize_stringify() -> Result<()> {
        for seed in 0..NUM_TEST_ITERATIONS {
            let test_struct = TestStruct::new(seed);
            let json_buf = json_serialize(&test_struct, true /* stringify */)?;
            let deser: TestStruct = json_deserialize(&json_buf, true /* stringify */)?;
            assert_eq!(test_struct, deser);
        }
        Ok(())
    }

    /// Diagnostic test: isolates f64 precision loss caused by serde_json's `arbitrary_precision`
    /// feature if `float_roundtrip` is absent.
    #[test]
    fn test_f64_arbitrary_precision_roundtrip() {
        let val = 0.37652320722764565_f64;

        // Path A: plain serde_json roundtrip (affected by arbitrary_precision)
        let json = serde_json::to_string(&val).unwrap();
        let parsed: f64 = serde_json::from_str(&json).unwrap();

        println!("JSON representation: {json}");
        println!("Original bits:   {:064b}", val.to_bits());
        println!("Parsed bits:     {:064b}", parsed.to_bits());
        println!("Bits match: {}", val.to_bits() == parsed.to_bits());

        // Path B: Rust's own f64 parsing (not affected by arbitrary_precision)
        let rust_parsed: f64 = json.parse().unwrap();
        println!("Rust parse bits: {:064b}", rust_parsed.to_bits());
        println!("Rust parse matches original: {}", val.to_bits() == rust_parsed.to_bits());

        assert_eq!(
            val.to_bits(),
            parsed.to_bits(),
            "serde_json roundtrip changed f64 bits: {val} -> {parsed}"
        );
    }
}
