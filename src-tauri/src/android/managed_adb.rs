use super::{adb::Server, environment};
use std::{
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Stdio},
    time::{Duration, Instant},
};

// Windows can take slightly over two seconds to report a closed loopback
// listener. Let that refusal arrive; a real timeout still fails closed below.
const LISTENER_PROBE_TIMEOUT: Duration = if cfg!(windows) {
    Duration::from_secs(5)
} else {
    Duration::from_secs(2)
};

/// A shared server is never killed by Lomi. Its foreground entry point
/// only binds a listener; unlike an ordinary client it cannot replace a peer.
#[derive(Default)]
pub struct SharedServer {
    child: Option<Child>,
}

impl SharedServer {
    pub fn ensure(&mut self, root: &Path, program: &Path) -> Result<(), String> {
        self.ensure_at(root, program, 5037)
    }

    pub(super) fn ensure_at(
        &mut self,
        root: &Path,
        program: &Path,
        port: u16,
    ) -> Result<(), String> {
        if let Some(child) = &mut self.child {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                self.child = None;
            } else {
                // A readiness timeout does not prove exit. Keep the same child
                // until it is reaped instead of overwriting its only handle.
                return Server { port }.preflight().map_err(|error| {
                    format!("Managed ADB is still running but unavailable: {error}. Its process is retained; retry connection.")
                });
            }
        }
        let server = Server { port };
        match TcpStream::connect_timeout(&([127,0,0,1], port).into(), LISTENER_PROBE_TIMEOUT) {
            Ok(_) => return server.preflight(),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {},
            Err(error) => return Err(format!("Cannot inspect the local ADB server: {error}. Retry when the listener is available.")),
        }
        let reservation = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(socket) => socket,
            Err(_) => return server.preflight(),
        };
        let mut command = environment::command(program, root, &root.join("sdk"), None, port)?;
        // adb's server listener rejects an explicit hostname in -L. tcp:PORT
        // binds loopback by default; -a (all interfaces) is deliberately absent.
        command
            .args(["-L", &format!("tcp:{port}"), "server", "nodaemon"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        drop(reservation);
        self.child = Some(command.spawn().map_err(|e| {
            format!("Cannot start managed ADB: {e}. Repair Platform Tools in Android settings.")
        })?);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            // A competing server may win the bind. Preflight that server without
            // invoking a version-checking client or sending host:kill.
            if server.preflight().is_ok() {
                return Ok(());
            }
            if let Some(child) = &mut self.child {
                if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                    self.child = None;
                    return server.preflight().map_err(|error| {
                        format!("Managed ADB could not acquire its listener: {error}")
                    });
                }
            }
            if Instant::now() >= deadline {
                return Err(
                    "Managed ADB has not become ready. Its process is retained; retry connection."
                        .into(),
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
