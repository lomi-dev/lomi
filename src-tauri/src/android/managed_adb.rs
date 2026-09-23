use super::{adb::Server, environment};
use std::{
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Stdio},
    time::{Duration, Instant},
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
        match TcpStream::connect_timeout(&([127,0,0,1], port).into(), Duration::from_secs(2)) {
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

    #[cfg(any(test, feature = "android-probe", feature = "mcp-probe"))]
    pub fn stop_private_fixture(&mut self, port: u16) -> Result<(), String> {
        assert!(port >= 10_000 && port != 5037);
        if let Some(child) = &mut self.child {
            if child.try_wait().map_err(|e| e.to_string())?.is_none() {
                child.kill().map_err(|e| e.to_string())?;
            }
            child.wait().map_err(|e| e.to_string())?;
        }
        self.child = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    #[ignore = "Subprocess fixture for the retained server test"]
    fn retained_server_fixture() {
        if std::env::var_os("LOMI_ADB_RETAINED_FIXTURE").is_some() {
            let _ = std::io::stdin().read(&mut [0]);
        }
    }

    #[test]
    fn retry_keeps_an_unready_server_child_until_confirmed_exit() {
        let root = tempfile::tempdir().unwrap();
        let reservation = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "android::managed_adb::tests::retained_server_fixture",
                "--ignored",
            ])
            .env("LOMI_ADB_RETAINED_FIXTURE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut server = SharedServer { child: Some(child) };
        let error = server
            .ensure_at(root.path(), &root.path().join("missing-adb"), port)
            .unwrap_err();
        assert!(error.contains("process is retained"), "{error}");
        let child = server.child.as_mut().unwrap();
        assert_eq!(child.id(), pid);
        drop(child.stdin.take());
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn incompatible_shared_server_is_preserved_without_launching_a_client() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let other = std::thread::spawn(move || {
            drop(listener.accept().unwrap());
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 16];
            socket.read_exact(&mut request).unwrap();
            assert_eq!(&request, b"000chost:version");
            socket.write_all(b"OKAY00040028").unwrap();
            let mut more = [0; 4];
            assert_eq!(socket.read(&mut more).unwrap(), 0);
        });
        let root = tempfile::tempdir().unwrap();
        let mut server = SharedServer::default();
        let error = server
            .ensure_at(root.path(), &root.path().join("missing-adb"), port)
            .unwrap_err();
        assert!(error.contains("incompatible"));
        assert!(server.child.is_none());
        other.join().unwrap();
    }
}
