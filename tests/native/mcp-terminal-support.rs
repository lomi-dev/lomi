//! P0 native PTY evidence using the same shell builder and integration scripts.
//! This does not yet qualify the MCP terminal adapter or the xterm ACK path.
use crate::shell::{self, Profile};
use lomi_control_core::terminal::{Prompt, TerminalControl};
use lomi_control_core::terminal_io;
use lomi_control_protocol::ErrorCode;
use std::os::fd::BorrowedFd;
use std::{
    io::Read,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn until(monitor: &Arc<Mutex<TerminalControl>>, predicate: impl Fn(&TerminalControl) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if predicate(&monitor.lock().unwrap()) {
            return;
        }
        assert!(Instant::now() < deadline, "PTY condition did not arrive");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "Native macOS PTY qualification; creates and cleans its own Zsh process"]
fn zsh_prompt_completion_unicode_server_and_manual_takeover() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    std::fs::create_dir(&config).unwrap();
    std::fs::write(
        config.join(".zshrc"),
        "PROMPT='fixture> '\nHISTFILE=/dev/null\n",
    )
    .unwrap();
    let integration = directory.path().join("integration");
    shell::prepare(&integration).unwrap();
    let profile = Profile {
        id: "fixture:zsh".into(),
        name: "Zsh fixture".into(),
        kind: "zsh".into(),
        program: "/bin/zsh".into(),
        distro: None,
        home: config.to_string_lossy().into_owned(),
    };
    let (mut command, _) =
        shell::build(&profile, &directory.path().to_string_lossy(), &integration).unwrap();
    command.env("LOMI_ZDOTDIR", &config);
    command.env_remove("HISTFILE");
    let pair = portable_pty::native_pty_system()
        .openpty(portable_pty::PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let writer = pair.master.take_writer().unwrap();
    let raw_fd = pair.master.as_raw_fd().unwrap();
    // The master stays owned until the reader thread is joined.
    let fd = unsafe { BorrowedFd::borrow_raw(raw_fd) };
    terminal_io::make_nonblocking(fd).unwrap();
    let monitor = Arc::new(Mutex::new(
        TerminalControl::new("native-fixture".into(), "fixture-pty-generation".into()).unwrap(),
    ));
    let reading_monitor = monitor.clone();
    let reading = std::thread::spawn(move || {
        let mut bytes = [0; 4096];
        loop {
            match reader.read(&mut bytes) {
                Ok(0) => break,
                Ok(count) => reading_monitor.lock().unwrap().observe(&bytes[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    // The parent retains the master through join, including failure cleanup.
                    if terminal_io::wait_readable(unsafe { BorrowedFd::borrow_raw(raw_fd) })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    // Reap the private process even if an assertion fails.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        until(&monitor, |m| m.prompt() == Prompt::Ready);
        let lease = monitor.lock().unwrap().lease().unwrap().to_string();
        let send =
            |bytes: &[u8]| terminal_io::write(fd, bytes, Duration::from_secs(2), || true).unwrap();
        let run = |operation: &str, text: &str| {
            let busy = pair
                .master
                .process_group_leader()
                .is_some_and(|group| Some(group as u32) != child.process_id());
            let packet = monitor
                .lock()
                .unwrap()
                .prepare_run(&lease, operation, text, busy)
                .unwrap();
            terminal_io::write(fd, &packet, Duration::from_secs(2), || {
                monitor.lock().unwrap().lease() == Some(lease.as_str())
            })
            .unwrap();
        };
        run("unicode", "printf 'MCP_NATIVE:%s\\n' 'Zażółć 🙂'");
        until(&monitor, |m| {
            m.command("unicode").is_some_and(|c| c.completed) && m.prompt() == Prompt::Ready
        });
        {
            let state = monitor.lock().unwrap();
            let command = state.command("unicode").unwrap();
            assert!(command.started_observed);
            assert_eq!(command.exit_code, Some(0));
            assert!(state
                .read(Some(command.start_cursor), 65536)
                .unwrap()
                .text
                .contains("MCP_NATIVE:Zażółć 🙂"));
        }
        run("failure", "false");
        until(&monitor, |m| {
            m.command("failure").is_some_and(|c| c.completed) && m.prompt() == Prompt::Ready
        });
        assert_eq!(
            monitor
                .lock()
                .unwrap()
                .command("failure")
                .unwrap()
                .exit_code,
            Some(1)
        );
        let packet = b"printf x >> input-once\r";
        assert_eq!(
            monitor
                .lock()
                .unwrap()
                .prepare_input(&lease, 1, packet)
                .unwrap(),
            None
        );
        send(packet);
        monitor.lock().unwrap().finish_input(1, true);
        assert_eq!(
            monitor
                .lock()
                .unwrap()
                .prepare_input(&lease, 1, packet)
                .unwrap(),
            Some(lomi_control_core::terminal::InputReceipt::Dispatched)
        );
        until(&monitor, |m| m.prompt() == Prompt::Ready);
        assert_eq!(
            std::fs::read(directory.path().join("input-once")).unwrap(),
            b"x"
        );
        run("server", "sleep 30");
        until(&monitor, |m| {
            m.command("server").is_some_and(|c| c.started_observed)
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !monitor.lock().unwrap().command("server").unwrap().completed,
            "Output silence cannot finish a running command"
        );
        assert_eq!(
            monitor
                .lock()
                .unwrap()
                .prepare_run(&lease, "illegal", "echo WRONG", true)
                .unwrap_err(),
            ErrorCode::TargetBusy
        );
        send(&[3]);
        until(&monitor, |m| {
            m.command("server").is_some_and(|c| c.completed) && m.prompt() == Prompt::Ready
        });
        assert_eq!(
            monitor.lock().unwrap().command("server").unwrap().exit_code,
            Some(130)
        );
        // A human keystroke revokes input before the same serialized writer sends it.
        monitor.lock().unwrap().manual_input();
        send(b"echo HUMAN");
        assert_eq!(
            monitor
                .lock()
                .unwrap()
                .prepare_run(&lease, "after-takeover", "echo WRONG", false)
                .unwrap_err(),
            ErrorCode::ControlRevoked
        );
        send(b"\r");
        until(&monitor, |m| {
            m.read(None, 65536).unwrap().text.contains("HUMAN\r\n")
        });
        assert_eq!(
            monitor.lock().unwrap().lease(),
            None,
            "A later shell prompt cannot restore authority"
        );
    }));
    let _ = child.kill();
    let _ = child.wait();
    drop(writer);
    reading.join().unwrap();
    drop(pair.master);
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}
