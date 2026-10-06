//! Decoding and encoding of SMC data types.
//!
//! Apple Silicon uses little-endian `flt ` for most sensor and fan values;
//! Intel-era fixed-point (`fpe2`, `sp78`, ...) and integer types are big-endian.

use crate::FourCC;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Float(f64),
    Unsigned(u64),
    Signed(i64),
    Flag(bool),
    Text(String),
    Bytes(Vec<u8>),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Value::Float(v) => Some(v),
            Value::Unsigned(v) => Some(v as f64),
            Value::Signed(v) => Some(v as f64),
            Value::Flag(b) => Some(b as u8 as f64),
            _ => None,
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Float(v) => write!(f, "{v:.3}"),
            Value::Unsigned(v) => write!(f, "{v}"),
            Value::Signed(v) => write!(f, "{v}"),
            Value::Flag(b) => write!(f, "{b}"),
            Value::Text(s) => write!(f, "{s:?}"),
            Value::Bytes(b) => {
                for x in b {
                    write!(f, "{x:02x}")?;
                }
                Ok(())
            }
        }
    }
}

fn hex_digit(c: u8) -> Option<u32> {
    (c as char).to_digit(16)
}

fn be_uint(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |acc, &x| (acc << 8) | x as u64)
}

pub fn decode(ty: FourCC, b: &[u8]) -> Value {
    let t = ty.bytes();
    match (&t, b.len()) {
        (b"flt ", 4) => Value::Float(f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
        (b"flt ", 8) => Value::Float(f64::from_le_bytes(b.try_into().unwrap())),
        (b"ioft", 8) => Value::Float(u64::from_le_bytes(b.try_into().unwrap()) as f64 / 65536.0),
        (b"flag", 1) => Value::Flag(b[0] != 0),
        (b"ui8 " | b"ui16" | b"ui32" | b"ui64", 1..=8) => Value::Unsigned(be_uint(b)),
        (b"si8 ", 1) => Value::Signed(b[0] as i8 as i64),
        (b"si16", 2) => Value::Signed(i16::from_be_bytes([b[0], b[1]]) as i64),
        (b"si32", 4) => Value::Signed(i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64),
        (b"si64", 8) => Value::Signed(i64::from_be_bytes(b.try_into().unwrap())),
        ([b'f', b'p', i, f], 2) if hex_digit(*i).is_some() && hex_digit(*f).is_some() => {
            let frac = hex_digit(*f).unwrap();
            Value::Float(u16::from_be_bytes([b[0], b[1]]) as f64 / (1u32 << frac) as f64)
        }
        ([b's', b'p', i, f], 2) if hex_digit(*i).is_some() && hex_digit(*f).is_some() => {
            let frac = hex_digit(*f).unwrap();
            Value::Float(i16::from_be_bytes([b[0], b[1]]) as f64 / (1u32 << frac) as f64)
        }
        ([b'c', b'h', b'8', b'*'], _) => Value::Text(String::from_utf8_lossy(b).trim_end_matches('\0').to_string()),
        _ => Value::Bytes(b.to_vec()),
    }
}

/// Encode `v` as type `ty` occupying `size` bytes. Returns `None` for
/// types we don't know how to write.
pub fn encode(ty: FourCC, size: usize, v: f64) -> Option<Vec<u8>> {
    let t = ty.bytes();
    let out = match (&t, size) {
        (b"flt ", 4) => (v as f32).to_le_bytes().to_vec(),
        (b"flag", 1) => vec![(v != 0.0) as u8],
        (b"ui8 " | b"ui16" | b"ui32" | b"ui64", 1..=8) => {
            let n = v.round().max(0.0) as u64;
            n.to_be_bytes()[8 - size..].to_vec()
        }
        ([b'f', b'p', _, f], 2) => {
            let frac = hex_digit(*f)?;
            ((v * (1u32 << frac) as f64).round().clamp(0.0, u16::MAX as f64) as u16).to_be_bytes().to_vec()
        }
        ([b's', b'p', _, f], 2) => {
            let frac = hex_digit(*f)?;
            ((v * (1u32 << frac) as f64).round().clamp(i16::MIN as f64, i16::MAX as f64) as i16).to_be_bytes().to_vec()
        }
        _ => return None,
    };
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key;

    #[test]
    fn flt_roundtrip() {
        let bytes = encode(key("flt "), 4, 2317.5).unwrap();
        assert_eq!(decode(key("flt "), &bytes), Value::Float(2317.5));
    }

    #[test]
    fn fpe2_decodes_intel_fan_speed() {
        // 0x2328 >> 2 = 2250 rpm
        assert_eq!(decode(key("fpe2"), &[0x23, 0x28]), Value::Float(2250.0));
        assert_eq!(encode(key("fpe2"), 2, 2250.0).unwrap(), vec![0x23, 0x28]);
    }

    #[test]
    fn sp78_decodes_negative_and_positive() {
        assert_eq!(decode(key("sp78"), &[0x32, 0x80]), Value::Float(50.5));
        assert_eq!(decode(key("sp78"), &[0xff, 0x00]), Value::Float(-1.0));
    }

    #[test]
    fn integers_are_big_endian() {
        assert_eq!(decode(key("ui32"), &[0, 0, 0x05, 0x39]), Value::Unsigned(1337));
        assert_eq!(encode(key("ui8 "), 1, 1.0).unwrap(), vec![1]);
        assert_eq!(encode(key("ui16"), 2, 258.0).unwrap(), vec![1, 2]);
    }
}
