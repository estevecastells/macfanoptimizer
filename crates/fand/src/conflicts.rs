//! Detect other fan-control apps that would fight us over the SMC.

const KNOWN: &[&str] = &["Macs Fan Control", "smcFanControl", "TG Pro", "FanControl"];

pub fn running_fan_apps() -> Vec<String> {
    let mut pids = vec![0i32; 4096];
    let n = unsafe {
        libc::proc_listallpids(pids.as_mut_ptr() as *mut _, (pids.len() * std::mem::size_of::<i32>()) as i32)
    };
    if n <= 0 {
        return vec![];
    }
    let mut found = Vec::new();
    let mut buf = [0u8; 256];
    for &pid in &pids[..n as usize] {
        let len = unsafe { libc::proc_name(pid, buf.as_mut_ptr() as *mut _, buf.len() as u32) };
        if len <= 0 {
            continue;
        }
        let name = String::from_utf8_lossy(&buf[..len as usize]);
        if let Some(app) = KNOWN.iter().find(|k| name.starts_with(*k)) {
            if !found.iter().any(|f| f == app) {
                found.push(app.to_string());
            }
        }
    }
    found
}
