use std::{
    collections::VecDeque,
    io::Read,
    process::{Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: usize = 64 * 1024;
use super::installer_tree::Tree;

/// Only direct, verified SDK-manager/JVM invocations use this owner. Emulator
/// shutdown has a separate graceful protocol and must not use installer deadlines.
pub struct InstallerChild {
    child: Tree,
    deadline: Instant,
    output: Arc<Mutex<VecDeque<u8>>>,
    readers: Vec<JoinHandle<()>>,
    stop_reason: Option<&'static str>,
    exit: Option<ExitStatus>,
}

#[derive(Debug)]
pub struct Outcome {
    pub success: bool,
    pub output: String,
    pub stop_reason: Option<&'static str>,
}

impl InstallerChild {
    pub fn spawn(mut command: Command, timeout: Duration) -> Result<Self, String> {
        if timeout.is_zero() || timeout > Duration::from_secs(30 * 60) {
            return Err("Invalid Android installer deadline".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child =
            Tree::spawn(&mut command).map_err(|e| format!("Cannot start Android tools: {e}"))?;
        let output = Arc::new(Mutex::new(VecDeque::with_capacity(OUTPUT_LIMIT)));
        let stdout = child
            .child
            .stdout
            .take()
            .ok_or("Android installer stdout is unavailable")?;
        let stderr = child
            .child
            .stderr
            .take()
            .ok_or("Android installer stderr is unavailable")?;
        Ok(Self {
            child,
            deadline: Instant::now() + timeout,
            readers: vec![drain(stdout, output.clone()), drain(stderr, output.clone())],
            output,
            stop_reason: None,
            exit: None,
        })
    }

    pub fn cancel(&mut self) {
        self.stop_reason
            .get_or_insert("Android installation cancelled");
    }

    /// None always means the caller must retain this owner and the directory lock.
    /// A kill failure also leaves the child handle available for a later Stop retry.
    pub fn poll(&mut self) -> Result<Option<Outcome>, String> {
        if self.exit.is_none() {
            self.exit = self.child.try_wait().map_err(|e| e.to_string())?;
        }
        if self.child.abandoned_descendants {
            self.stop_reason.get_or_insert(
                "Android installer exited while its child processes were still running",
            );
        }
        if let Some(exit) = self.exit {
            if self.readers.iter().any(|reader| !reader.is_finished()) {
                return Ok(None);
            }
            for reader in self.readers.drain(..) {
                reader
                    .join()
                    .map_err(|_| "Android installer output reader failed")?;
            }
            return Ok(Some(Outcome {
                success: exit.success() && self.stop_reason.is_none(),
                output: self.output(),
                stop_reason: self.stop_reason,
            }));
        }
        if Instant::now() >= self.deadline {
            self.stop_reason
                .get_or_insert("Android installation exceeded its deadline");
        }
        if self.stop_reason.is_some() {
            self.child.kill().map_err(|e| format!("Cannot stop Android installer yet: {e}. Its process handle is retained; retry Stop."))?;
        }
        Ok(None)
    }

    pub fn output(&self) -> String {
        String::from_utf8_lossy(
            &self
                .output
                .lock()
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
        )
        .into_owned()
    }
}

fn drain(
    mut stream: impl Read + Send + 'static,
    output: Arc<Mutex<VecDeque<u8>>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0; 8192];
        loop {
            let read = match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            let mut output = output.lock().unwrap();
            let discard = output
                .len()
                .saturating_add(read)
                .saturating_sub(OUTPUT_LIMIT);
            output.drain(..discard);
            output.extend(&buffer[..read]);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture(mode: &str, timeout: Duration) -> InstallerChild {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "android::installer_process::tests::installer_child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("SIMPLEBENCH_INSTALLER_CHILD_FIXTURE", mode);
        InstallerChild::spawn(command, timeout).unwrap()
    }

    fn finish(child: &mut InstallerChild) -> Outcome {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(outcome) = child.poll().unwrap() {
                return outcome;
            }
            assert!(Instant::now() < deadline, "Installer fixture did not exit");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn installer_output_is_bounded_and_cancellation_waits_for_exit() {
        let mut child = fixture("output", Duration::from_secs(5));
        let result = finish(&mut child);
        assert!(result.success, "{result:?}");
        assert!(result.output.len() <= OUTPUT_LIMIT);
        assert!(result.output.contains("xxxxxxxx"));
        let mut child = fixture("wait", Duration::from_secs(5));
        child.cancel();
        let result = finish(&mut child);
        assert!(!result.success);
        assert_eq!(result.stop_reason, Some("Android installation cancelled"));
        assert!(child.child.try_wait().unwrap().is_some());
        let mut child = fixture("wait", Duration::from_millis(50));
        let result = finish(&mut child);
        assert_eq!(
            result.stop_reason,
            Some("Android installation exceeded its deadline")
        );
        assert!(child.child.try_wait().unwrap().is_some());
    }

    #[test]
    fn installer_parent_exit_cannot_leave_a_writer_or_mutator_behind() {
        let mut child = fixture("orphan", Duration::from_secs(5));
        let outcome = finish(&mut child);
        assert!(!outcome.success);
        assert_eq!(
            outcome.stop_reason,
            Some("Android installer exited while its child processes were still running")
        );
        assert!(child.child.try_wait().unwrap().is_some());
        let pid: u32 = outcome
            .output
            .lines()
            .find_map(|line| line.strip_prefix("DESCENDANT="))
            .unwrap()
            .parse()
            .unwrap();
        assert!(super::super::process_identity::Identity::read(pid).is_err());
    }

    #[test]
    #[ignore = "Owned subprocess for installer lifecycle tests"]
    fn installer_child_fixture() {
        match std::env::var("SIMPLEBENCH_INSTALLER_CHILD_FIXTURE")
            .unwrap()
            .as_str()
        {
            "output" => {
                let buffer = vec![b'x'; OUTPUT_LIMIT * 3];
                std::io::stdout().write_all(&buffer).unwrap();
                std::io::stderr().write_all(&buffer).unwrap();
                println!("LAST OUTPUT");
            }
            "wait" => thread::sleep(Duration::from_secs(60)),
            "orphan" => {
                let child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "android::installer_process::tests::installer_child_fixture",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env("SIMPLEBENCH_INSTALLER_CHILD_FIXTURE", "wait")
                    .spawn()
                    .unwrap();
                println!("DESCENDANT={}", child.id());
                // The outer Tree owns this intentionally orphaned fixture process.
                drop(child);
            }
            _ => panic!("Invalid installer fixture mode"),
        }
    }
}
