//! Library half of the daemon, shared with the `fanctl` CLI.

pub mod client;
pub mod conflicts;
pub mod log;
pub mod server;
pub mod smc_hw;

pub use smc_hw::SmcHardware;

/// Models on which fan writes have been validated end to end. Others run
/// read-only unless `allow_unsupported_model = true` in the config.
pub const SUPPORTED_MODELS: &[&str] = &[
    "Mac17,9", // MacBook Pro with M5 Pro
];

/// Hardware model identifier (`sysctl hw.model`), e.g. `Mac17,9`.
pub fn hardware_model() -> String {
    let mut buf = [0u8; 64];
    let mut len = buf.len();
    let r = unsafe {
        libc::sysctlbyname(c"hw.model".as_ptr(), buf.as_mut_ptr() as *mut _, &mut len, std::ptr::null_mut(), 0)
    };
    if r != 0 {
        return "unknown".into();
    }
    String::from_utf8_lossy(&buf[..len]).trim_end_matches('\0').to_string()
}

pub fn is_supported_model(model: &str) -> bool {
    SUPPORTED_MODELS.contains(&model)
}

pub const DEFAULT_CONFIG_PATH: &str = "/Library/Application Support/MacFanOptimizer/config.toml";
