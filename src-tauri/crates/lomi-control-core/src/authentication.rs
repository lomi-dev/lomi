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
