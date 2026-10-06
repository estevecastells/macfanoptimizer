//! Wire protocol between the daemon and its clients (CLI, Swift app).
//!
//! Transport: a Unix domain socket carrying newline-delimited JSON. Each
//! request line gets exactly one response line.

use crate::config::{Config, Mode, Profile};
use crate::controller::{Decision, Reason};
use crate::hardware::{FanInfo, FanReading};
use crate::sensors::{GroupSummary, SensorKey};
use serde::{Deserialize, Serialize};

pub const DEFAULT_SOCKET_PATH: &str = "/var/run/macfanoptimizer.sock";
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Ping,
    Status,
    /// Every temperature sensor with its current value.
    Sensors,
    GetConfig,
    SetMode {
        mode: Mode,
    },
    SetProfile {
        profile: Profile,
    },
    SetConfig {
        config: Box<Config>,
    },
}

impl Request {
    pub fn is_mutation(&self) -> bool {
        matches!(self, Request::SetMode { .. } | Request::SetProfile { .. } | Request::SetConfig { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Pong { version: String, protocol: u32 },
    Status { status: Status },
    Sensors { sensors: Vec<SensorValue> },
    Config { config: Config },
    Error { message: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FanStatus {
    pub info: FanInfo,
    pub reading: Option<FanReading>,
    /// RPM the daemon is currently commanding, if it is controlling this fan.
    pub commanded_rpm: Option<f64>,
    /// Runtime check that the fan obeys: `None` until tested, then pass/fail.
    #[serde(default)]
    pub verified: Option<bool>,
}

/// How much we trust fan control on this Mac.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    /// Tested end to end by maintainers or contributors.
    Validated,
    /// Has the standard fan keys and recognizable sensors, but nobody has
    /// validated it yet. Control is enabled, guarded by runtime verification.
    Compatible,
    /// No fans, no controllable fans, or no recognizable sensors.
    #[default]
    MonitorOnly,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MachineInfo {
    /// e.g. `Mac17,9`
    pub model: String,
    /// e.g. `Apple M5 Pro`
    pub chip: String,
    pub support: SupportLevel,
    /// Human-readable explanation of `support`.
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SensorValue {
    pub key: SensorKey,
    pub group: String,
    pub celsius: Option<f64>,
    /// True if this sensor feeds the controller.
    pub control: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub mode: Mode,
    pub profile: Profile,
    pub decision: Decision,
    pub reason: Reason,
    /// Hottest valid control sensor this tick.
    pub hotspot_key: Option<SensorKey>,
    pub hotspot_c: Option<f64>,
    /// Smoothed value the controller acts on.
    pub control_c: Option<f64>,
    pub duty_pct: f64,
    pub fans: Vec<FanStatus>,
    pub groups: Vec<GroupSummary>,
    pub control_sensor_count: usize,
    /// Duration of the last control tick, microseconds.
    pub tick_us: u64,
    pub ticks: u64,
    /// Total SMC writes since start (low numbers = calm control).
    pub smc_writes: u64,
    pub last_error: Option<String>,
    /// Another process changed fan targets behind our back (e.g. another fan app).
    pub external_override: bool,
    /// Other fan-control apps detected running (they will fight over the fans).
    #[serde(default)]
    pub conflicts: Vec<String>,
    /// Seconds since the daemon started.
    pub uptime_s: f64,
    /// Hardware model identifier, e.g. `Mac17,9`.
    #[serde(default)]
    pub model: String,
    /// False in dry-run mode or on an unsupported model: decisions are computed but not applied.
    #[serde(default)]
    pub writes_enabled: bool,
    /// e.g. `Apple M5 Pro`.
    #[serde(default)]
    pub chip: String,
    #[serde(default)]
    pub support: SupportLevel,
    #[serde(default)]
    pub support_note: String,
    /// Set if fans failed runtime verification; control stays off until restart.
    #[serde(default)]
    pub control_disabled: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_wire_format() {
        let r: Request = serde_json::from_str(r#"{"cmd":"set_mode","mode":{"kind":"fixed","rpm":3000}}"#).unwrap();
        assert_eq!(r, Request::SetMode { mode: Mode::Fixed { rpm: 3000.0 } });
        assert!(r.is_mutation());
        assert_eq!(serde_json::to_string(&Request::Status).unwrap(), r#"{"cmd":"status"}"#);
    }

    /// Shared with the Swift app's checks (app/Sources/KitChecks).
    #[test]
    fn shared_fixtures_parse() {
        let status: Response = serde_json::from_str(include_str!("../../../fixtures/status_response.json")).unwrap();
        let Response::Status { status } = status else { panic!("expected status") };
        assert_eq!(status.fans.len(), 2);
        assert_eq!(status.conflicts, vec!["Macs Fan Control".to_string()]);
        // Round-trips losslessly, so the fixture covers every field the daemon emits.
        let again: Status = serde_json::from_value(serde_json::to_value(&status).unwrap()).unwrap();
        assert_eq!(again, status);
        let fields = serde_json::to_value(&status).unwrap().as_object().unwrap().len();
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/status_response.json")).unwrap();
        assert_eq!(fields, fixture["status"].as_object().unwrap().len(), "fixture is missing Status fields");

        // Same order as the Swift checks encode them.
        let expected = [
            Request::Status,
            Request::SetMode { mode: Mode::Fixed { rpm: 3000.0 } },
            Request::SetMode { mode: Mode::Smart },
            Request::SetProfile { profile: Profile::Quiet },
        ];
        let parsed: Vec<Request> = include_str!("../../../fixtures/requests.jsonl")
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(parsed, expected);
    }

    #[test]
    fn response_wire_format() {
        let r = Response::Error { message: "nope".into() };
        assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"type":"error","message":"nope"}"#);
    }
}
