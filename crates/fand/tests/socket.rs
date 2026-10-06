//! End-to-end test of the daemon socket protocol against simulated hardware.

use fan_core::protocol::{Request, Response};
use fan_core::sim::SimHardware;
use fan_core::{Config, Engine, Mode, Profile};
use fand::{client, server};
use std::path::PathBuf;

fn start(allowed_uids: Vec<u32>) -> (PathBuf, std::sync::Arc<server::Shared<SimHardware>>) {
    let dir = std::env::temp_dir().join(format!("fand-test-{}-{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("s.sock");
    let cfg = Config { allowed_uids, ..Config::default() };
    let engine = Engine::new(SimHardware::new(), cfg).unwrap();
    let shared = server::Shared::new(engine, Some(dir.join("config.toml")));
    server::spawn(shared.clone(), &socket).unwrap();
    (socket, shared)
}

fn rand_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

fn my_uid() -> u32 {
    unsafe { libc::getuid() }
}

#[test]
fn ping_and_status() {
    let (sock, shared) = start(vec![]);
    shared.engine().tick(0.0);
    assert!(matches!(client::request(&sock, &Request::Ping).unwrap(), Response::Pong { protocol: 1, .. }));
    let Response::Status { status } = client::request(&sock, &Request::Status).unwrap() else { panic!() };
    assert_eq!(status.fans.len(), 2);
    assert_eq!(status.ticks, 1);
}

#[test]
fn mutations_require_permission() {
    if my_uid() == 0 {
        return; // root is always allowed; nothing to test
    }
    let (sock, _) = start(vec![]);
    let r = client::request(&sock, &Request::SetMode { mode: Mode::Max }).unwrap();
    assert!(matches!(r, Response::Error { ref message } if message.contains("permission denied")), "{r:?}");
}

#[test]
fn allowed_user_can_change_mode_and_profile_and_it_persists() {
    let (sock, shared) = start(vec![my_uid()]);
    let r = client::request(&sock, &Request::SetMode { mode: Mode::Fixed { rpm: 3000.0 } }).unwrap();
    assert!(matches!(r, Response::Config { .. }), "{r:?}");
    client::request(&sock, &Request::SetProfile { profile: Profile::Quiet }).unwrap();
    let cfg = shared.engine().config().clone();
    assert_eq!(cfg.mode, Mode::Fixed { rpm: 3000.0 });
    assert_eq!(cfg.profile, Profile::Quiet);
    let path = sock.parent().unwrap().join("config.toml");
    let saved = Config::from_toml(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(saved, cfg);
}

#[test]
fn non_root_cannot_grant_itself_more_users() {
    if my_uid() == 0 {
        return;
    }
    let (sock, shared) = start(vec![my_uid()]);
    let config = Config { allowed_uids: vec![my_uid(), 4242], ..Config::default() };
    client::request(&sock, &Request::SetConfig { config: Box::new(config) }).unwrap();
    assert_eq!(shared.engine().config().allowed_uids, vec![my_uid()]);
}

#[test]
fn invalid_config_is_rejected() {
    let (sock, _) = start(vec![my_uid()]);
    let config = Config { poll_interval_ms: 1, ..Config::default() };
    let r = client::request(&sock, &Request::SetConfig { config: Box::new(config) }).unwrap();
    assert!(matches!(r, Response::Error { .. }));
}

#[test]
fn garbage_gets_an_error_not_a_crash() {
    use std::io::{BufRead, BufReader, Write};
    let (sock, _) = start(vec![]);
    let mut s = std::os::unix::net::UnixStream::connect(&sock).unwrap();
    writeln!(s, "{{not json").unwrap();
    let mut line = String::new();
    BufReader::new(s.try_clone().unwrap()).read_line(&mut line).unwrap();
    assert!(line.contains("\"type\":\"error\""), "{line}");
    // Connection is still usable.
    assert!(matches!(client::request(&sock, &Request::Ping).unwrap(), Response::Pong { .. }));
}
