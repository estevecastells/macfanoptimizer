//! Safe wrapper around the Apple SMC.
//!
//! Reading keys works for any user. Writing keys (fan control) requires root.

mod ffi;
mod value;

pub use value::{decode, encode, Value};

use std::fmt;

/// A four-character SMC key or data type, e.g. `F0Ac` or `flt `.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FourCC(pub u32);

impl FourCC {
    pub const fn from_bytes(b: [u8; 4]) -> Self {
        FourCC(u32::from_be_bytes(b))
    }

    pub fn bytes(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }
}

impl std::str::FromStr for FourCC {
    type Err = Error;
    fn from_str(s: &str) -> std::result::Result<Self, Error> {
        let b = s.as_bytes();
        if b.len() != 4 || !b.is_ascii() {
            return Err(Error::InvalidKey(s.to_string()));
        }
        Ok(FourCC::from_bytes([b[0], b[1], b[2], b[3]]))
    }
}

impl fmt::Display for FourCC {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in self.bytes() {
            let c = if c.is_ascii_graphic() || c == b' ' { c as char } else { '?' };
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for FourCC {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{self}'")
    }
}

/// Convenience: `key!("F0Ac")`-style construction from a string literal.
pub fn key(s: &str) -> FourCC {
    s.parse().expect("SMC keys are exactly four ASCII characters")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInfo {
    pub size: u32,
    pub data_type: FourCC,
    pub attributes: u8,
}

#[derive(Debug)]
pub enum Error {
    ServiceNotFound,
    Open(i32),
    Call {
        key: FourCC,
        kr: i32,
    },
    /// SMC firmware result code (e.g. 0x84 = key not found).
    Smc {
        key: FourCC,
        code: u8,
    },
    InvalidKey(String),
    SizeMismatch {
        key: FourCC,
        expected: u32,
        got: usize,
    },
}

impl Error {
    /// True when the key simply doesn't exist on this machine.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::Smc { code: 0x84, .. })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ServiceNotFound => write!(f, "AppleSMC service not found"),
            Error::Open(kr) => write!(f, "IOServiceOpen failed: {kr:#x}"),
            Error::Call { key, kr } => {
                let hint = if *kr == 0xe00002c1u32 as i32 { " (not privileged; run as root)" } else { "" };
                write!(f, "SMC call for {key} failed: {kr:#x}{hint}")
            }
            Error::Smc { key, code } => write!(f, "SMC returned {code:#x} for {key}"),
            Error::InvalidKey(s) => write!(f, "invalid SMC key {s:?}"),
            Error::SizeMismatch { key, expected, got } => {
                write!(f, "{key}: expected {expected} bytes, got {got}")
            }
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// An open connection to the AppleSMC user client.
pub struct Smc {
    conn: ffi::io_connect_t,
    /// Key info rarely changes; caching it halves the number of kernel calls per read.
    info_cache: std::collections::HashMap<FourCC, KeyInfo>,
}

// The connection is a mach port; it is safe to move between threads (not to share).
unsafe impl Send for Smc {}

impl Smc {
    pub fn open() -> Result<Self> {
        unsafe {
            let matching = ffi::IOServiceMatching(c"AppleSMC".as_ptr());
            let service = ffi::IOServiceGetMatchingService(ffi::kIOMainPortDefault, matching);
            if service == 0 {
                return Err(Error::ServiceNotFound);
            }
            let mut conn = 0;
            let kr = ffi::IOServiceOpen(service, ffi::mach_task_self_, 0, &mut conn);
            ffi::IOObjectRelease(service);
            if kr != ffi::KERN_SUCCESS {
                return Err(Error::Open(kr));
            }
            Ok(Smc { conn, info_cache: Default::default() })
        }
    }

    fn call(&self, key: FourCC, input: &ffi::SMCParamStruct) -> Result<ffi::SMCParamStruct> {
        let mut output = ffi::SMCParamStruct::default();
        let mut out_size = std::mem::size_of::<ffi::SMCParamStruct>();
        let kr = unsafe {
            ffi::IOConnectCallStructMethod(
                self.conn,
                ffi::KERNEL_INDEX_SMC,
                input as *const _ as *const _,
                std::mem::size_of::<ffi::SMCParamStruct>(),
                &mut output as *mut _ as *mut _,
                &mut out_size,
            )
        };
        if kr != ffi::KERN_SUCCESS {
            return Err(Error::Call { key, kr });
        }
        if output.result != 0 {
            return Err(Error::Smc { key, code: output.result });
        }
        Ok(output)
    }

    pub fn key_info(&mut self, key: FourCC) -> Result<KeyInfo> {
        if let Some(info) = self.info_cache.get(&key) {
            return Ok(*info);
        }
        let input = ffi::SMCParamStruct { key: key.0, data8: ffi::SMC_CMD_READ_KEYINFO, ..Default::default() };
        let out = self.call(key, &input)?;
        let info = KeyInfo {
            size: out.key_info.data_size,
            data_type: FourCC(out.key_info.data_type),
            attributes: out.key_info.data_attributes,
        };
        self.info_cache.insert(key, info);
        Ok(info)
    }

    /// Read the raw bytes of a key along with its type information.
    pub fn read_raw(&mut self, key: FourCC) -> Result<(KeyInfo, Vec<u8>)> {
        let info = self.key_info(key)?;
        let mut input = ffi::SMCParamStruct { key: key.0, data8: ffi::SMC_CMD_READ_BYTES, ..Default::default() };
        input.key_info.data_size = info.size;
        let out = self.call(key, &input)?;
        let n = (info.size as usize).min(32);
        Ok((info, out.bytes[..n].to_vec()))
    }

    /// Read and decode a key.
    pub fn read(&mut self, key: FourCC) -> Result<Value> {
        let (info, bytes) = self.read_raw(key)?;
        Ok(decode(info.data_type, &bytes))
    }

    /// Read a key as a number, whatever its numeric encoding.
    pub fn read_f64(&mut self, key: FourCC) -> Result<f64> {
        let v = self.read(key)?;
        v.as_f64().ok_or(Error::InvalidKey(format!("{key} is not numeric ({v:?})")))
    }

    /// Write raw bytes. Requires root.
    pub fn write_raw(&mut self, key: FourCC, bytes: &[u8]) -> Result<()> {
        let info = self.key_info(key)?;
        if bytes.len() != info.size as usize {
            return Err(Error::SizeMismatch { key, expected: info.size, got: bytes.len() });
        }
        let mut input = ffi::SMCParamStruct { key: key.0, data8: ffi::SMC_CMD_WRITE_BYTES, ..Default::default() };
        input.key_info.data_size = info.size;
        input.bytes[..bytes.len()].copy_from_slice(bytes);
        self.call(key, &input).map(|_| ())
    }

    /// Encode a number using the key's native type and write it. Requires root.
    pub fn write_f64(&mut self, key: FourCC, v: f64) -> Result<()> {
        let info = self.key_info(key)?;
        let bytes = encode(info.data_type, info.size as usize, v)
            .ok_or(Error::InvalidKey(format!("{key}: cannot encode into type {}", info.data_type)))?;
        self.write_raw(key, &bytes)
    }

    /// Total number of keys exposed by the SMC.
    pub fn key_count(&mut self) -> Result<u32> {
        match self.read(key("#KEY"))? {
            Value::Unsigned(n) => Ok(n as u32),
            v => Err(Error::InvalidKey(format!("#KEY has unexpected value {v:?}"))),
        }
    }

    pub fn key_at_index(&mut self, index: u32) -> Result<FourCC> {
        let input = ffi::SMCParamStruct { data8: ffi::SMC_CMD_READ_INDEX, data32: index, ..Default::default() };
        let out = self.call(FourCC(0), &input)?;
        Ok(FourCC(out.key))
    }

    /// Enumerate every key. Takes a few hundred milliseconds; cache the result.
    pub fn all_keys(&mut self) -> Result<Vec<FourCC>> {
        let n = self.key_count()?;
        (0..n).map(|i| self.key_at_index(i)).collect()
    }
}

impl Drop for Smc {
    fn drop(&mut self) {
        unsafe {
            ffi::IOServiceClose(self.conn);
        }
    }
}
