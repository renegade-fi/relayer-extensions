//! A formatter which stringifies all numbers in a response

use std::{fmt::Display, io};

use serde::Serialize;
use serde_json::ser::{CompactFormatter, Formatter, Serializer};

use crate::error::AuthServerError;

/// Serialize a value to json, possibly stringifying all numbers
pub(crate) fn json_serialize<T: Serialize>(
    value: &T,
    stringify: bool,
) -> Result<String, AuthServerError> {
    if stringify {
        let mut buf = Vec::new();
        let mut ser = Serializer::with_formatter(&mut buf, StringifyNumbersFormatter::default());
        value.serialize(&mut ser).unwrap();
        String::from_utf8(buf).map_err(AuthServerError::serde)
    } else {
        serde_json::to_string(&value).map_err(AuthServerError::serde)
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

#[cfg(test)]
mod test {
    use eyre::Result;
    use rand::Rng;
    use serde::Deserialize;

    use super::*;

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
        /// Create a new random test struct
        fn new() -> Self {
            let mut rng = rand::thread_rng();
            Self {
                a: rng.gen(),
                b: rng.gen(),
                c: rng.gen(),
                d: rng.gen(),
                e: "test".to_string(),
                f: (0..10).map(|_| rng.gen()).collect(),
                g: EmbeddedStruct { a: rng.gen(), b: rng.gen(), c: rng.gen(), d: [rng.gen(); 10] },
            }
        }
    }

    // ---------
    // | Tests |
    // ---------

    /// Tests serializing a struct without stringifying the numbers
    #[test]
    fn test_json_serialize() -> Result<()> {
        let test_struct = TestStruct::new();
        let json = json_serialize(&test_struct, false /* stringify */)?;
        let deser: TestStruct = serde_json::from_str(&json)?;
        assert_eq!(test_struct, deser);
        Ok(())
    }

    /// Tests stringifying the numbers in a response
    #[test]
    fn test_stringify_numbers_formatter() -> Result<()> {
        let test_struct = TestStruct::new();
        let json = json_serialize(&test_struct, true /* stringify */)?;
        let stringified_deser: StringifiedTestStruct = serde_json::from_str(&json)?;

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

        Ok(())
    }
}
