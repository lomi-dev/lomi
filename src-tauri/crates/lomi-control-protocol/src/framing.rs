use std::{io, time::Duration};

use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Reads one length-prefixed IPC frame. The deadline includes the prefix and
/// payload; a rejected/partial frame makes the connection unusable.
pub async fn read_frame<R: AsyncRead + Unpin, T: DeserializeOwned>(
    reader: &mut R,
    limit: usize,
    deadline: Duration,
) -> io::Result<T> {
    tokio::time::timeout(deadline, async {
        let length = reader.read_u32().await? as usize;
        if length == 0 || length > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid frame length",
            ));
        }
        let mut payload = vec![0; length];
        reader.read_exact(&mut payload).await?;
        serde_json::from_slice(&payload)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid frame payload"))
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Frame deadline exceeded"))?
}

struct LimitedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl io::Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Frame limit exceeded",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    payload: &T,
    limit: usize,
    deadline: Duration,
) -> io::Result<()> {
    let mut buffer = LimitedBuffer {
        bytes: Vec::new(),
        limit: limit.min(u32::MAX as usize),
    };
    serde_json::to_writer(&mut buffer, payload)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Frame encoding failed"))?;
    tokio::time::timeout(deadline, async {
        writer.write_u32(buffer.bytes.len() as u32).await?;
        writer.write_all(&buffer.bytes).await?;
        writer.flush().await
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Frame deadline exceeded"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_oversized_prefix_without_waiting_for_payload() {
        let mut input = &u32::MAX.to_be_bytes()[..];
        let error = read_frame::<_, serde_json::Value>(&mut input, 1024, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn fragmented_frame_round_trips_and_rejects_unknown_fields() {
        let (mut writer, mut reader) = tokio::io::duplex(1);
        let task = tokio::spawn(async move {
            write_frame(
                &mut writer,
                &crate::EmptyInput {},
                1024,
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        });
        read_frame::<_, crate::EmptyInput>(&mut reader, 1024, Duration::from_secs(1))
            .await
            .unwrap();
        task.await.unwrap();
        let mut input = &b"\0\0\0\x0b{\"extra\":1}"[..];
        let error = read_frame::<_, crate::EmptyInput>(&mut input, 1024, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_frame_has_a_total_deadline() {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        writer.write_all(&[0, 0]).await.unwrap();
        let error = read_frame::<_, serde_json::Value>(&mut reader, 1024, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn oversized_output_writes_no_partial_frame() {
        let mut output = Vec::new();
        assert!(
            write_frame(&mut output, &"x".repeat(1025), 1024, Duration::from_secs(1))
                .await
                .is_err()
        );
        assert!(output.is_empty());
    }
}
