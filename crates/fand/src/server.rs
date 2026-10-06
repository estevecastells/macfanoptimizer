//! Unix-socket server: newline-delimited JSON requests/responses.
//!
//! The socket is world-connectable, but every mutating request is checked
//! against the peer's uid (root or `config.allowed_uids`).

use fan_core::protocol::{Request, Response, PROTOCOL_VERSION};
use fan_core::{Config, Engine, Hardware};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

const MAX_LINE: usize = 64 * 1024;

pub struct Shared<H: Hardware> {
    engine: Mutex<Engine<H>>,
    kick: (Mutex<bool>, Condvar),
    config_path: Option<PathBuf>,
}

impl<H: Hardware> Shared<H> {
    pub fn new(engine: Engine<H>, config_path: Option<PathBuf>) -> Arc<Self> {
        Arc::new(Shared { engine: Mutex::new(engine), kick: (Mutex::new(false), Condvar::new()), config_path })
    }

    /// Lock the engine, recovering from a poisoned mutex (a panicked thread
    /// must not prevent us from handing fans back to macOS).
    pub fn engine(&self) -> MutexGuard<'_, Engine<H>> {
        self.engine.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Wake the control loop early (e.g. after a mode change).
    pub fn kick(&self) {
        let (m, cv) = &self.kick;
        *m.lock().unwrap_or_else(|e| e.into_inner()) = true;
        cv.notify_all();
    }

    /// Sleep up to `timeout`. Returns true (early) if kicked.
    pub fn wait(&self, timeout: Duration) -> bool {
        let (m, cv) = &self.kick;
        let guard = m.lock().unwrap_or_else(|e| e.into_inner());
        let (mut guard, _) =
            cv.wait_timeout_while(guard, timeout, |kicked| !*kicked).unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *guard)
    }

    pub fn handle(&self, req: Request, peer_uid: u32) -> Response {
        if req.is_mutation() && peer_uid != 0 && !self.engine().config().allowed_uids.contains(&peer_uid) {
            return Response::Error { message: format!("permission denied for uid {peer_uid}") };
        }
        match req {
            Request::Ping => Response::Pong { version: env!("CARGO_PKG_VERSION").into(), protocol: PROTOCOL_VERSION },
            Request::Status => Response::Status { status: self.engine().status().clone() },
            Request::Sensors => Response::Sensors { sensors: self.engine().all_sensors() },
            Request::GetConfig => Response::Config { config: self.engine().config().clone() },
            Request::SetMode { mode } => self.update(|c| c.mode = mode),
            Request::SetProfile { profile } => self.update(|c| c.profile = profile),
            Request::SetConfig { config } => {
                let is_root = peer_uid == 0;
                self.update(move |c| {
                    let allowed = std::mem::take(&mut c.allowed_uids);
                    *c = *config;
                    // Only root may change who is allowed to change settings.
                    if !is_root {
                        c.allowed_uids = allowed;
                    }
                })
            }
        }
    }

    fn update(&self, f: impl FnOnce(&mut Config)) -> Response {
        let mut engine = self.engine();
        let mut cfg = engine.config().clone();
        f(&mut cfg);
        if let Err(e) = engine.set_config(cfg.clone()) {
            return Response::Error { message: e };
        }
        drop(engine);
        if let Some(path) = &self.config_path {
            if let Err(e) = save_config(path, &cfg) {
                crate::log!("warning: could not persist config to {}: {e}", path.display());
            }
        }
        crate::log!("config updated: mode={:?} profile={:?}", cfg.mode, cfg.profile);
        self.kick();
        Response::Config { config: cfg }
    }
}

pub fn save_config(path: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, cfg.to_toml())?;
    std::fs::rename(tmp, path)
}

fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let (mut uid, mut gid) = (0, 0);
    let r = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    (r == 0).then_some(uid)
}

fn serve_connection<H: Hardware>(shared: &Shared<H>, stream: UnixStream) {
    let Some(uid) = peer_uid(&stream) else { return };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let Ok(mut writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match Read::by_ref(&mut reader).take(MAX_LINE as u64).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if !line.ends_with('\n') && line.len() >= MAX_LINE {
            let _ = writeln!(writer, "{}", json(&Response::Error { message: "request too large".into() }));
            return;
        }
        let resp = match serde_json::from_str::<Request>(line.trim()) {
            Ok(req) => shared.handle(req, uid),
            Err(e) => Response::Error { message: format!("bad request: {e}") },
        };
        if writeln!(writer, "{}", json(&resp)).is_err() {
            return;
        }
    }
}

fn json(r: &Response) -> String {
    serde_json::to_string(r).unwrap_or_else(|e| format!(r#"{{"type":"error","message":"{e}"}}"#))
}

/// Bind the socket (replacing a stale one) and serve forever on background threads.
pub fn spawn<H: Hardware + Send + 'static>(shared: Arc<Shared<H>>, path: &Path) -> std::io::Result<()> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))?;
    std::thread::Builder::new().name("socket-accept".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let shared = shared.clone();
            let _ =
                std::thread::Builder::new().name("socket-conn".into()).spawn(move || serve_connection(&shared, stream));
        }
    })?;
    Ok(())
}
