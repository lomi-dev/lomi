use super::process_owner::{exit_pending, OwnedChild};
use std::{
    fs,
    io::{ErrorKind, Read},
    os::unix::{io::AsRawFd, process::CommandExt},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn resolved_client_still_requires_trusted_executable_identity() {
    use crate::{cli_catalog::TitleCli, shell, terminal::Shells};
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let client = bin.join("kimi");
    let launched = root.path().join("launched");
    fs::write(
        &client,
        format!(
            "#!/bin/sh\nprintf launched > {}\n",
            shell::quote(launched.to_str().unwrap(), "sh").unwrap()
        ),
    )
    .unwrap();
    fs::set_permissions(&client, fs::Permissions::from_mode(0o700)).unwrap();
    let wrapper = root.path().join("shell");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexport PATH={}\nexec /bin/sh \"$@\"\n",
            shell::quote(bin.to_str().unwrap(), "sh").unwrap()
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let profile = shell::Profile {
        id: "test:sh".into(),
        name: "sh".into(),
        kind: "sh".into(),
        program: wrapper.to_str().unwrap().into(),
        distro: None,
        home: root.path().to_str().unwrap().into(),
    };
    let shells = Shells {
        profiles: vec![profile],
        integration: root.path().join("integration"),
    };
    shell::prepare(&shells.integration).unwrap();
    let profile = super::process_owner::shell_profile(&shells, "test:sh").unwrap();
    let program = super::process_owner::resolve(
        &shells,
        profile,
        root.path().to_str().unwrap(),
        TitleCli::Kimi,
    )
    .unwrap();
    assert_eq!(program, client.canonicalize().unwrap());
    assert!(super::process_owner::executable(&program).is_ok());
    fs::set_permissions(&client, fs::Permissions::from_mode(0o722)).unwrap();
    assert!(super::process_owner::executable(&program).is_err());
    assert!(!launched.exists());
}

fn fixture(script: &str) -> (tempfile::TempDir, OwnedChild) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("owned-process.sh");
    fs::write(&path, script).unwrap();
    let child = Command::new("/bin/sh")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap();
    (root, OwnedChild::new(child))
}

fn make_nonblocking(reader: &impl AsRawFd) {
    let descriptor = reader.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
}

fn read_until(
    reader: &mut impl Read,
    output: &mut Vec<u8>,
    complete: impl Fn(&[u8], bool) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut bytes = [0; 2048];
        let eof = match reader.read(&mut bytes) {
            Ok(0) => true,
            Ok(count) => {
                output.extend_from_slice(&bytes[..count]);
                false
            }
            Err(cause) if cause.kind() == ErrorKind::WouldBlock => false,
            Err(cause) => panic!("fixture stream failed: {cause}"),
        };
        if complete(output, eof) {
            return;
        }
        assert!(Instant::now() < deadline, "fixture stream did not settle");
        thread::sleep(Duration::from_millis(10));
    }
}

fn live_process(pid: u32) -> bool {
    let output = Command::new("/bin/ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let status = String::from_utf8(output.stdout).unwrap();
    let status = status.trim();
    !status.is_empty() && !status.starts_with('Z')
}

#[test]
fn owned_group_settles_child_grandchild_and_streams_before_target() {
    let (_root, mut source) = fixture(
        r#"
case "$1" in
  grandchild) printf 'grandchild:%s\n' "$$"; while :; do sleep 1; done ;;
  child) printf 'child:%s\n' "$$"; /bin/sh "$0" grandchild & wait ;;
  *) printf 'leader:%s\n' "$$"; /bin/sh "$0" child & wait ;;
esac
"#,
    );
    let mut stdout = source.stdout.take().unwrap();
    let mut stderr = source.stderr.take().unwrap();
    make_nonblocking(&stdout);
    make_nonblocking(&stderr);
    let mut output = Vec::new();
    read_until(&mut stdout, &mut output, |bytes, _| {
        String::from_utf8_lossy(bytes).contains("grandchild:")
    });
    let descendants: Vec<u32> = String::from_utf8(output.clone())
        .unwrap()
        .lines()
        .filter(|line| line.starts_with("child:") || line.starts_with("grandchild:"))
        .map(|line| line.split_once(':').unwrap().1.parse().unwrap())
        .collect();
    assert_eq!(descendants.len(), 2);
    assert!(descendants.iter().all(|pid| live_process(*pid)));
    source.stop_and_wait().unwrap();
    read_until(&mut stdout, &mut output, |_, eof| eof);
    read_until(&mut stderr, &mut Vec::new(), |_, eof| eof);
    assert!(descendants.iter().all(|pid| !live_process(*pid)));

    // The target is created only after the owned leader, descendants and streams
    // have settled. No native provider or credential is involved in this fixture.
    let status = Command::new("/bin/sh")
        .args(["-c", "exit 0"])
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn exited_leader_does_not_prove_descendant_stream_eof() {
    let (_root, mut source) = fixture(
        r#"
if [ "$1" = child ]; then
  printf 'child:%s\n' "$$"
  while :; do sleep 1; done
fi
/bin/sh "$0" child &
exit 0
"#,
    );
    let mut stdout = source.stdout.take().unwrap();
    make_nonblocking(&stdout);
    let mut output = Vec::new();
    read_until(&mut stdout, &mut output, |bytes, _| {
        String::from_utf8_lossy(bytes).contains("child:")
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !exit_pending(&source).unwrap() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        stdout.read(&mut [0; 1]).unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    source.stop_and_wait().unwrap();
    read_until(&mut stdout, &mut output, |_, eof| eof);
}
