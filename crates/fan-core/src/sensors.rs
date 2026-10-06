//! Temperature sensor naming, grouping and control-sensor selection.

use crate::config::SensorConfig;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// A four-character SMC key, stored inline (no allocation per reading).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SensorKey(pub [u8; 4]);

impl SensorKey {
    pub fn parse(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        (b.len() == 4 && b.is_ascii()).then(|| SensorKey([b[0], b[1], b[2], b[3]]))
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("????")
    }

    pub fn starts_with(&self, prefix: &str) -> bool {
        self.0.starts_with(prefix.as_bytes())
    }
}

impl fmt::Display for SensorKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for SensorKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}'", self.as_str())
    }
}

impl Serialize for SensorKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SensorKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        SensorKey::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid sensor key {s:?}")))
    }
}

/// Human-readable group for a sensor key.
///
/// Apple doesn't document SMC keys and renames them between chip generations.
/// Labels for `Tm`/`Ts` come from a load test on an M5 Pro: they rise most
/// under all-core CPU load, so they are treated as CPU-cluster die sensors.
pub fn group_of(key: SensorKey) -> &'static str {
    let k = key.0;
    match (k[1], k[2]) {
        (b'p', _) => "CPU P-cluster",
        (b'e', _) => "CPU E-cluster",
        (b'f', _) => "CPU/GPU (f)",
        (b'm', _) => "CPU cluster (m)",
        (b's', _) => "SoC (s)",
        (b'g', _) => "GPU",
        (b'B', _) | (b'b', _) => "Battery",
        (b'H', _) => "SSD",
        (b'P', b'D') | (b'R', b'D') | (b'U', b'D') => "Power delivery",
        (b'V', _) => "Virtual / aggregate",
        (b'a', _) | (b'A', _) => "Airflow / ambient",
        (b'W', _) => "Wireless",
        (b'D', _) => "Display",
        _ => "Other",
    }
}

/// Keys that drive the controller: those matching a configured family, plus
/// `include`, minus `exclude`.
pub fn select_control_keys(available: &[SensorKey], cfg: &SensorConfig) -> Vec<SensorKey> {
    let excluded = |k: &SensorKey| cfg.exclude.iter().any(|e| e == k.as_str());
    let mut keys: Vec<SensorKey> = available
        .iter()
        .copied()
        .filter(|k| cfg.families.iter().any(|f| k.starts_with(f)) || cfg.include.iter().any(|i| i == k.as_str()))
        .filter(|k| !excluded(k))
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

pub fn is_valid(cfg: &SensorConfig, celsius: f64) -> bool {
    celsius.is_finite() && celsius >= cfg.min_valid_c && celsius <= cfg.max_valid_c
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupSummary {
    pub group: String,
    pub max_c: f64,
    pub avg_c: f64,
    pub count: usize,
}

/// Summarise readings by group, hottest group first.
pub fn summarize(readings: &[(SensorKey, f64)]) -> Vec<GroupSummary> {
    let mut groups: Vec<(&'static str, f64, f64, usize)> = Vec::new();
    for &(k, c) in readings {
        let g = group_of(k);
        match groups.iter_mut().find(|e| e.0 == g) {
            Some(e) => {
                e.1 = e.1.max(c);
                e.2 += c;
                e.3 += 1;
            }
            None => groups.push((g, c, c, 1)),
        }
    }
    let mut out: Vec<GroupSummary> = groups
        .into_iter()
        .map(|(g, max, sum, n)| GroupSummary { group: g.to_string(), max_c: max, avg_c: sum / n as f64, count: n })
        .collect();
    out.sort_by(|a, b| b.max_c.total_cmp(&a.max_c));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> SensorKey {
        SensorKey::parse(s).unwrap()
    }

    #[test]
    fn selects_families_include_exclude() {
        let avail = [k("Tp00"), k("Tm0g"), k("TB0T"), k("Tg08"), k("TH0x")];
        let cfg = SensorConfig {
            families: vec!["Tp".into(), "Tm".into(), "Tg".into()],
            include: vec!["TH0x".into()],
            exclude: vec!["Tg08".into()],
            ..SensorConfig::default()
        };
        assert_eq!(select_control_keys(&avail, &cfg), vec![k("TH0x"), k("Tm0g"), k("Tp00")]);
    }

    #[test]
    fn groups_and_summary() {
        assert_eq!(group_of(k("Tg1x")), "GPU");
        assert_eq!(group_of(k("TB0T")), "Battery");
        let s = summarize(&[(k("Tp00"), 50.0), (k("Tp04"), 60.0), (k("Tg08"), 70.0)]);
        assert_eq!(s[0].group, "GPU");
        assert_eq!(s[1].max_c, 60.0);
        assert_eq!(s[1].avg_c, 55.0);
    }

    #[test]
    fn key_serde() {
        assert_eq!(serde_json::to_string(&k("TCMb")).unwrap(), "\"TCMb\"");
        assert!(serde_json::from_str::<SensorKey>("\"TOOLONG\"").is_err());
    }
}
