//! The real daemon binary exits promptly on SIGTERM even mid-sleep: the
//! control loop sleeps a whole interval and relies on the signal to wake it.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn sigterm_wakes_a_long_sleep() {
    // Not temp_dir(): on macOS that path is too long for a Unix socket.
    let dir = std::path::PathBuf::from(format!("/tmp/fand-shutdown-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (config, socket) = (dir.join("config.toml"), dir.join("s.sock"));
    // A 60 s interval: without the wake-up, shutdown would take up to a minute.
    std::fs::write(&config, "poll_interval_ms = 60000\n").unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_fand"))
        .args(["--dry-run", "--config"])
        .arg(&config)
        .arg("--socket")
        .arg(&socket)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Wait until it is serving, which also means it is past its first tick.
    let start = Instant::now();
    while !socket.exists() {
        if child.try_wait().unwrap().is_some() {
            let mut err = String::new();
            std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut err).unwrap();
            let _ = std::fs::remove_dir_all(&dir);
            // No SMC (e.g. a CI virtual machine): nothing to test here.
            assert!(err.contains("SMC:"), "daemon exited early: {err}");
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "daemon never opened its socket");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(200));

    let sent = Instant::now();
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "{status:?}");
            break;
        }
        if sent.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            panic!("daemon still running 5 s after SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(sent.elapsed() < Duration::from_secs(2), "shutdown took {:?}", sent.elapsed());
    assert!(!socket.exists(), "socket left behind");
    let _ = std::fs::remove_dir_all(&dir);
}
