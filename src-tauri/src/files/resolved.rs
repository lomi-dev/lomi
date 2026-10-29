use std::{fs, path::Path};

#[cfg(unix)]
pub(crate) fn open_resolved_file(path: &Path) -> std::io::Result<fs::File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
        path::Component,
    };
    if !path.is_absolute() || path.components().count() > 128 {
        return Err(std::io::Error::other(
            "The resolved file path is unavailable.",
        ));
    }
    // Resolve has already checked containment and intentionally followed any
    // user-selected aliases. Reopen that canonical result without following a
    // replacement link at any component, including the final file.
    let mut current = fs::File::open("/")?;
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        let name = match component {
            Component::RootDir => continue,
            Component::Normal(name) => name,
            _ => return Err(std::io::Error::other("The resolved file path changed.")),
        };
        let name = CString::new(name.as_bytes()).map_err(std::io::Error::other)?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if components.peek().is_some() {
                libc::O_DIRECTORY
            } else {
                0
            };
        let fd = unsafe { libc::openat(current.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        current = unsafe { fs::File::from_raw_fd(fd) };
    }
    Ok(current)
}

#[cfg(windows)]
pub(crate) fn open_resolved_file(path: &Path) -> std::io::Result<fs::File> {
    use std::os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        GetFinalPathNameByHandleW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    };
    if !path.is_absolute() {
        return Err(std::io::Error::other(
            "The resolved file path is unavailable.",
        ));
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(std::io::Error::other(
            "The resolved file was replaced by a reparse point.",
        ));
    }
    // The leaf handle cannot follow a replacement reparse point. Check its final
    // name before reading, so a replacement ancestor cannot redirect the read.
    let mut final_name = vec![0u16; 32768];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            final_name.as_mut_ptr(),
            final_name.len() as u32,
            0,
        )
    } as usize;
    if length == 0 {
        return Err(std::io::Error::last_os_error());
    }
    if length >= final_name.len()
        || final_name[..length] != path.as_os_str().encode_wide().collect::<Vec<_>>()
    {
        return Err(std::io::Error::other("The resolved file path changed."));
    }
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn open_resolved_file(_path: &Path) -> std::io::Result<fs::File> {
    Err(std::io::Error::other(
        "Safe file previews are unsupported on this platform.",
    ))
}
