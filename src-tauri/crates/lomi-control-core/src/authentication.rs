//! Session-only TLS identities. Trust anchors must come from an approved pairing,
//! never from the connecting peer, its label, or its endpoint descriptor.
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    ClientConfig, RootCertStore, ServerConfig,
};
use std::{io, sync::Arc};

pub struct Identity {
    certificate: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
}

fn failure() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "IPC identity validation failed",
    )
}

impl Identity {
    pub fn ephemeral(instance_dns_name: &str) -> io::Result<Self> {
        if !instance_dns_name.ends_with(".lomi.invalid")
            || instance_dns_name.len() > 128
            || !instance_dns_name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
        {
            return Err(failure());
        }
        let generated = rcgen::generate_simple_self_signed(vec![instance_dns_name.into()])
            .map_err(|_| failure())?;
        Ok(Self {
            certificate: generated.cert.der().clone(),
            key: PrivatePkcs8KeyDer::from(generated.signing_key.serialize_der()).into(),
        })
    }

    pub fn certificate(&self) -> CertificateDer<'static> {
        self.certificate.clone()
    }

    /// Session grants decide which peer certificate may reach this listener.
    /// TLS 1.3 authenticates both possession proofs against fresh transcripts.
    pub fn server(&self, approved_peer: CertificateDer<'static>) -> io::Result<Arc<ServerConfig>> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = RootCertStore::empty();
        roots.add(approved_peer).map_err(|_| failure())?;
        let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
            Arc::new(roots),
            provider.clone(),
        )
        .build()
        .map_err(|_| failure())?;
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| failure())?
            .with_client_cert_verifier(verifier)
            .with_single_cert(vec![self.certificate()], self.key.clone_key())
            .map_err(|_| failure())?;
        config.alpn_protocols = vec![b"lomi-control/1".to_vec()];
        config.max_early_data_size = 0;
        config.send_tls13_tickets = 0;
        config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
        Ok(Arc::new(config))
    }

    pub fn client(
        &self,
        approved_broker: CertificateDer<'static>,
    ) -> io::Result<Arc<ClientConfig>> {
        let mut roots = RootCertStore::empty();
        roots.add(approved_broker).map_err(|_| failure())?;
        let mut config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .map_err(|_| failure())?
                .with_root_certificates(roots)
                .with_client_auth_cert(vec![self.certificate()], self.key.clone_key())
                .map_err(|_| failure())?;
        config.alpn_protocols = vec![b"lomi-control/1".to_vec()];
        config.enable_early_data = false;
        config.resumption = rustls::client::Resumption::disabled();
        Ok(Arc::new(config))
    }
}

#[cfg(unix)]
pub fn require_same_user(stream: &tokio::net::UnixStream) -> io::Result<()> {
    if stream.peer_cred()?.uid() == unsafe { libc::geteuid() } {
        Ok(())
    } else {
        Err(failure())
    }
}

#[cfg(unix)]
pub fn peer_pid(stream: &tokio::net::UnixStream) -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        let mut pid: libc::pid_t = 0;
        let mut size = std::mem::size_of_val(&pid) as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                0,
                libc::LOCAL_PEERPID,
                (&mut pid as *mut libc::pid_t).cast(),
                &mut size,
            )
        };
        (result == 0 && pid > 0 && size as usize == std::mem::size_of_val(&pid))
            .then_some(pid as u32)
    }
    #[cfg(not(target_os = "macos"))]
    {
        stream
            .peer_cred()
            .ok()?
            .pid()
            .and_then(|pid| u32::try_from(pid).ok())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use lomi_control_protocol::{
        framing::{read_frame, write_frame},
        EmptyInput, MAX_HANDSHAKE_BYTES,
    };
    use std::{os::unix::fs::PermissionsExt, time::Duration};
    use tokio::net::{UnixListener, UnixStream};

    async fn exchange(
        client: Arc<ClientConfig>,
        server: Arc<ServerConfig>,
        name: &str,
    ) -> (bool, bool) {
        let root = tempfile::Builder::new()
            .prefix("lmcp-")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.path().join("c.sock");
        let listener = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            require_same_user(&stream).unwrap();
            let Ok(Ok(mut tls)) = tokio::time::timeout(
                Duration::from_secs(2),
                tokio_rustls::TlsAcceptor::from(server).accept(stream),
            )
            .await
            else {
                return false;
            };
            let Ok(_) =
                read_frame::<_, EmptyInput>(&mut tls, MAX_HANDSHAKE_BYTES, Duration::from_secs(2))
                    .await
            else {
                return false;
            };
            write_frame(
                &mut tls,
                &EmptyInput {},
                MAX_HANDSHAKE_BYTES,
                Duration::from_secs(2),
            )
            .await
            .is_ok()
        });
        let stream = UnixStream::connect(path).await.unwrap();
        require_same_user(&stream).unwrap();
        let connect = tokio_rustls::TlsConnector::from(client)
            .connect(name.to_owned().try_into().unwrap(), stream);
        let result = match tokio::time::timeout(Duration::from_secs(2), connect).await {
            Ok(Ok(mut tls)) => {
                let sent = write_frame(
                    &mut tls,
                    &EmptyInput {},
                    MAX_HANDSHAKE_BYTES,
                    Duration::from_secs(2),
                )
                .await
                .is_ok();
                sent && read_frame::<_, EmptyInput>(
                    &mut tls,
                    MAX_HANDSHAKE_BYTES,
                    Duration::from_secs(2),
                )
                .await
                .is_ok()
            }
            _ => false,
        };
        (result, server.await.unwrap())
    }

    #[tokio::test]
    async fn unix_socket_mutually_authenticates_pinned_session_identities() {
        let broker = Identity::ephemeral("instance-one.lomi.invalid").unwrap();
        let helper = Identity::ephemeral("helper.lomi.invalid").unwrap();
        assert_eq!(
            exchange(
                helper.client(broker.certificate()).unwrap(),
                broker.server(helper.certificate()).unwrap(),
                "instance-one.lomi.invalid"
            )
            .await,
            (true, true)
        );
    }

    #[tokio::test]
    async fn fake_broker_and_unapproved_helper_cannot_exchange_frames() {
        let broker = Identity::ephemeral("instance-one.lomi.invalid").unwrap();
        let helper = Identity::ephemeral("helper.lomi.invalid").unwrap();
        let impostor = Identity::ephemeral("instance-one.lomi.invalid").unwrap();
        assert_eq!(
            exchange(
                helper.client(broker.certificate()).unwrap(),
                impostor.server(helper.certificate()).unwrap(),
                "instance-one.lomi.invalid"
            )
            .await,
            (false, false)
        );
        assert_eq!(
            exchange(
                impostor.client(broker.certificate()).unwrap(),
                broker.server(helper.certificate()).unwrap(),
                "instance-one.lomi.invalid"
            )
            .await,
            (false, false)
        );
        assert_eq!(
            exchange(
                helper.client(broker.certificate()).unwrap(),
                broker.server(helper.certificate()).unwrap(),
                "instance-two.lomi.invalid"
            )
            .await,
            (false, false)
        );
    }

    #[test]
    fn a_recorded_handshake_cannot_authenticate_again() {
        let broker = Identity::ephemeral("instance-one.lomi.invalid").unwrap();
        let helper = Identity::ephemeral("helper.lomi.invalid").unwrap();
        let server_config = broker.server(helper.certificate()).unwrap();
        let mut client = rustls::ClientConnection::new(
            helper.client(broker.certificate()).unwrap(),
            "instance-one.lomi.invalid".try_into().unwrap(),
        )
        .unwrap();
        let mut server = rustls::ServerConnection::new(server_config.clone()).unwrap();
        let mut recording = Vec::new();
        for _ in 0..10 {
            let mut to_server = Vec::new();
            client.write_tls(&mut to_server).unwrap();
            recording.extend_from_slice(&to_server);
            server.read_tls(&mut &to_server[..]).unwrap();
            server.process_new_packets().unwrap();
            let mut to_client = Vec::new();
            server.write_tls(&mut to_client).unwrap();
            client.read_tls(&mut &to_client[..]).unwrap();
            client.process_new_packets().unwrap();
            if !client.is_handshaking() && !server.is_handshaking() {
                break;
            }
        }
        assert!(!server.is_handshaking());
        let mut replacement = rustls::ServerConnection::new(server_config).unwrap();
        replacement.read_tls(&mut &recording[..]).unwrap();
        assert!(replacement.process_new_packets().is_err());
        assert!(replacement.is_handshaking());
    }
}
