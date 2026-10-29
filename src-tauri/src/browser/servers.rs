use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    sync::Mutex,
    time::Duration,
};

fn loopback(address: SocketAddr) -> Option<SocketAddr> {
    let ip = match address.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.into(),
        IpAddr::V6(ip) if ip.is_unspecified() => Ipv6Addr::LOCALHOST.into(),
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or(IpAddr::V6(ip), IpAddr::V4),
        ip => ip,
    };
    (ip.is_loopback() && address.port() != 0).then(|| SocketAddr::new(ip, address.port()))
}

#[cfg(target_os = "linux")]
fn proc_listener(line: &str) -> Option<SocketAddr> {
    let mut fields = line.split_whitespace();
    let local = fields.nth(1)?;
    if fields.nth(1)? != "0A" {
        return None;
    }
    let (ip, port) = local.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let ip = match ip.len() {
        8 => IpAddr::V4(Ipv4Addr::from(
            u32::from_str_radix(ip, 16).ok()?.to_ne_bytes(),
        )),
        32 => {
            let mut bytes = [0; 16];
            for (chunk, word) in bytes
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(ip.as_bytes().as_chunks::<8>().0)
            {
                chunk.copy_from_slice(
                    &u32::from_str_radix(std::str::from_utf8(word).ok()?, 16)
                        .ok()?
                        .to_ne_bytes(),
                );
            }
            IpAddr::V6(Ipv6Addr::from(bytes))
        }
        _ => return None,
    };
    loopback(SocketAddr::new(ip, port))
}

#[cfg(target_os = "linux")]
fn listeners() -> Result<BTreeSet<SocketAddr>, String> {
    let mut addresses = BTreeSet::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Cannot read local listeners: {error}")),
        };
        addresses.extend(text.lines().filter_map(proc_listener));
    }
    Ok(addresses)
}

#[cfg(target_os = "macos")]
fn lsof_listeners(text: &str) -> BTreeSet<SocketAddr> {
    text.lines()
        .filter_map(|line| line.strip_prefix('n'))
        .flat_map(|address| {
            if let Some(port) = address
                .strip_prefix("*:")
                .and_then(|port| port.parse().ok())
            {
                vec![
                    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port),
                    SocketAddr::new(Ipv6Addr::LOCALHOST.into(), port),
                ]
            } else {
                address
                    .parse()
                    .ok()
                    .and_then(loopback)
                    .into_iter()
                    .collect()
            }
        })
        .collect()
}

#[cfg(windows)]
fn netstat_listeners(text: &str) -> BTreeSet<SocketAddr> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            if fields.next()? != "TCP" {
                return None;
            }
            let local = fields.next()?;
            if fields.nth(1)? != "LISTENING" {
                return None;
            }
            local.parse().ok().and_then(loopback)
        })
        .collect()
}

#[cfg(any(target_os = "macos", windows))]
fn listeners() -> Result<BTreeSet<SocketAddr>, String> {
    #[cfg(target_os = "macos")]
    let output = crate::shell::quiet_command("/usr/sbin/lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fn"])
        .output();
    #[cfg(windows)]
    let output = crate::shell::quiet_command("netstat.exe")
        .args(["-an"])
        .output();
    let output = output.map_err(|error| format!("Cannot read local listeners: {error}"))?;
    // lsof exits with 1 when no sockets match its filter.
    #[cfg(target_os = "macos")]
    if output.status.code() == Some(1) && output.stderr.is_empty() {
        return Ok(BTreeSet::new());
    }
    if !output.status.success() {
        return Err("Cannot read local listeners.".into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    #[cfg(target_os = "macos")]
    return Ok(lsof_listeners(&text));
    #[cfg(windows)]
    return Ok(netstat_listeners(&text));
}

fn server_url(address: SocketAddr) -> String {
    let host = match address.ip() {
        IpAddr::V4(ip) if ip == Ipv4Addr::LOCALHOST => "localhost".into(),
        IpAddr::V6(ip) if ip == Ipv6Addr::LOCALHOST => "localhost".into(),
        ip => ip.to_string(),
    };
    format!("http://{host}:{}", address.port())
}

fn responds_to_http(address: SocketAddr) -> std::io::Result<bool> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(200))?;
    stream.set_read_timeout(Some(Duration::from_millis(1200)))?;
    stream.set_write_timeout(Some(Duration::from_millis(200)))?;
    // OPTIONS checks HTTP support without fetching a page body. Next.js rejects the * target.
    // Discovery probes plain HTTP only; no TLS handshake is performed.
    write!(
        stream,
        "OPTIONS / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        server_url(address).trim_start_matches("http://")
    )?;
    let mut prefix = [0; 12];
    stream.read_exact(&mut prefix)?;
    Ok(
        (prefix.starts_with(b"HTTP/1.0 ") || prefix.starts_with(b"HTTP/1.1 "))
            && prefix[9..].iter().all(u8::is_ascii_digit),
    )
}

fn discover(addresses: BTreeSet<SocketAddr>, on_found: impl Fn(String) + Sync) -> Vec<String> {
    let queue = Mutex::new(addresses.iter());
    let found = Mutex::new(BTreeSet::new());
    std::thread::scope(|scope| {
        for _ in 0..addresses.len().min(8) {
            scope.spawn(|| loop {
                let Some(&address) = queue.lock().unwrap().next() else {
                    break;
                };
                if responds_to_http(address).unwrap_or(false) {
                    let url = server_url(address);
                    let inserted = found.lock().unwrap().insert((address.port(), url.clone()));
                    if inserted {
                        on_found(url);
                    }
                }
            });
        }
    });
    found
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|(_, url)| url)
        .collect()
}

#[tauri::command]
pub async fn local_web_servers(
    window: tauri::Window,
    on_found: tauri::ipc::Channel<String>,
) -> Result<Vec<String>, String> {
    crate::files::main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        Ok(discover(listeners()?, |url| {
            let _ = on_found.send(url);
        }))
    })
    .await
    .map_err(|error| error.to_string())?
}
