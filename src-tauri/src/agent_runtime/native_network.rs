//! A bounded CONNECT transport. TLS and authentication remain in the native
//! client. No request body is interpreted, logged or translated by Lomi.
use super::host_boundary::{EffectGuard, EffectScope};
use crate::cli_catalog::TitleCli;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const MAX_CONNECTIONS: usize = 32;
const HEADER_LIMIT: usize = 8192;
const IO_WAIT: Duration = Duration::from_millis(250);

fn failure() -> String {
    "The native provider transport is unavailable under the restricted host policy.".into()
}

// Explicit provider endpoints, never arbitrary URLs from a task or environment.
// Auth redirects opened in the user's browser do not pass through this transport.
fn hosts(cli: TitleCli) -> Result<&'static [&'static str], String> {
    Ok(match cli {
        TitleCli::Codex | TitleCli::Pi => &["api.openai.com", "chatgpt.com", "auth.openai.com"],
        TitleCli::Claude => &[
            "api.anthropic.com",
            "claude.ai",
            "claude.com",
            "platform.claude.com",
            "console.anthropic.com",
        ],
        TitleCli::Kimi => &[
            "api.kimi.com",
            "auth.kimi.com",
            "www.kimi.com",
            "kimi.com",
            "api.moonshot.ai",
            "api.moonshot.cn",
        ],
        TitleCli::Kilo => &[
            "api.kilo.ai",
            "kilo.ai",
            "app.kilo.ai",
            "openrouter.ai",
            "api.openai.com",
            "api.anthropic.com",
            "api.x.ai",
        ],
        TitleCli::Opencode => &[
            "opencode.ai",
            "api.openai.com",
            "chatgpt.com",
            "auth.openai.com",
            "api.anthropic.com",
            "claude.ai",
            "platform.claude.com",
            "openrouter.ai",
            "api.x.ai",
        ],
        TitleCli::Grok => &["api.x.ai", "accounts.x.ai"],
        TitleCli::Agy => &[
            "oauth2.googleapis.com",
            "accounts.google.com",
            "cloudcode-pa.googleapis.com",
            "daily-cloudcode-pa.googleapis.com",
            "generativelanguage.googleapis.com",
        ],
        _ => return Err(failure()),
    })
}

struct State {
    stop: AtomicBool,
    effects: Mutex<Option<Arc<EffectScope>>>,
    connections: Mutex<HashMap<u64, Vec<TcpStream>>>,
    sequence: AtomicU64,
    authorization: Zeroizing<String>,
    hosts: &'static [&'static str],
    #[cfg(test)]
    fixture_upstream: Mutex<Option<SocketAddr>>,
}
impl State {
    fn cancelled(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
            || self
                .effects
                .lock()
                .map(|e| e.as_ref().is_some_and(|s| s.cancelled()))
                .unwrap_or(true)
    }
    fn close_connections(&self) {
        if let Ok(connections) = self.connections.lock() {
            for socket in connections.values().flatten() {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
    }
    fn enter(&self) -> Result<EffectGuard, String> {
        if self.cancelled() {
            return Err(failure());
        }
        self.effects
            .lock()
            .map_err(|_| failure())?
            .as_ref()
            .ok_or_else(failure)?
            .enter()
    }
}

pub(crate) struct Proxy {
    state: Arc<State>,
    port: u16,
    url: Zeroizing<String>,
    worker: Mutex<Option<JoinHandle<()>>>,
    // Bound originals outlive cancellation of acceptance/tunnels. The native
    // boundary retains this Proxy until positive retirement, including Drop.
    _reservations: [TcpListener; 2],
}
impl Proxy {
    pub(crate) fn start(cli: TitleCli) -> Result<Arc<Self>, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|_| failure())?;
        Self::start_with_listener(cli, listener)
    }
    fn start_with_listener(cli: TitleCli, listener: TcpListener) -> Result<Arc<Self>, String> {
        let port = listener.local_addr().map_err(|_| failure())?.port();
        // Seatbelt's localhost selector permits both families. Reserve both
        // exact endpoints before native release, or fail closed.
        let ipv6 = TcpListener::bind(("::1", port)).map_err(|_| failure())?;
        let reservations = [listener, ipv6];
        let mut listeners = Vec::new();
        for listener in &reservations {
            listener.set_nonblocking(true).map_err(|_| failure())?;
            listeners.push(listener.try_clone().map_err(|_| failure())?);
        }
        let token = Zeroizing::new(format!("{}{}", super::new_id()?, super::new_id()?));
        let authorization = Zeroizing::new(format!(
            "Basic {}",
            STANDARD.encode(format!("lomi:{}", token.as_str()))
        ));
        let state = Arc::new(State {
            stop: AtomicBool::new(false),
            effects: Mutex::new(None),
            connections: Mutex::new(HashMap::new()),
            sequence: AtomicU64::new(1),
            authorization,
            hosts: hosts(cli)?,
            #[cfg(test)]
            fixture_upstream: Mutex::new(None),
        });
        let owner = state.clone();
        let worker = thread::Builder::new()
            .name("native-provider-transport".into())
            .spawn(move || {
                let mut workers = Vec::new();
                'accepting: while !owner.cancelled() {
                    workers.retain(|worker: &JoinHandle<()>| !worker.is_finished());
                    for listener in &listeners {
                        match listener.accept() {
                            Ok((stream, peer))
                                if peer.ip().is_loopback() && workers.len() < MAX_CONNECTIONS =>
                            {
                                let state = owner.clone();
                                if let Ok(worker) = thread::Builder::new()
                                    .name("native-provider-tunnel".into())
                                    .spawn(move || {
                                        let _ = tunnel(&state, stream);
                                    })
                                {
                                    workers.push(worker);
                                }
                            }
                            Ok((stream, _)) => {
                                let _ = stream.shutdown(Shutdown::Both);
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                            Err(_) => break 'accepting,
                        }
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                owner.stop.store(true, Ordering::SeqCst);
                owner.close_connections();
                for worker in workers {
                    let _ = worker.join();
                }
            })
            .map_err(|_| failure())?;
        Ok(Arc::new(Self {
            state,
            port,
            url: Zeroizing::new(format!("http://lomi:{}@127.0.0.1:{port}", token.as_str())),
            worker: Mutex::new(Some(worker)),
            _reservations: reservations,
        }))
    }
    pub(crate) fn port(&self) -> u16 {
        self.port
    }
    pub(crate) fn environment(&self) -> Vec<(String, String)> {
        [
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "ALL_PROXY",
            "https_proxy",
            "http_proxy",
            "all_proxy",
        ]
        .into_iter()
        .map(|key| (key.into(), self.url.to_string()))
        .chain([
            ("NO_PROXY".into(), "127.0.0.1,localhost,::1".into()),
            ("no_proxy".into(), "127.0.0.1,localhost,::1".into()),
            ("CLAUDE_CODE_CERT_STORE".into(), "bundled".into()),
            ("NODE_USE_ENV_PROXY".into(), "1".into()),
        ])
        .collect()
    }
    /// Attach while the cohort is held; admission itself opens at release.
    pub(crate) fn attach(&self, scope: Arc<EffectScope>) -> Result<(), String> {
        let mut effects = self.state.effects.lock().map_err(|_| failure())?;
        if effects.is_some() || self.state.stop.load(Ordering::SeqCst) {
            return Err(failure());
        }
        *effects = Some(scope);
        Ok(())
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        self.state.close_connections();
        // DNS may be pending in a tunnel; the retained EffectGuard keeps the
        // cohort fenced until it returns. Never block the UI on resolver join.
        if let Ok(mut worker) = self.worker.lock() {
            if worker.as_ref().is_some_and(JoinHandle::is_finished) {
                if let Some(worker) = worker.take() {
                    let _ = worker.join();
                }
            }
        }
    }
}

struct Connection<'a> {
    state: &'a State,
    id: u64,
}
impl Drop for Connection<'_> {
    fn drop(&mut self) {
        if let Ok(mut connections) = self.state.connections.lock() {
            if let Some(sockets) = connections.remove(&self.id) {
                for socket in sockets {
                    let _ = socket.shutdown(Shutdown::Both);
                }
            }
        }
    }
}
fn connect_host(header: &str, state: &State) -> Result<String, String> {
    let mut lines = header.split("\r\n");
    let words: Vec<_> = lines.next().ok_or_else(failure)?.split(' ').collect();
    if words.len() != 3 || words[0] != "CONNECT" || words[2] != "HTTP/1.1" {
        return Err(failure());
    }
    let hostname = words[1].strip_suffix(":443").ok_or_else(failure)?;
    if !state.hosts.contains(&hostname) {
        return Err(failure());
    }
    let mut authorized = false;
    let mut host = false;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (key, value) = line.split_once(':').ok_or_else(failure)?;
        if key.eq_ignore_ascii_case("proxy-authorization") {
            if authorized || value.trim() != state.authorization.as_str() {
                return Err(failure());
            }
            authorized = true;
        } else if key.eq_ignore_ascii_case("host") {
            if host || value.trim() != words[1] {
                return Err(failure());
            }
            host = true;
        } else if !(key.eq_ignore_ascii_case("proxy-connection")
            || key.eq_ignore_ascii_case("user-agent"))
        {
            return Err(failure());
        }
    }
    if !authorized || !host {
        return Err(failure());
    }
    Ok(hostname.into())
}
fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_unspecified()
                && !ip.is_loopback()
                && !ip.is_private()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_multicast()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 192 && b == 0)
                && !(a == 198 && (b == 18 || b == 19))
                && !(a == 198 && b == 51 && c == 100)
                && !(a == 203 && b == 0 && c == 113)
        }
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return public_address(IpAddr::V4(ip));
            }
            let parts = ip.segments();
            // Only ordinary global unicast; exclude documentation, translation,
            // Teredo/6to4 and local networks before opening any socket.
            parts[0] & 0xe000 == 0x2000
                && !(parts[0] == 0x2001 && parts[1] < 0x200)
                && !(parts[0] == 0x2001 && parts[1] == 0xdb8)
                && parts[0] != 0x2002
        }
    }
}
fn tunnel(state: &State, mut client: TcpStream) -> Result<(), String> {
    client
        .set_read_timeout(Some(IO_WAIT))
        .map_err(|_| failure())?;
    client
        .set_write_timeout(Some(IO_WAIT))
        .map_err(|_| failure())?;
    let id = state.sequence.fetch_add(1, Ordering::SeqCst);
    state
        .connections
        .lock()
        .map_err(|_| failure())?
        .insert(id, vec![client.try_clone().map_err(|_| failure())?]);
    let _connection = Connection { state, id };
    let mut header = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !header.ends_with(b"\r\n\r\n") {
        if state.cancelled() || Instant::now() >= deadline || header.len() >= HEADER_LIMIT {
            return Err(failure());
        }
        let mut byte = [0];
        match client.read(&mut byte) {
            Ok(1) => header.push(byte[0]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            _ => return Err(failure()),
        }
    }
    let hostname = match std::str::from_utf8(&header)
        .ok()
        .and_then(|header| connect_host(header, state).ok())
    {
        Some(host) => host,
        None => {
            let _ = client.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
            return Err(failure());
        }
    };
    let _effect = state.enter()?;
    #[cfg(test)]
    let fixture = *state.fixture_upstream.lock().map_err(|_| failure())?;
    #[cfg(not(test))]
    let fixture: Option<SocketAddr> = None;
    let addresses: Vec<SocketAddr> = if let Some(address) = fixture {
        vec![address]
    } else {
        (hostname.as_str(), 443)
            .to_socket_addrs()
            .map_err(|_| failure())?
            .collect()
    };
    if addresses.is_empty()
        || addresses.len() > 32
        || fixture.is_none() && addresses.iter().any(|a| !public_address(a.ip()))
    {
        return Err(failure());
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut remote = None;
    for address in addresses {
        if state.cancelled() || Instant::now() >= deadline {
            return Err(failure());
        }
        if let Ok(socket) = TcpStream::connect_timeout(&address, Duration::from_millis(500)) {
            remote = Some(socket);
            break;
        }
    }
    let mut remote = remote.ok_or_else(failure)?;
    remote
        .set_read_timeout(Some(IO_WAIT))
        .map_err(|_| failure())?;
    remote
        .set_write_timeout(Some(IO_WAIT))
        .map_err(|_| failure())?;
    state
        .connections
        .lock()
        .map_err(|_| failure())?
        .get_mut(&id)
        .ok_or_else(failure)?
        .push(remote.try_clone().map_err(|_| failure())?);
    if state.cancelled() {
        return Err(failure());
    }
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .map_err(|_| failure())?;
    thread::scope(|scope| {
        let mut remote_write = remote.try_clone().map_err(|_| failure())?;
        let mut client_read = client.try_clone().map_err(|_| failure())?;
        let outbound = scope.spawn(move || {
            let result = relay(state, &mut client_read, &mut remote_write);
            if result.is_err() {
                let _ = client_read.shutdown(Shutdown::Both);
                let _ = remote_write.shutdown(Shutdown::Both);
            }
            result
        });
        let inbound = relay(state, &mut remote, &mut client);
        // Failure in either direction closes both. An orderly client write EOF
        // preserves the response half until the upstream completes or Stop.
        let _ = client.shutdown(Shutdown::Both);
        let _ = remote.shutdown(Shutdown::Both);
        let result = outbound.join().map_err(|_| failure())?;
        inbound.and(result)
    })
}
fn relay(state: &State, input: &mut TcpStream, output: &mut TcpStream) -> Result<(), String> {
    let mut buffer = [0u8; 16384];
    while !state.cancelled() {
        match input.read(&mut buffer) {
            Ok(0) => {
                let _ = output.shutdown(Shutdown::Write);
                return Ok(());
            }
            Ok(count) => output.write_all(&buffer[..count]).map_err(|_| failure())?,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return Err(failure()),
        }
    }
    Err(failure())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocked_outbound_write_closes_an_otherwise_idle_response_half() {
        use std::os::fd::AsRawFd;
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let buffer_size: libc::c_int = 1024;
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    upstream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_RCVBUF,
                    (&buffer_size as *const libc::c_int).cast(),
                    std::mem::size_of_val(&buffer_size) as libc::socklen_t,
                )
            },
            0
        );
        let proxy = Proxy::start(TitleCli::Codex).unwrap();
        *proxy.state.fixture_upstream.lock().unwrap() = Some(upstream.local_addr().unwrap());
        proxy
            .attach(super::super::host_boundary::test_effect_scope())
            .unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", proxy.port())).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(client,"CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\nProxy-Authorization: {}\r\n\r\n",proxy.state.authorization.as_str()).unwrap();
        let (_remote, _) = upstream.accept().unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            client.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
        }
        assert!(header.starts_with(b"HTTP/1.1 200"));
        let bytes = [42; 16384];
        let mut failed = false;
        for _ in 0..4096 {
            if client.write_all(&bytes).is_err() {
                failed = true;
                break;
            }
        }
        assert!(
            failed,
            "The fixture must cause actual outbound backpressure."
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while !proxy.state.connections.lock().unwrap().is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            proxy.state.connections.lock().unwrap().is_empty(),
            "The response half and retained tunnel must close without Stop."
        );
    }
    #[test]
    fn cancellation_closes_both_ends_of_an_admitted_owned_tunnel() {
        for address in ["127.0.0.1", "::1"] {
            let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let proxy = Proxy::start(TitleCli::Codex).unwrap();
            *proxy.state.fixture_upstream.lock().unwrap() = Some(upstream.local_addr().unwrap());
            let effects = super::super::host_boundary::test_effect_scope();
            proxy.attach(effects.clone()).unwrap();
            assert!(proxy.attach(effects.clone()).is_err());
            let mut client = TcpStream::connect((address, proxy.port())).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            write!(client,"CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\nProxy-Authorization: {}\r\n\r\n",proxy.state.authorization.as_str()).unwrap();
            let (mut remote, _) = upstream.accept().unwrap();
            remote
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                client.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
            }
            assert!(header.starts_with(b"HTTP/1.1 200"));
            // The transport carries arbitrary TLS bytes unchanged, without decoding.
            let bytes = [0x16, 0x03, 0x01, 0x00, 0x05, 0xde, 0xad, 0xbe, 0xef, 0x00];
            client.write_all(&bytes).unwrap();
            let mut observed = [0; 10];
            remote.read_exact(&mut observed).unwrap();
            assert_eq!(observed, bytes);
            remote.write_all(&bytes).unwrap();
            client.read_exact(&mut observed).unwrap();
            assert_eq!(observed, bytes);
            effects.cancel();
            let mut byte = [0];
            assert_eq!(client.read(&mut byte).unwrap(), 0);
            assert_eq!(remote.read(&mut byte).unwrap(), 0);
            assert!(effects.enter().is_err());
            let deadline = Instant::now() + Duration::from_secs(2);
            while !proxy.state.connections.lock().unwrap().is_empty() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert!(proxy.state.connections.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn transport_rejects_unauthorized_and_ambiguous_connects_before_dns() {
        let proxy = Proxy::start(TitleCli::Codex).unwrap();
        for address in ["127.0.0.1", "::1"] {
            for authority in [
                "127.0.0.1:443",
                "api.openai.com:80",
                "api.openai.com.evil:443",
                "api.openai.com:443@evil",
                "api.openai.com:443",
            ] {
                let mut client = TcpStream::connect((address, proxy.port())).unwrap();
                client
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                write!(client,"CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: Basic wrong\r\n\r\n").unwrap();
                let mut output = String::new();
                client.read_to_string(&mut output).unwrap();
                assert!(output.starts_with("HTTP/1.1 403"));
            }
        }
        let auth = proxy.state.authorization.as_str();
        assert!(connect_host(&format!("CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\nProxy-Authorization: {auth}\r\n\r\n"), &proxy.state).is_ok());
        assert!(connect_host(&format!("CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\nProxy-Authorization: {auth}\r\nProxy-Authorization: {auth}\r\n\r\n"), &proxy.state).is_err());
    }
    #[test]
    fn cancellation_preserves_both_reserved_endpoints_until_owner_release() {
        let proxy = Proxy::start(TitleCli::Codex).unwrap();
        let port = proxy.port();
        let effects = super::super::host_boundary::test_effect_scope();
        proxy.attach(effects.clone()).unwrap();
        effects.cancel();
        let deadline = Instant::now() + Duration::from_secs(2);
        while proxy
            .worker
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|w| !w.is_finished())
        {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        for address in ["127.0.0.1", "::1"] {
            assert_eq!(
                TcpListener::bind((address, port)).unwrap_err().kind(),
                std::io::ErrorKind::AddrInUse
            );
        }
        // Managed launch additionally holds the Proxy in its boundary until a
        // verified receipt. This direct transport fixture releases its owner.
        drop(proxy);
        for address in ["127.0.0.1", "::1"] {
            drop(TcpListener::bind((address, port)).unwrap());
        }
    }
    #[test]
    fn a_shadow_ipv6_listener_prevents_transport_admission() {
        let ipv4 = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = ipv4.local_addr().unwrap().port();
        let _shadow = TcpListener::bind(("::1", port)).unwrap();
        assert!(Proxy::start_with_listener(TitleCli::Codex, ipv4).is_err());
        // Failed admission also releases the incomplete IPv4 reservation.
        drop(TcpListener::bind(("127.0.0.1", port)).unwrap());
    }
    #[test]
    fn endpoint_resolution_refuses_local_and_translated_addresses() {
        for ip in [
            "0.0.0.0",
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "64:ff9b::a00:1",
            "2002:a00:1::1",
            "2001:db8::1",
        ] {
            assert!(!public_address(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_address("8.8.8.8".parse().unwrap()));
        assert!(public_address("2606:4700::1111".parse().unwrap()));
    }
}
