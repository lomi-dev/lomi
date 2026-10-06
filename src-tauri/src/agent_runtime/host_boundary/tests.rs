use super::*;
use std::{io::Read, process::Command};
#[test]
fn compiled_worker_entry() {
    if let Some(root) = std::env::var_os("LOMI_HOST_BOUNDARY_TEST_ROOT") {
        let result = worker(Path::new(&root));
        if let Err(ref e) = result {
            let _ = std::fs::write(Path::new(&root).join("worker-error"), e);
        }
        std::process::exit(result.unwrap_or(2));
    }
}
#[test]
fn compiled_native_runner_entry() {
    if let Some(root) = std::env::var_os("LOMI_HOST_NATIVE_CHILD_ROOT") {
        std::process::exit(native_runner(Path::new(&root)).unwrap_or(2));
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    base: PathBuf,
    project: PathBuf,
    account: PathBuf,
    tmp: PathBuf,
    program: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("lomi-host-boundary-fixture-")
            .tempdir()
            .unwrap();
        let root_path = root.path().canonicalize().unwrap();
        fs::set_permissions(&root_path, fs::Permissions::from_mode(0o700)).unwrap();
        let base = root_path.join("records");
        let project = root_path.join("project");
        let account = root_path.join("account");
        let tmp = root_path.join("tmp");
        for path in [&base, &project, &account, &tmp] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let source = root_path.join("fixture.c");
        let program = root_path.join("fixture");
        fs::write(&source,br#"#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <netinet/in.h>
#include <arpa/inet.h>
extern char **environ;
static void pidfile(const char *root,const char *name){char p[4096];snprintf(p,sizeof(p),"%s/%s",root,name);int fd=open(p,O_WRONLY|O_CREAT|O_APPEND,0600);if(fd<0)_exit(31);dprintf(fd,"%d\n",getpid());close(fd);}
int main(int n,char **v){if(n<3)return 2;pidfile(v[2],"native.pid");
if(!strcmp(v[1],"tcp")){struct sockaddr_in a={0};a.sin_family=AF_INET;a.sin_addr.s_addr=htonl(INADDR_LOOPBACK);a.sin_port=htons(atoi(v[3]));int fd=socket(AF_INET,SOCK_STREAM,0);if(fd<0||connect(fd,(struct sockaddr*)&a,sizeof(a))<0)return 18;close(fd);a.sin_port=htons(atoi(v[4]));fd=socket(AF_INET,SOCK_STREAM,0);if(fd<0||connect(fd,(struct sockaddr*)&a,sizeof(a))==0||(errno!=EPERM&&errno!=EACCES))return 19;close(fd);struct sockaddr_in6 b={0};b.sin6_family=AF_INET6;b.sin6_addr=in6addr_loopback;b.sin6_port=htons(atoi(v[4]));fd=socket(AF_INET6,SOCK_STREAM,0);if(fd<0||connect(fd,(struct sockaddr*)&b,sizeof(b))==0||(errno!=EPERM&&errno!=EACCES))return 20;close(fd);b.sin6_port=htons(atoi(v[3]));fd=socket(AF_INET6,SOCK_STREAM,0);int v6=connect(fd,(struct sockaddr*)&b,sizeof(b));int v6_errno=errno;close(fd);inet_pton(AF_INET,"127.0.0.2",&a.sin_addr);a.sin_port=htons(atoi(v[3]));fd=socket(AF_INET,SOCK_STREAM,0);int alias=connect(fd,(struct sockaddr*)&a,sizeof(a));int alias_errno=errno;close(fd);printf("approved-port shadow IPv6 %s; IPv4 alias %s\n",v6==0?"allowed":"denied",alias==0?"allowed":"denied");if((alias<0&&alias_errno!=EPERM&&alias_errno!=EACCES)||(v6<0&&v6_errno!=EPERM&&v6_errno!=EACCES))return 22;a.sin_addr.s_addr=htonl(INADDR_LOOPBACK);a.sin_port=0;fd=socket(AF_INET,SOCK_STREAM,0);if(fd<0||bind(fd,(struct sockaddr*)&a,sizeof(a))<0||listen(fd,1)<0)return 21;close(fd);puts("exact loopback allowed; other IPv4 and IPv6 ports denied");return 0;}
if(!strcmp(v[1],"unix")){struct sockaddr_un a={0};a.sun_family=AF_UNIX;snprintf(a.sun_path,sizeof(a.sun_path),"%s",v[3]);int fd=socket(AF_UNIX,SOCK_STREAM,0);if(fd<0||connect(fd,(struct sockaddr*)&a,sizeof(a))<0)return 13;if(write(fd,"owned canary",12)!=12)return 14;close(fd);if(chmod(v[3],0700)==0||(errno!=EPERM&&errno!=EACCES))return 15;if(unlink(v[3])==0||(errno!=EPERM&&errno!=EACCES))return 16;if(rename(v[3],v[4])==0||(errno!=EPERM&&errno!=EACCES))return 17;puts("unix connection allowed; mutation denied");return 0;}
if(!strcmp(v[1],"pty")){struct winsize w;if(!isatty(0)||!isatty(1)||!isatty(2)||ioctl(0,TIOCGWINSZ,&w)<0)return 10;printf("pty ready %d %d\n",w.ws_row,w.ws_col);char b[32];if(!fgets(b,sizeof(b),stdin))return 11;if(ioctl(0,TIOCGWINSZ,&w)<0)return 12;printf("pty resized %d %d\n",w.ws_row,w.ws_col);while(fgets(b,sizeof(b),stdin)){}return 0;}
if(!strcmp(v[1],"version")){puts("fixture 1.0");return 0;}
if(!strcmp(v[1],"sentinel")){sleep(60);return 0;}
if(!strcmp(v[1],"echo")){int c;while((c=getchar())!=EOF)putchar(c);return ferror(stdin)||ferror(stdout)?6:0;}
if(!strcmp(v[1],"large-output")){for(int i=0;i<65536;i++)putchar('x');puts("\nlarge-output-complete");return ferror(stdout)?6:0;}
if(!strcmp(v[1],"exit-error")){fputs("fixture deliberate native error\n",stderr);return 7;}
if(!strcmp(v[1],"deny-read")){for(int i=3;i<n;i++){int fd=open(v[i],O_RDONLY);if(fd>=0){close(fd);return 8;}if(errno!=EPERM&&errno!=EACCES)return 9;printf("denied read %d\n",i-2);}return 0;}
if(!strcmp(v[1],"deny-control")){int fd=open(v[3],O_WRONLY|O_TRUNC);int denied=fd<0&&(errno==EPERM||errno==EACCES);if(fd>=0)close(fd);char p[4096];snprintf(p,sizeof(p),"%s/control-result",v[2]);fd=open(p,O_CREAT|O_WRONLY,0600);dprintf(fd,"%d",denied);close(fd);return denied?0:5;}
pid_t c=fork();if(c<0)return 3;if(c==0){if(setsid()<0)_exit(4);pid_t g=fork();if(g<0)_exit(5);if(g>0)_exit(0);static char *empty[]={NULL};environ=empty;for(int fd=0;fd<256;fd++)close(fd);pidfile(v[2],"descendants.pid");if(!strcmp(v[1],"fork-race")){for(int i=0;i<40;i++){pid_t x=fork();if(x==0){pidfile(v[2],"descendants.pid");sleep(60);_exit(0);}usleep(10000);}}sleep(60);_exit(0);}usleep(100000);return 0;}
"#).unwrap();
        let status = Command::new("/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/clang")
            .args(["-isysroot", "/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.5.sdk"])
            .args(["-o", program.to_str().unwrap(), source.to_str().unwrap()])
            .status()
            .unwrap();
        assert!(status.success());
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            _root: root,
            base,
            project,
            account,
            tmp,
            program,
        }
    }
    fn scope(&self) -> Scope {
        Scope {
            operation_id: crate::agent_runtime::new_id().unwrap(),
            parent_operation_id: None,
            physical_account_root: None,
            storage_root: None,
            account_id: "fixture-account".into(),
            auth_revision: 1,
            task_id: Some("fixture-task".into()),
            attempt_id: Some("fixture-attempt".into()),
            generation: Some(1),
            project_root: self.project.clone(),
            purpose: Purpose::Attempt,
        }
    }
    fn spec(&self, mode: &str) -> NativeSpec {
        NativeSpec {
            program: self.program.clone(),
            arguments: vec![mode.into(), self.project.to_string_lossy().into()],
            environment: vec![],
            cwd: self.project.clone(),
            io: NativeIo::Pipes,
            policy: sandbox::Policy {
                project_root: self.project.clone(),
                account_root: self.account.clone(),
                temp_root: self.tmp.clone(),
                runtime_reads: vec![self.program.clone()],
                protected_reads: vec![],
                blocked_reads: vec![],
                runtime_read_roots: vec![],
                codex_preferences: false,
                loopback_tcp_ports: vec![],
                loopback_listener: false,
                unix_sockets: vec![],
                allow_native_tools: false,
            },
        }
    }
    fn pids(&self, held_pid: u32) -> Vec<u32> {
        let mut pids = vec![held_pid];
        for name in ["native.pid", "descendants.pid"] {
            if let Ok(text) = fs::read_to_string(self.project.join(name)) {
                pids.extend(text.lines().filter_map(|s| s.parse::<u32>().ok()));
            }
        }
        pids.sort();
        pids.dedup();
        pids
    }
    fn retire(&self, b: &mut Boundary) -> Result<RetiredReceipt, String> {
        let held = b.record.held.as_ref().unwrap().identity.pid;
        eprintln!(
            "owned worker pid={held} verification hash millis: {:?}",
            fs::read_to_string(b.root.join("worker-hash-millis"))
        );
        b.retire_using(|| Ok(self.pids(held)))
    }
    fn wait(&self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.project.join(name).exists() {
            assert!(Instant::now() < deadline, "fixture did not write {name}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn captured_native(f: &Fixture, spec: NativeSpec) -> (String, String, RetiredReceipt, String) {
    captured_native_input(f, spec, None)
}

fn captured_native_input(
    f: &Fixture,
    spec: NativeSpec,
    input: Option<Vec<u8>>,
) -> (String, String, RetiredReceipt, String) {
    let mut scope = f.scope();
    scope.purpose = Purpose::VersionProbe;
    captured_native_scope(f, scope, spec, input)
}

fn captured_native_scope(
    f: &Fixture,
    scope: Scope,
    spec: NativeSpec,
    input: Option<Vec<u8>>,
) -> (String, String, RetiredReceipt, String) {
    let mut b = Boundary::create(&f.base, scope, spec).unwrap();
    let held = b.record.held.as_ref().unwrap().identity.pid;
    let mut transport = b.take_transport().unwrap();
    b.release().unwrap();
    let input = input.map(|bytes| {
        let mut stream = transport.stdin;
        std::thread::spawn(move || write_stream(&mut stream, &bytes))
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut drain = || {
        for (stream, output) in [
            (&mut transport.stdout, &mut stdout),
            (&mut transport.stderr, &mut stderr),
        ] {
            loop {
                let mut bytes = [0; 4096];
                match stream.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(n) => output.extend_from_slice(&bytes[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => panic!("owned native stream failed: {e}"),
                }
                assert!(output.len() < 1024 * 1024);
            }
        }
    };
    let exited = loop {
        drain();
        if kernel::gone(held) {
            drain();
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();
    eprintln!("owned native pid={held}, exited={exited}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    let job = format!("{}/{}", b.record.domain, b.record.label);
    let job_status = Command::new("/bin/launchctl")
        .args(["print", &job])
        .env_clear()
        .output()
        .unwrap();
    let job_status = String::from_utf8_lossy(&job_status.stdout).into_owned();
    eprintln!("owned launchd status before sealing:\n{job_status}");
    // Process exit governs capture only; only the sealed coalition supplies
    // retirement evidence, including when native startup fails or times out.
    let receipt = f.retire(&mut b).unwrap();
    eprintln!("kernel retirement receipt: {receipt:?}");
    if let Some(input) = input {
        input.join().unwrap().unwrap();
    }
    assert!(exited, "owned version/help command did not exit");
    (stdout, stderr, receipt, job_status)
}

#[test]
#[ignore = "Opt-in owned filesystem Data-volume alias denial fixture"]
fn outside_files_and_boundary_control_are_unreadable_through_data_volume_aliases() {
    let f = Fixture::new();
    let outside = f._root.path().canonicalize().unwrap().join("outside");
    fs::write(&outside, b"owned outside sentinel").unwrap();
    let scope = f.scope();
    let control = f.base.join(&scope.operation_id).join("record.json");
    let transport = control.parent().unwrap().join("stdin");
    let alias =
        |path: &Path| Path::new("/System/Volumes/Data").join(path.strip_prefix("/").unwrap());
    assert_eq!(
        fs::read(alias(&outside)).unwrap(),
        b"owned outside sentinel"
    );
    let mut spec = f.spec("deny-read");
    for path in [
        &outside,
        &alias(&outside),
        &control,
        &alias(&control),
        &transport,
        &alias(&transport),
    ] {
        spec.arguments.push(path.to_string_lossy().into());
    }
    let (stdout, stderr, _, status) = captured_native_scope(&f, scope, spec, None);
    assert_eq!(
        stdout,
        "denied read 1\ndenied read 2\ndenied read 3\ndenied read 4\ndenied read 5\ndenied read 6\n"
    );
    assert!(stderr.contains("native exit: code=Some(0) signal=None"));
    assert!(status.contains("last exit code = 0"));
}

fn public_cli_version_and_help(name: &str, expected_digest: &str, version: &str) {
    let root = std::env::var_os("LOMI_HOST_BOUNDARY_PUBLIC_CLI_ROOT")
        .expect("set LOMI_HOST_BOUNDARY_PUBLIC_CLI_ROOT to the pinned public artifact fixture");
    let root = Path::new(&root).canonicalize().unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert!(manifest["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| { entry["name"] == name && entry["native_sha256"] == expected_digest }));
    let program = root.join(name).canonicalize().unwrap();
    assert_eq!(binary_digest(&program).unwrap(), expected_digest);
    for argument in ["--version", "--help"] {
        let f = Fixture::new();
        let mut spec = f.spec("version");
        spec.program = program.clone();
        spec.policy.runtime_reads = vec![program.clone()];
        spec.arguments = vec![argument.into()];
        spec.environment = vec![
            ("HOME".into(), f.account.to_string_lossy().into()),
            ("TMPDIR".into(), f.tmp.to_string_lossy().into()),
            ("CLAUDE_CODE_TMPDIR".into(), f.tmp.to_string_lossy().into()),
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("TERM".into(), "dumb".into()),
        ];
        for (key, relative) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_DATA_HOME", "data"),
            ("CODEX_HOME", "codex"),
            ("CLAUDE_CONFIG_DIR", "claude"),
        ] {
            let path = f.account.join(relative);
            fs::create_dir(&path).unwrap();
            spec.environment
                .push((key.into(), path.to_string_lossy().into()));
        }
        eprintln!("pinned public {name} {argument}, sha256={expected_digest}");
        let (stdout, _, _, _) = captured_native(&f, spec);
        assert!(stdout.contains(if argument == "--version" {
            version
        } else {
            "Usage:"
        }));
        if argument == "--help" {
            // The final help section also verifies output beyond FIFO capacity.
            assert!(stdout.contains(if name == "claude" {
                "update|upgrade"
            } else {
                "Print version"
            }));
        }
    }
}

#[test]
#[ignore = "Opt-in pinned public CLI fixture; no login, accounts, or inference"]
fn pinned_public_codex_version_and_help_use_the_full_held_boundary() {
    public_cli_version_and_help(
        "codex",
        "112fae7a5a1223e673c8a1791d32338f37df8b527ff1159bb8adac6c4dbf1b4b",
        "0.160.0",
    );
}

#[test]
#[ignore = "Opt-in pinned public CLI fixture; no login, accounts, or inference"]
fn pinned_public_claude_version_and_help_use_the_full_held_boundary() {
    public_cli_version_and_help(
        "claude",
        "6eab8333fe2121553100d8f40bfada384a3e989b94f947e18ba6677a6fcb41ea",
        "2.1.287",
    );
}

#[test]
#[ignore = "Opt-in owned launchd delegation refusal fixture"]
fn launchd_delegation_is_refused_and_an_unrelated_owned_sentinel_survives_stop() {
    use std::os::unix::process::CommandExt;
    let sentinel = Fixture::new();
    let child = Command::new(&sentinel.program)
        .args(["sentinel", sentinel.project.to_str().unwrap()])
        .env_clear()
        .process_group(0)
        .spawn()
        .unwrap();
    struct OwnedSentinel {
        child: std::process::Child,
        member: kernel::Member,
    }
    impl Drop for OwnedSentinel {
        fn drop(&mut self) {
            let _ = kernel::signal_member(&self.member, self.member.resource_coalition);
            let _ = self.child.wait();
        }
    }
    let member = kernel::member(child.id()).unwrap();
    let mut sentinel_child = OwnedSentinel { child, member };
    sentinel.wait("native.pid");
    let f = Fixture::new();
    let label = format!(
        "dev.lomi.fixture.delegation.{}",
        super::super::new_id().unwrap()
    );
    let domain = format!("gui/{}", unsafe { libc::getuid() });
    struct OwnedJob(String);
    impl Drop for OwnedJob {
        fn drop(&mut self) {
            let _ = launchctl(&["bootout", &self.0]);
        }
    }
    let cleanup = OwnedJob(format!("{domain}/{label}"));
    let plist = f.project.join("delegation.plist");
    fs::write(&plist, format!(
        "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>RunAtLoad</key><true/><key>ProgramArguments</key><array><string>{}</string><string>version</string><string>{}</string></array></dict></plist>",
        xml(&label), xml(f.program.to_str().unwrap()), xml(f.project.to_str().unwrap()),
    )).unwrap();
    let mut spec = f.spec("version");
    spec.program = Path::new("/bin/launchctl").canonicalize().unwrap();
    spec.policy.runtime_reads = vec![spec.program.clone(), f.program.clone()];
    spec.arguments = vec![
        "bootstrap".into(),
        domain.clone(),
        plist.to_string_lossy().into(),
    ];
    let (_, stderr, _, _) = captured_native(&f, spec);
    let delegated = f.project.join("native.pid").exists();
    // Even an unexpected successful delegation remains an owned job, and is
    // cleaned before reporting the failed refusal assertion.
    drop(cleanup);
    assert!(sentinel_child.child.try_wait().unwrap().is_none());
    assert_eq!(
        kernel::member(sentinel_child.member.identity.pid).unwrap(),
        sentinel_child.member
    );
    drop(sentinel_child);
    assert!(!delegated, "sandboxed launchctl delegated execution");
    assert!(stderr.contains("Bootstrap failed") || stderr.contains("Could not"));
}

#[test]
#[ignore = "Opt-in owned anonymous-pipe stdin/backpressure/native-outcome fixture"]
fn anonymous_pipe_relays_roundtrip_stdin_and_drain_large_output_and_native_errors() {
    let f = Fixture::new();
    let payload = vec![b'e'; 32768];
    let (stdout, stderr, _, status) =
        captured_native_input(&f, f.spec("echo"), Some(payload.clone()));
    assert_eq!(stdout.as_bytes(), payload);
    assert!(stderr.contains("native exit: code=Some(0) signal=None"));
    assert!(status.contains("last exit code = 0"));
    let f = Fixture::new();
    let (stdout, stderr, _, _) = captured_native(&f, f.spec("large-output"));
    assert_eq!(
        stdout,
        format!("{}\nlarge-output-complete\n", "x".repeat(65536))
    );
    assert!(stderr.contains("native exit: code=Some(0) signal=None"));
    let f = Fixture::new();
    let (_, stderr, _, status) = captured_native(&f, f.spec("exit-error"));
    assert!(stderr.contains("fixture deliberate native error"));
    assert!(stderr.contains("native exit: code=Some(7) signal=None"));
    assert!(status.contains("last exit code = 7"));
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(entries) = fs::read_dir(&self.base) {
            for entry in entries.flatten() {
                if std::thread::panicking() {
                    let root = entry.path();
                    eprintln!("owned fixture diagnostics: {}", root.display());
                    for name in [
                        "record.json",
                        "held.json",
                        "worker-error",
                        "worker-hash-millis",
                    ] {
                        eprintln!("{name}: {:?}", fs::read_to_string(root.join(name)));
                    }
                    eprintln!(
                        "worker stages: {:?}",
                        fs::read_dir(&root)
                            .into_iter()
                            .flatten()
                            .flatten()
                            .map(|e| e.file_name())
                            .collect::<Vec<_>>()
                    );
                    for name in ["stdout", "stderr"] {
                        if let Ok(mut stream) = open_fifo(&root.join(name), true, true) {
                            let mut bytes = [0; 16384];
                            if let Ok(n) = stream.read(&mut bytes) {
                                eprintln!("{name}: {}", String::from_utf8_lossy(&bytes[..n]));
                            }
                        }
                    }
                }
                if let Ok(record) = read_json::<Record>(&entry.path().join("record.json")) {
                    let _ = launchctl(&["bootout", &format!("{}/{}", record.domain, record.label)]);
                    if let Some(held) = record.held {
                        for pid in self.pids(held.identity.pid) {
                            if let Ok(member) = kernel::member(pid) {
                                if member.resource_coalition == held.resource_coalition {
                                    let _ = kernel::signal_member(&member, held.resource_coalition);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "Experimental host boundary launch compatibility is not qualified"]
fn held_before_exec_is_durable_and_only_kernel_retirement_releases_it() {
    let f = Fixture::new();
    let b = Boundary::create(&f.base, f.scope(), f.spec("version")).unwrap();
    let cid = b.record.held.as_ref().unwrap().resource_coalition;
    assert_eq!(b.record.phase, Phase::Held);
    assert!(!f.project.join("native.pid").exists());
    assert_eq!(kernel::usage(cid).unwrap(), kernel::Usage::Alive);
    let root = b.root.clone();
    drop(b);
    let mut restored = Boundary::restore(&root).unwrap();
    let receipt = restored.retire_using(|| Ok(vec![])).unwrap();
    assert_eq!(receipt.resource_coalition, cid);
    assert_eq!(kernel::usage(cid).unwrap(), kernel::Usage::Retired);
    assert!(!f.project.join("native.pid").exists());
    assert!(!kernel::production_qualified());
}
#[test]
#[ignore = "Experimental host boundary launch compatibility is not qualified"]
fn released_stdio_is_preserved_but_not_used_as_retirement_proof() {
    let f = Fixture::new();
    let mut b = Boundary::create(&f.base, f.scope(), f.spec("version")).unwrap();
    let mut transport = b.take_transport().unwrap();
    b.release().unwrap();
    f.wait("native.pid");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = String::new();
    loop {
        let mut bytes = [0; 128];
        match transport.stdout.read(&mut bytes) {
            Ok(n) if n > 0 => output.push_str(std::str::from_utf8(&bytes[..n]).unwrap()),
            _ => {}
        }
        if output.contains("fixture 1.0") {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let receipt = f.retire(&mut b).unwrap();
    assert!(receipt.resource_coalition > 1);
}
#[test]
#[ignore = "Experimental host boundary launch compatibility is not qualified"]
fn env_clear_setsid_double_fork_and_leader_exit_remain_in_the_kernel_boundary() {
    let f = Fixture::new();
    let mut b = Boundary::create(&f.base, f.scope(), f.spec("detach")).unwrap();
    b.release().unwrap();
    f.wait("descendants.pid");
    let cid = b.record.held.as_ref().unwrap().resource_coalition;
    let detached = fs::read_to_string(f.project.join("descendants.pid"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .parse::<u32>()
        .unwrap();
    assert_eq!(kernel::member(detached).unwrap().resource_coalition, cid);
    assert_eq!(kernel::usage(cid).unwrap(), kernel::Usage::Alive);
    let root = b.root.clone();
    drop(b);
    let mut recovered = Boundary::restore(&root).unwrap();
    let receipt = f.retire(&mut recovered).unwrap();
    assert_eq!(receipt.resource_coalition, cid);
}
#[test]
#[ignore = "Experimental host boundary launch compatibility is not qualified"]
fn fork_during_stop_cannot_turn_an_empty_scan_into_a_receipt() {
    let f = Fixture::new();
    let mut b = Boundary::create(&f.base, f.scope(), f.spec("fork-race")).unwrap();
    b.release().unwrap();
    f.wait("descendants.pid");
    let cid = b.record.held.as_ref().unwrap().resource_coalition;
    let receipt = f.retire(&mut b).unwrap();
    assert_eq!(receipt.resource_coalition, cid);
    assert_eq!(kernel::usage(cid).unwrap(), kernel::Usage::Retired);
}
#[test]
#[ignore = "Experimental host boundary launch compatibility is not qualified"]
fn native_code_cannot_write_boundary_control_after_release() {
    let f = Fixture::new();
    let scope = f.scope();
    let path = f.base.join(&scope.operation_id).join("record.json");
    let mut spec = f.spec("deny-control");
    spec.arguments.push(path.to_string_lossy().into());
    let mut b = Boundary::create(&f.base, scope, spec).unwrap();
    b.release().unwrap();
    f.wait("control-result");
    assert_eq!(
        fs::read_to_string(f.project.join("control-result")).unwrap(),
        "1"
    );
    f.retire(&mut b).unwrap();
}

#[test]
#[ignore = "Owned native host-pinned PTY fixture"]
fn worker_owned_pty_preserves_stdio_resize_eof_and_trusted_outcome() {
    let f = Fixture::new();
    let mut spec = f.spec("pty");
    spec.io = NativeIo::Pty { rows: 24, cols: 80 };
    let mut b = Boundary::create(&f.base, f.scope(), spec).unwrap();
    let mut transport = b.take_transport().unwrap();
    b.release().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut output = String::new();
    while !output.contains("pty ready 24 80") {
        let mut bytes = [0; 4096];
        match transport.stdout.read(&mut bytes) {
            Ok(n) => output.push_str(&String::from_utf8_lossy(&bytes[..n])),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("PTY output: {e}"),
        }
        assert!(
            Instant::now() < deadline,
            "PTY bootstrap failed: {output}; worker: {:?}",
            fs::read_to_string(b.root.join("worker-error"))
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    b.resize(31, 95).unwrap();
    // The protected fixed-size resize channel is independent of terminal input.
    std::thread::sleep(Duration::from_millis(100));
    write_stream(&mut transport.stdin, b"ready\n").unwrap();
    drop(transport.stdin);
    let outcome = loop {
        let mut bytes = [0; 4096];
        if let Ok(n) = transport.stdout.read(&mut bytes) {
            output.push_str(&String::from_utf8_lossy(&bytes[..n]));
        }
        if let Some(status) = b.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "PTY completion failed: {output}");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(output.contains("pty resized 31 95"), "{output}");
    assert_eq!(outcome.code(), Some(0));
    f.retire(&mut b).unwrap();
}

#[test]
#[ignore = "Owned native host-pinned exit fixture"]
fn native_error_outcome_comes_from_the_owned_job() {
    let f = Fixture::new();
    let mut b = Boundary::create(&f.base, f.scope(), f.spec("exit-error")).unwrap();
    b.release().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let outcome = loop {
        if let Some(status) = b.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(outcome.code(), Some(7));
    f.retire(&mut b).unwrap();
}

#[test]
#[ignore = "Owned authenticated-socket connection and mutation canary"]
fn unix_socket_connect_is_allowed_but_unlink_chmod_and_rename_are_denied() {
    use std::os::unix::net::UnixListener;
    let f = Fixture::new();
    let socket = f._root.path().canonicalize().unwrap().join("broker.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let mode = fs::symlink_metadata(&socket).unwrap().mode();
    let mut spec = f.spec("unix");
    spec.arguments.push(socket.to_string_lossy().into_owned());
    spec.arguments
        .push(f.project.join("stolen.sock").to_string_lossy().into_owned());
    spec.policy.unix_sockets.push(socket.clone());
    let (output, errors, _, job) = captured_native(&f, spec);
    assert!(
        output.contains("unix connection allowed; mutation denied"),
        "{output} {errors} {job}"
    );
    let (mut connection, _) = listener.accept().unwrap();
    let mut data = Vec::new();
    connection.read_to_end(&mut data).unwrap();
    assert_eq!(data, b"owned canary");
    assert_eq!(fs::symlink_metadata(&socket).unwrap().mode(), mode);
    assert!(!f.project.join("stolen.sock").exists());
}

#[test]
#[ignore = "Owned live boundary parent-query cancellation regression"]
fn parent_receipt_queries_preserve_a_live_registered_effect_scope() {
    let mut f = Fixture::new();
    let storage = f._root.path().canonicalize().unwrap();
    let parent = "owned-query-fixture";
    f.base = prepare_parent(&storage, parent).unwrap();
    let mut scope = f.scope();
    scope.parent_operation_id = Some(parent.into());
    scope.storage_root = Some(storage.clone());
    scope.physical_account_root = Some(f.account.clone());
    let mut b = Boundary::create(&f.base, scope, f.spec("sentinel")).unwrap();
    b.release().unwrap();
    let held = b.record.held.as_ref().unwrap().identity.pid;
    let (_, effects) = registered_scope(held).unwrap().unwrap();
    let guard = effects.enter().unwrap();
    assert!(!parent_completed(&storage, parent).unwrap());
    assert_eq!(parent_scopes(&storage, parent).unwrap().len(), 1);
    assert!(!effects.cancelled());
    assert!(registered_scope(held).unwrap().is_some());
    drop(guard);
    f.retire(&mut b).unwrap();
    assert!(parent_completed(&storage, parent).unwrap());
    assert_eq!(parent_scopes(&storage, parent).unwrap().len(), 1);
    assert!(registered_scope(held).is_err());
}

#[test]
#[ignore = "Owned native Stop ordering with a delayed external host effect"]
fn stop_retires_the_native_cohort_before_waiting_for_a_host_effect() {
    let mut f = Fixture::new();
    let storage = f._root.path().canonicalize().unwrap();
    let parent = "owned-effect-stop-fixture";
    f.base = prepare_parent(&storage, parent).unwrap();
    let mut scope = f.scope();
    scope.parent_operation_id = Some(parent.into());
    scope.storage_root = Some(storage.clone());
    scope.physical_account_root = Some(f.account.clone());
    let mut b = Boundary::create(&f.base, scope, f.spec("sentinel")).unwrap();
    b.release().unwrap();
    f.wait("native.pid");
    let held = b.record.held.as_ref().unwrap();
    let cid = held.resource_coalition;
    let pids = f.pids(held.identity.pid);
    let effects = b.effects();
    let guard = effects.enter().unwrap();
    let stop = std::thread::spawn(move || b.retire_using(|| Ok(pids.clone())));
    let deadline = Instant::now() + Duration::from_secs(5);
    while kernel::usage(cid).unwrap() != kernel::Usage::Retired {
        assert!(
            Instant::now() < deadline,
            "host effect prevented native retirement"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(effects.cancelled());
    assert!(effects.enter().is_err());
    assert!(!parent_completed(&storage, parent).unwrap());
    assert!(!stop.is_finished(), "receipt preceded the effect drain");
    drop(guard);
    assert_eq!(stop.join().unwrap().unwrap().resource_coalition, cid);
    assert!(parent_completed(&storage, parent).unwrap());
}

#[test]
#[ignore = "Owned exact loopback policy admission and connection canary"]
fn loopback_policy_allows_only_reviewed_ports_and_local_listener() {
    use std::net::TcpListener;
    let f = Fixture::new();
    let allowed = TcpListener::bind("127.0.0.1:0").unwrap();
    let denied = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut spec = f.spec("tcp");
    let allowed_port = allowed.local_addr().unwrap().port();
    let _v6_shadow = TcpListener::bind(("::1", allowed_port)).unwrap();
    let _alias_shadow = match TcpListener::bind(("127.0.0.2", allowed_port)) {
        Ok(listener) => Some(listener),
        Err(error) if error.kind() == std::io::ErrorKind::AddrNotAvailable => {
            eprintln!("127.0.0.2 is not assigned on this host; native probe still requires EPERM/EACCES rather than a connection failure.");
            None
        }
        Err(error) => panic!("cannot bind owned alias canary: {error}"),
    };
    spec.arguments.push(allowed_port.to_string());
    spec.arguments
        .push(denied.local_addr().unwrap().port().to_string());
    spec.policy.loopback_tcp_ports.push(allowed_port);
    spec.policy.loopback_listener = true;
    let (output, errors, _, job) = captured_native(&f, spec);
    eprintln!("owned loopback probe: {output}");
    assert!(
        output.contains("approved-port shadow IPv6 allowed; IPv4 alias denied"),
        "{output} {errors} {job}"
    );
    assert!(
        output.contains("exact loopback allowed; other IPv4 and IPv6 ports denied"),
        "{output} {errors} {job}"
    );
}
