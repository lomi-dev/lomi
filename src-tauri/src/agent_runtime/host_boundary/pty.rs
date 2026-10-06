use super::{failure, native_stdio, open_fifo};
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    path::{Path, PathBuf},
};

pub(super) struct Pty {
    pub master: File,
    pub slave: File,
    pub resize: File,
    pub ack: File,
}
impl Pty {
    pub(super) fn create(root: &Path, rows: u16, cols: u16) -> Result<Self, String> {
        let mut master = -1;
        let mut slave = -1;
        let mut name = [0i8; 128];
        let mut size = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                name.as_mut_ptr(),
                std::ptr::null_mut(),
                &mut size,
            )
        } != 0
        {
            return Err(failure());
        }
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        for file in [&master, &slave] {
            let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
            if flags < 0
                || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, flags | libc::FD_CLOEXEC) }
                    < 0
            {
                return Err(failure());
            }
            native_stdio(file)?;
        }
        let path = PathBuf::from(
            unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }
                .to_str()
                .map_err(|_| failure())?,
        );
        if !path.starts_with("/dev") {
            return Err(failure());
        }
        Ok(Self {
            master,
            slave,
            resize: open_fifo(&root.join("resize"), true, true)?,
            ack: open_fifo(&root.join("resize-ack"), true, true)?,
        })
    }
    pub(super) fn relay(
        self,
        mut stdin: File,
        mut stdout: File,
    ) -> Result<std::thread::JoinHandle<Result<(), String>>, String> {
        let mut input = self.master.try_clone().map_err(|_| failure())?;
        let resize_master = self.master.try_clone().map_err(|_| failure())?;
        let mut resize = self.resize;
        let mut ack = self.ack;
        std::thread::spawn(move || {
            let mut bytes = [0; 4];
            let mut at = 0;
            loop {
                match resize.read(&mut bytes[at..]) {
                    Ok(0) => std::thread::sleep(std::time::Duration::from_millis(10)),
                    Ok(n) => {
                        at += n;
                        if at == 4 {
                            let rows = u16::from_le_bytes([bytes[0], bytes[1]]);
                            let cols = u16::from_le_bytes([bytes[2], bytes[3]]);
                            if rows != 0 && cols != 0 {
                                let size = libc::winsize {
                                    ws_row: rows,
                                    ws_col: cols,
                                    ws_xpixel: 0,
                                    ws_ypixel: 0,
                                };
                                if unsafe {
                                    libc::ioctl(resize_master.as_raw_fd(), libc::TIOCSWINSZ, &size)
                                } < 0
                                {
                                    let _ = super::write_stream(&mut ack, &[0; 4]);
                                    break;
                                }
                                let mut observed = unsafe { std::mem::zeroed::<libc::winsize>() };
                                if unsafe {
                                    libc::ioctl(
                                        resize_master.as_raw_fd(),
                                        libc::TIOCGWINSZ,
                                        &mut observed,
                                    )
                                } < 0
                                    || observed.ws_row != rows
                                    || observed.ws_col != cols
                                {
                                    let _ = super::write_stream(&mut ack, &[0; 4]);
                                    break;
                                }
                                if super::write_stream(&mut ack, &bytes).is_err() {
                                    break;
                                }
                            } else {
                                let _ = super::write_stream(&mut ack, &[0; 4]);
                            }
                            at = 0;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut stdin, &mut input);
            // Deliver the terminal's configured EOF character when input closes.
            let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
            if unsafe { libc::tcgetattr(input.as_raw_fd(), &mut termios) } == 0 {
                let _ = input.write_all(&[termios.c_cc[libc::VEOF]]);
            }
        });
        drop(self.slave);
        let mut master = self.master;
        Ok(std::thread::spawn(move || {
            let mut bytes = [0; 65536];
            loop {
                match master.read(&mut bytes) {
                    Ok(0) => return Ok(()),
                    Ok(n) => stdout.write_all(&bytes[..n]).map_err(|_| failure())?,
                    Err(e) if e.raw_os_error() == Some(libc::EIO) => return Ok(()),
                    Err(_) => return Err(failure()),
                }
            }
        }))
    }
}
