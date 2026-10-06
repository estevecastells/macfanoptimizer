//! Library half of the daemon, shared with the `fanctl` CLI.

pub mod client;
pub mod conflicts;
pub mod log;
pub mod server;
pub mod smc_hw;

pub use smc_hw::SmcHardware;

use fan_core::protocol::MachineInfo;
use fan_core::Hardware;

pub const DEFAULT_CONFIG_PATH: &str = "/Library/Application Support/MacFanOptimizer/config.toml";

/// Models on which fan control has been validated end to end. Other Macs with
/// the standard fan keys are "compatible": control is enabled and guarded by
/// runtime verification (see `fan_core::engine::VERIFY_TIMEOUT_S`).
/// Add a model here only with a test report (see docs/HARDWARE.md).
pub const VALIDATED_MODELS: &[(&str, &str)] = &[("Mac17,9", "MacBook Pro (M5 Pro)")];

/// Minimum number of recognized die sensors before we trust the readings
/// enough to drive the fans.
pub const MIN_CONTROL_SENSORS: usize = 4;

fn sysctl_string(name: &std::ffi::CStr) -> Option<String> {
    let mut buf = [0u8; 256];
    let mut len = buf.len();
    let r = unsafe { libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr() as *mut _, &mut len, std::ptr::null_mut(), 0) };
    (r == 0).then(|| String::from_utf8_lossy(&buf[..len]).trim_end_matches('\0').to_string())
}

/// Hardware model identifier, e.g. `Mac17,9`.
pub fn hardware_model() -> String {
    sysctl_string(c"hw.model").unwrap_or_else(|| "unknown".into())
}

/// Chip name, e.g. `Apple M5 Pro`.
pub fn chip_name() -> String {
    sysctl_string(c"machdep.cpu.brand_string").unwrap_or_else(|| "unknown".into())
}

/// macOS version, e.g. `26.6.2`.
pub fn os_version() -> String {
    sysctl_string(c"kern.osproductversion").unwrap_or_else(|| "unknown".into())
}

pub fn is_validated_model(model: &str) -> bool {
    VALIDATED_MODELS.iter().any(|(m, _)| *m == model)
}

/// Decide how far to trust fan control on this machine, from what the SMC
/// actually exposes rather than from a list of model names.
pub fn assess(model: &str, chip: &str, hw: &SmcHardware, control_sensors: usize) -> MachineInfo {
    assess_facts(&Facts {
        model,
        chip,
        apple_silicon: cfg!(target_arch = "aarch64"),
        fans: hw.fans().len(),
        issues: hw.issues(),
        control_sensors,
    })
}

/// What `assess` looks at, separated from the SMC so it can be unit tested.
pub struct Facts<'a> {
    pub model: &'a str,
    pub chip: &'a str,
    pub apple_silicon: bool,
    pub fans: usize,
    pub issues: &'a [String],
    pub control_sensors: usize,
}

pub fn assess_facts(f: &Facts) -> MachineInfo {
    use fan_core::protocol::SupportLevel::*;
    let (support, note) = if !f.apple_silicon || !f.chip.starts_with("Apple") {
        (MonitorOnly, "Only Apple Silicon Macs are supported.".to_string())
    } else if f.fans == 0 {
        let why = if f.issues.is_empty() {
            "This Mac has no fans (e.g. MacBook Air), so there is nothing to control.".to_string()
        } else {
            format!("No controllable fans found: {}.", f.issues.join("; "))
        };
        (MonitorOnly, why)
    } else if f.control_sensors < MIN_CONTROL_SENSORS {
        (
            MonitorOnly,
            format!(
                "Only {} recognized chip temperature sensors (need {MIN_CONTROL_SENSORS}). \
                 Please report this Mac so its sensors can be added.",
                f.control_sensors
            ),
        )
    } else if is_validated_model(f.model) {
        (Validated, "Validated on this model.".to_string())
    } else {
        (
            Compatible,
            "Not yet validated on this model. Fan control is enabled and verified at runtime: \
             if the fans don't respond, they are handed back to macOS."
                .to_string(),
        )
    };
    MachineInfo { model: f.model.to_string(), chip: f.chip.to_string(), support, note }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fan_core::protocol::SupportLevel;

    fn facts<'a>(model: &'a str, fans: usize, sensors: usize, issues: &'a [String]) -> Facts<'a> {
        Facts { model, chip: "Apple M4 Pro", apple_silicon: true, fans, issues, control_sensors: sensors }
    }

    #[test]
    fn validated_model() {
        assert_eq!(assess_facts(&facts("Mac17,9", 2, 118, &[])).support, SupportLevel::Validated);
    }

    #[test]
    fn unknown_model_with_fans_and_sensors_is_compatible() {
        assert_eq!(assess_facts(&facts("Mac16,6", 2, 40, &[])).support, SupportLevel::Compatible);
    }

    #[test]
    fn fanless_mac_is_monitor_only() {
        let m = assess_facts(&facts("Mac15,12", 0, 30, &[]));
        assert_eq!(m.support, SupportLevel::MonitorOnly);
        assert!(m.note.contains("no fans"));
    }

    #[test]
    fn fans_with_missing_keys_are_explained() {
        let issues = vec!["fan 0 skipped: missing SMC keys F0md".to_string()];
        let m = assess_facts(&facts("Mac99,1", 0, 30, &issues));
        assert_eq!(m.support, SupportLevel::MonitorOnly);
        assert!(m.note.contains("F0md"));
    }

    #[test]
    fn unrecognized_sensors_are_monitor_only() {
        assert_eq!(assess_facts(&facts("Mac99,1", 2, 2, &[])).support, SupportLevel::MonitorOnly);
    }

    #[test]
    fn even_a_validated_model_needs_sensors() {
        assert_eq!(assess_facts(&facts("Mac17,9", 2, 0, &[])).support, SupportLevel::MonitorOnly);
    }

    #[test]
    fn intel_is_monitor_only() {
        let f = Facts { chip: "Intel(R) Core(TM) i9", apple_silicon: false, ..facts("MacBookPro16,1", 2, 20, &[]) };
        assert_eq!(assess_facts(&f).support, SupportLevel::MonitorOnly);
    }
}
