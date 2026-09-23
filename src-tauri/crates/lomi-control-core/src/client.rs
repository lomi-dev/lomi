use crate::{
    authentication::{require_same_user, Identity},
    broker::{certificate_hash, new_id, Endpoint, HandshakeIo},
};
use lomi_control_protocol::{
    control::*,
    framing::{read_frame, write_frame},
    IPC_VERSION, MAX_FRAME_BYTES,
};
use std::{
    io,
    os::unix::fs::{FileTypeExt, MetadataExt},
    time::Duration,
};
use tokio::{net::UnixStream, sync::Mutex};

type Connection = tokio_rustls::client::TlsStream<HandshakeIo<UnixStream>>;
pub struct Client {
    stream: Mutex<Option<Connection>>,
}
fn failure() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Cannot authenticate the selected Lomi instance",
    )
}

impl Client {
    /// The pin is supplied by the user-approved configuration, never inferred
    /// from a runtime descriptor or from the server's untrusted public welcome.
    pub async fn connect(
        endpoint: &Endpoint,
        label: &str,
        pending: impl FnOnce(String),
    ) -> io::Result<Self> {
        if endpoint.ipc_version != IPC_VERSION
            || !valid_id(&endpoint.instance_id)
            || endpoint.broker_sha256.len() != 64
            || !endpoint
                .broker_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err(failure());
        }
        let parent = endpoint
            .endpoint
            .parent()
            .ok_or_else(failure)?
            .symlink_metadata()?;
        let socket = endpoint.endpoint.symlink_metadata()?;
        let uid = unsafe { libc::geteuid() };
        if !parent.is_dir()
            || parent.uid() != uid
            || parent.mode() & 0o777 != 0o700
            || !socket.file_type().is_socket()
            || socket.uid() != uid
            || socket.mode() & 0o777 != 0o600
        {
            return Err(failure());
        }
        let identity = Identity::ephemeral("helper.lomi.invalid")?;
        let mut stream = tokio::time::timeout(
            Duration::from_secs(5),
            UnixStream::connect(&endpoint.endpoint),
        )
        .await
        .map_err(|_| failure())??;
        require_same_user(&stream)?;
        let hello = Enrollment {
            ipc_version: IPC_VERSION,
            instance_id: endpoint.instance_id.clone(),
            client_label: label.into(),
            certificate: identity.certificate().as_ref().to_vec(),
        };
        write_frame(&mut stream, &hello, 16 * 1024, Duration::from_secs(5)).await?;
        let welcome: Welcome = read_frame(&mut stream, 16 * 1024, Duration::from_secs(10)).await?;
        if welcome.ipc_version != IPC_VERSION
            || welcome.instance_id != endpoint.instance_id
            || certificate_hash(&welcome.certificate) != endpoint.broker_sha256
            || !valid_id(&welcome.pairing_request_id)
        {
            return Err(failure());
        }
        pending(welcome.pairing_request_id);
        let config = identity.client(welcome.certificate.into())?;
        let stream = HandshakeIo {
            inner: stream,
            remaining: 64 * 1024,
        };
        let name = format!("{}.lomi.invalid", endpoint.instance_id)
            .try_into()
            .map_err(|_| failure())?;
        let mut stream = tokio::time::timeout(
            Duration::from_secs(130),
            tokio_rustls::TlsConnector::from(config).connect(name, stream),
        )
        .await
        .map_err(|_| failure())??;
        stream.get_mut().0.remaining = usize::MAX;
        Ok(Self {
            stream: Mutex::new(Some(stream)),
        })
    }

    pub async fn call(&self, request: Request) -> io::Result<Reply> {
        tokio::time::timeout(Duration::from_secs(20), async {
            let mut guard = self.stream.lock().await;
            // Cancellation discards a possibly partial exchange. A later call
            // cannot consume a previous request's response or replay its action.
            let mut stream = guard.take().ok_or_else(failure)?;
            let id = new_id()?;
            write_frame(
                &mut stream,
                &Message {
                    ipc_version: IPC_VERSION,
                    request_id: id.clone(),
                    request,
                },
                MAX_FRAME_BYTES,
                Duration::from_secs(5),
            )
            .await?;
            let response: Response =
                read_frame(&mut stream, MAX_FRAME_BYTES, Duration::from_secs(20)).await?;
            if response.request_id != id {
                return Err(failure());
            }
            *guard = Some(stream);
            Ok(response.result)
        })
        .await
        .map_err(|_| failure())?
    }
}
