use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, ReadBuf},
    time::{sleep, Sleep},
};

/// Bounds lines before rmcp's read_until allocates or parses them. Framing and
/// negotiation remain the SDK's responsibility. An overrun closes the stream.
pub struct BoundedStdin<R> {
    inner: R,
    bytes: usize,
    limit: usize,
    deadline: Option<Pin<Box<Sleep>>>,
    failed: bool,
}

impl<R> BoundedStdin<R> {
    pub fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            bytes: 0,
            limit,
            deadline: None,
            failed: false,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for BoundedStdin<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if self.failed {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP input closed",
            )));
        }
        if let Some(timer) = self.deadline.as_mut() {
            if timer.as_mut().poll(cx).is_ready() {
                self.failed = true;
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "MCP message deadline exceeded",
                )));
            }
        }
        let mut scratch = [0u8; 8192];
        let length = output.remaining().min(scratch.len());
        let mut buffer = ReadBuf::new(&mut scratch[..length]);
        match Pin::new(&mut self.inner).poll_read(cx, &mut buffer) {
            Poll::Ready(Ok(())) => {
                for &byte in buffer.filled() {
                    if byte == b'\n' {
                        self.bytes = 0;
                        self.deadline = None;
                    } else {
                        self.bytes += 1;
                        if self.bytes > self.limit {
                            self.failed = true;
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "MCP message limit exceeded",
                            )));
                        }
                        if self.deadline.is_none() {
                            self.deadline = Some(Box::pin(sleep(Duration::from_secs(5))));
                        }
                    }
                }
                output.put_slice(buffer.filled());
                Poll::Ready(Ok(()))
            }
            value => value,
        }
    }
}
