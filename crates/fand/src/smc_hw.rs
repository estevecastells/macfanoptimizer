//! Real hardware backend: fans and temperature sensors through the SMC.

use fan_core::sensors::SensorKey;
use fan_core::{FanInfo, FanReading, Hardware};
use smc::{FourCC, Smc};

/// Per-fan SMC keys, precomputed so the hot path never formats strings.
struct FanKeys {
    actual: FourCC,
    target: FourCC,
    mode: FourCC,
}

pub struct SmcHardware {
    smc: Smc,
    fans: Vec<FanInfo>,
    keys: Vec<FanKeys>,
    temps: Vec<SensorKey>,
    /// Older Apple Silicon (M1–M4 on recent macOS) requires `Ftst=1` before
    /// manual control sticks. The M5 doesn't have this key.
    unlock_key: Option<FourCC>,
    /// When false, writes are logged instead of performed.
    write_enabled: bool,
    /// Dry-run: values we pretended to write, returned by later reads so the
    /// engine behaves exactly as it would on a real install.
    shadow: std::collections::HashMap<FourCC, f64>,
}

fn fan_key(i: u8, suffix: &str) -> FourCC {
    smc::key(&format!("F{i}{suffix}"))
}

impl SmcHardware {
    /// Open the SMC and discover fans and temperature sensors.
    pub fn open(write_enabled: bool) -> Result<Self, String> {
        let mut smc = Smc::open().map_err(|e| e.to_string())?;

        let count = smc.read_f64(smc::key("FNum")).map_err(|e| format!("reading fan count: {e}"))? as u8;
        let mut fans = Vec::new();
        let mut keys = Vec::new();
        for i in 0..count {
            let min_rpm = smc.read_f64(fan_key(i, "Mn")).map_err(|e| e.to_string())?;
            let max_rpm = smc.read_f64(fan_key(i, "Mx")).map_err(|e| e.to_string())?;
            // Apple Silicon uses `F0md`; Intel Macs used `F0Md`.
            let mode = [fan_key(i, "md"), fan_key(i, "Md")]
                .into_iter()
                .find(|k| smc.key_info(*k).is_ok())
                .ok_or_else(|| format!("fan {i}: no mode key"))?;
            fans.push(FanInfo { index: i, min_rpm, max_rpm });
            keys.push(FanKeys { actual: fan_key(i, "Ac"), target: fan_key(i, "Tg"), mode });
        }

        let unlock_key = Some(smc::key("Ftst")).filter(|k| smc.key_info(*k).is_ok());

        let mut temps = Vec::new();
        for k in smc.all_keys().map_err(|e| e.to_string())? {
            let b = k.bytes();
            if b[0] != b'T' {
                continue;
            }
            let Ok(info) = smc.key_info(k) else { continue };
            if info.data_type != smc::key("flt ") || info.size != 4 {
                continue;
            }
            // Skip placeholders (0.0) and garbage values present on every boot.
            if let Ok(v) = smc.read_f64(k) {
                if v > 0.5 && v < 150.0 {
                    temps.push(SensorKey(b));
                }
            }
        }

        Ok(SmcHardware { smc, fans, keys, temps, unlock_key, write_enabled, shadow: Default::default() })
    }

    pub fn has_unlock_key(&self) -> bool {
        self.unlock_key.is_some()
    }

    pub fn smc(&mut self) -> &mut Smc {
        &mut self.smc
    }

    fn write(&mut self, key: FourCC, v: f64) -> Result<(), String> {
        if !self.write_enabled {
            crate::log!("[dry-run] would write {key} = {v}");
            self.shadow.insert(key, v);
            return Ok(());
        }
        self.smc.write_f64(key, v).map_err(|e| e.to_string())
    }

    fn read(&mut self, key: FourCC) -> Result<f64, String> {
        if let Some(v) = self.shadow.get(&key) {
            return Ok(*v);
        }
        self.smc.read_f64(key).map_err(|e| e.to_string())
    }
}

impl Hardware for SmcHardware {
    fn fans(&self) -> &[FanInfo] {
        &self.fans
    }

    fn temperature_keys(&self) -> &[SensorKey] {
        &self.temps
    }

    fn read_temperature(&mut self, key: SensorKey) -> Option<f64> {
        self.smc.read_f64(FourCC::from_bytes(key.0)).ok()
    }

    fn read_fan(&mut self, index: u8) -> Result<FanReading, String> {
        let k = &self.keys[index as usize];
        let (actual, target, mode) = (k.actual, k.target, k.mode);
        Ok(FanReading {
            actual_rpm: self.smc.read_f64(actual).map_err(|e| e.to_string())?,
            target_rpm: self.read(target)?,
            forced: self.read(mode)? != 0.0,
        })
    }

    fn set_target(&mut self, index: u8, rpm: f64) -> Result<(), String> {
        let (target, mode) = {
            let k = &self.keys[index as usize];
            (k.target, k.mode)
        };
        if let Some(unlock) = self.unlock_key {
            if self.read(unlock).unwrap_or(0.0) == 0.0 {
                self.write(unlock, 1.0)?;
            }
        }
        if self.read(mode).unwrap_or(0.0) != 1.0 {
            self.write(mode, 1.0)?;
        }
        self.write(target, rpm.round())
    }

    fn release(&mut self, index: u8) -> Result<(), String> {
        let mode = self.keys[index as usize].mode;
        self.write(mode, 0.0)?;
        if let Some(unlock) = self.unlock_key {
            let any_forced =
                (0..self.keys.len()).any(|i| i != index as usize && self.read(self.keys[i].mode).unwrap_or(0.0) != 0.0);
            if !any_forced {
                self.write(unlock, 0.0)?;
            }
        }
        Ok(())
    }
}
