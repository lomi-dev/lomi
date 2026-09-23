//! Bounded I/O for explicitly managed Unix PTYs. Setting O_NONBLOCK affects
//! duplicated descriptors too: the single native reader must handle WouldBlock.
use lomi_control_protocol::ErrorCode;
use std::{
    io,
    os::fd::{AsRawFd, BorrowedFd},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct WriteFailure {
    pub code: ErrorCode,
    pub written: usize,
}
pub fn make_nonblocking(fd: BorrowedFd<'_>) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn poll(fd: BorrowedFd<'_>, events: i16, timeout: Duration) -> io::Result<bool> {
    let mut descriptor = libc::pollfd {
        fd: fd.as_raw_fd(),
        events,
        revents: 0,
    };
    let result = unsafe {
        libc::poll(
            &mut descriptor,
            1,
            timeout.as_millis().min(i32::MAX as u128) as i32,
        )
    };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(error);
    }
    if descriptor.revents & libc::POLLNVAL != 0 {
        return Err(io::Error::other("Terminal descriptor closed"));
    }
    Ok(result > 0)
}
pub fn wait_readable(fd: BorrowedFd<'_>) -> io::Result<bool> {
    poll(fd, libc::POLLIN, Duration::from_secs(1))
}

pub fn write(
    fd: BorrowedFd<'_>,
    bytes: &[u8],
    budget: Duration,
    allowed: impl Fn() -> bool,
) -> Result<usize, WriteFailure> {
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 || flags & libc::O_NONBLOCK == 0 {
        return Err(WriteFailure {
            code: ErrorCode::UnsupportedCapability,
            written: 0,
        });
    }
    if bytes.len() > 256 * 1024 || budget > Duration::from_secs(5) || budget.is_zero() {
        return Err(WriteFailure {
            code: ErrorCode::ResourceExhausted,
            written: 0,
        });
    }
    let deadline = Instant::now() + budget;
    let mut written = 0;
    while written < bytes.len() {
        if !allowed() {
            return Err(WriteFailure {
                code: ErrorCode::ControlRevoked,
                written,
            });
        }
        if Instant::now() >= deadline {
            return Err(WriteFailure {
                code: ErrorCode::DeadlineExceeded,
                written,
            });
        }
        let end = (written + 4096).min(bytes.len());
        let count = unsafe {
            libc::write(
                fd.as_raw_fd(),
                bytes[written..end].as_ptr().cast(),
                end - written,
            )
        };
        if count > 0 {
            written += count as usize;
            continue;
        }
        if count == 0 {
            return Err(WriteFailure {
                code: ErrorCode::OutcomeUnknown,
                written,
            });
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if error.kind() != io::ErrorKind::WouldBlock {
            return Err(WriteFailure {
                code: ErrorCode::OutcomeUnknown,
                written,
            });
        }
        if poll(
            fd,
            libc::POLLOUT,
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(25)),
        )
        .is_err()
        {
            return Err(WriteFailure {
                code: ErrorCode::OutcomeUnknown,
                written,
            });
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::{fd::AsFd, unix::net::UnixStream},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };
    #[test]
    fn blocked_native_writer_obeys_deadline_and_manual_revoke() {
        let (sender, _receiver) = UnixStream::pair().unwrap();
        make_nonblocking(sender.as_fd()).unwrap();
        let size: libc::c_int = 4096;
        unsafe {
            assert_eq!(
                libc::setsockopt(
                    sender.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    (&size as *const libc::c_int).cast(),
                    std::mem::size_of_val(&size) as libc::socklen_t
                ),
                0
            );
        }
        let start = Instant::now();
        let error = write(
            sender.as_fd(),
            &vec![b'x'; 256 * 1024],
            Duration::from_millis(80),
            || true,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::DeadlineExceeded);
        assert!(error.written > 0);
        assert!(start.elapsed() < Duration::from_secs(1));
        let allowed = Arc::new(AtomicBool::new(true));
        let revoke = allowed.clone();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            revoke.store(false, Ordering::SeqCst);
        });
        let start = Instant::now();
        let error = write(
            sender.as_fd(),
            b"must not replay",
            Duration::from_secs(2),
            || allowed.load(Ordering::SeqCst),
        )
        .unwrap_err();
        worker.join().unwrap();
        assert_eq!(error.code, ErrorCode::ControlRevoked);
        assert_eq!(error.written, 0);
        assert!(start.elapsed() < Duration::from_millis(500));
    }
}
