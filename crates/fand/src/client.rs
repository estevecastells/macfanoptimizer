//! Blocking client for the daemon socket.

use fan_core::protocol::{Request, Response};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

pub fn request(socket: &Path, req: &Request) -> Result<Response, String> {
    let stream =
        UnixStream::connect(socket).map_err(|e| format!("cannot connect to daemon at {}: {e}", socket.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let mut w = stream.try_clone().map_err(|e| e.to_string())?;
    let line = serde_json::to_string(req).map_err(|e| e.to_string())?;
    writeln!(w, "{line}").map_err(|e| e.to_string())?;
    let mut resp = String::new();
    BufReader::new(stream).read_line(&mut resp).map_err(|e| e.to_string())?;
    serde_json::from_str(resp.trim()).map_err(|e| format!("bad response: {e}: {resp}"))
}
