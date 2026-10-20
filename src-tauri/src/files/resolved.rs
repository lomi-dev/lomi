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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        ffi::CString,
        io::Read,
        os::unix::{ffi::OsStrExt, fs::symlink},
    };

    fn previews_reject(path: &Path) {
        assert!(super::super::preview_resolved(path).is_err());
        assert!(super::super::images::read_resolved(path).is_err());
        assert!(super::super::markdown::read_resolved(path).is_err());
    }

    #[test]
    fn resolved_paths_reject_replacement_leaves_ancestors_and_fifos() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        fs::write(root.join("preview.png"), "approved").unwrap();
        fs::write(outside.path().join("preview.png"), "private").unwrap();
        let resolved = super::super::inside(root.to_str().unwrap(), "preview.png").unwrap();
        fs::remove_file(&resolved).unwrap();
        symlink(outside.path().join("preview.png"), &resolved).unwrap();
        assert!(open_resolved_file(&resolved).is_err());
        previews_reject(&resolved);
        fs::remove_file(&resolved).unwrap();
        let fifo = CString::new(resolved.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        // O_NONBLOCK means this returns without waiting for a FIFO writer.
        let opened = open_resolved_file(&resolved).unwrap();
        assert!(!opened.metadata().unwrap().is_file());
        previews_reject(&resolved);
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub/preview.png"), "approved").unwrap();
        let resolved = super::super::inside(root.to_str().unwrap(), "sub/preview.png").unwrap();
        fs::rename(root.join("sub"), root.join("old-sub")).unwrap();
        symlink(outside.path(), root.join("sub")).unwrap();
        assert!(open_resolved_file(&resolved).is_err());
        previews_reject(&resolved);
    }

    #[test]
    fn internal_aliases_are_resolved_before_the_guarded_open() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("preview.svg"), "approved").unwrap();
        symlink("preview.svg", directory.path().join("alias.svg")).unwrap();
        let resolved =
            super::super::inside(directory.path().to_str().unwrap(), "alias.svg").unwrap();
        let mut bytes = String::new();
        open_resolved_file(&resolved)
            .unwrap()
            .read_to_string(&mut bytes)
            .unwrap();
        assert_eq!(bytes, "approved");
        assert_eq!(
            super::super::preview_resolved(&resolved).unwrap(),
            "approved"
        );
        assert_eq!(
            super::super::images::read_resolved(&resolved).unwrap(),
            b"approved"
        );
        assert_eq!(
            super::super::markdown::read_resolved(&resolved).unwrap(),
            b"approved"
        );
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::os::windows::fs::{symlink_dir, symlink_file};

    fn previews_reject(path: &Path) {
        assert!(super::super::preview_resolved(path).is_err());
        assert!(super::super::images::read_resolved(path).is_err());
        assert!(super::super::markdown::read_resolved(path).is_err());
    }

    fn symlink_or_skip(result: std::io::Result<()>) -> bool {
        match result {
            Ok(()) => true,
            Err(error) if error.raw_os_error() == Some(1314) => {
                eprintln!("Skipping Windows symlink fixture: Developer Mode or symlink privilege is required (1314).");
                false
            }
            Err(error) => panic!("Could not create the Windows symlink fixture: {error}"),
        }
    }

    #[test]
    fn opened_handle_rejects_leaf_and_ancestor_reparse_replacements() {
        // Windows native qualification requires Developer Mode or symlink privilege.
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        fs::write(root.join("preview.png"), "approved").unwrap();
        fs::write(outside.path().join("preview.png"), "private").unwrap();
        let resolved = super::super::inside(root.to_str().unwrap(), "preview.png").unwrap();
        assert!(open_resolved_file(&resolved)
            .unwrap()
            .metadata()
            .unwrap()
            .is_file());
        fs::remove_file(&resolved).unwrap();
        if !symlink_or_skip(symlink_file(outside.path().join("preview.png"), &resolved)) {
            return;
        }
        previews_reject(&resolved);
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub/preview.png"), "approved").unwrap();
        let resolved = super::super::inside(root.to_str().unwrap(), "sub/preview.png").unwrap();
        fs::rename(root.join("sub"), root.join("old-sub")).unwrap();
        if !symlink_or_skip(symlink_dir(outside.path(), root.join("sub"))) {
            return;
        }
        previews_reject(&resolved);
    }

    #[test]
    fn internal_windows_aliases_remain_readable() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("preview.svg"), "approved").unwrap();
        if !symlink_or_skip(symlink_file(
            "preview.svg",
            directory.path().join("alias.svg"),
        )) {
            return;
        }
        let resolved =
            super::super::inside(directory.path().to_str().unwrap(), "alias.svg").unwrap();
        assert_eq!(
            super::super::preview_resolved(&resolved).unwrap(),
            "approved"
        );
        assert_eq!(
            super::super::images::read_resolved(&resolved).unwrap(),
            b"approved"
        );
        assert_eq!(
            super::super::markdown::read_resolved(&resolved).unwrap(),
            b"approved"
        );
    }
}
