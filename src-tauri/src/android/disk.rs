use std::{collections::BTreeMap, fs, path::Path};

#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Size {
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub free_bytes: u64,
    pub growth_reserve_bytes: u64,
    pub directories: BTreeMap<&'static str, Size>,
}

pub fn usage(root: &Path) -> Result<Usage, String> {
    let mut directories = BTreeMap::new();
    for name in [
        "sdk",
        "avd",
        "toolchains",
        "rollback",
        "staging",
        "cache",
        "logs",
        "user",
        "runtime",
        "tmp",
    ] {
        directories.insert(name, measure(&root.join(name))?);
    }
    Ok(Usage {
        free_bytes: available(root)?,
        growth_reserve_bytes: growth_reserve(root)?,
        directories,
    })
}

pub fn measure(path: &Path) -> Result<Size, String> {
    let mut size = Size::default();
    let mut pending = vec![path.to_owned()];
    let mut entries = 0;
    #[cfg(unix)]
    let mut identities = std::collections::HashSet::new();
    while let Some(path) = pending.pop() {
        entries += 1;
        if entries > 200000 {
            return Err("Android disk inspection exceeded its entry limit. Inspect the managed directory before retrying.".into());
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Cannot measure Android disk usage: {error}")),
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path).map_err(|e| e.to_string())? {
                if pending.len() >= 200000 {
                    return Err("Android disk inspection exceeded its directory limit".into());
                }
                pending.push(entry.map_err(|e| e.to_string())?.path());
            }
        } else if metadata.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if !identities.insert((metadata.dev(), metadata.ino())) {
                    continue;
                }
            }
            size.logical_bytes = size
                .logical_bytes
                .checked_add(metadata.len())
                .ok_or("Android logical size overflow")?;
            size.allocated_bytes = size
                .allocated_bytes
                .checked_add(allocated(&path, &metadata)?)
                .ok_or("Android allocated size overflow")?;
        }
    }
    Ok(size)
}

fn growth_reserve(root: &Path) -> Result<u64, String> {
    let devices: super::storage::Devices =
        super::storage::read(&root.join("devices.json"))?.unwrap_or_default();
    devices.validate()?;
    let mut reserve = 0u64;
    for device in devices.devices {
        let avd = super::installation::checked_path(
            root,
            &Path::new("avd").join(format!("sb_{}.avd", device.id)),
        )?;
        // QCOW's container length is not the virtual data partition capacity.
        // Reserve unwritten guest data, preserving space for every configured AVD.
        let qcow = avd.join("userdata-qemu.img.qcow2");
        let data = if qcow.exists() {
            qcow
        } else {
            avd.join("userdata-qemu.img")
        };
        let allocated = measure(&data)?.allocated_bytes;
        let capacity = u64::from(device.hardware.data_gib) * 1024 * 1024 * 1024;
        let snapshot = if device.hardware.quick_boot {
            u64::from(device.hardware.ram_mib) * 1024 * 1024
        } else {
            0
        };
        reserve = reserve
            .checked_add(capacity.saturating_sub(allocated))
            .and_then(|value| value.checked_add(snapshot))
            .ok_or("Android growth estimate overflow")?;
    }
    Ok(reserve)
}

#[cfg(unix)]
fn allocated(_path: &Path, metadata: &fs::Metadata) -> Result<u64, String> {
    use std::os::unix::fs::MetadataExt;
    metadata
        .blocks()
        .checked_mul(512)
        .ok_or_else(|| "Android allocated size overflow".into())
}

#[cfg(windows)]
fn allocated(path: &Path, _metadata: &fs::Metadata) -> Result<u64, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileStandardInfo, GetFileInformationByHandleEx, FILE_STANDARD_INFO,
    };
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut information = std::mem::MaybeUninit::<FILE_STANDARD_INFO>::uninit();
    // AllocationSize reports allocated clusters rather than a sparse file's EOF.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileStandardInfo,
            information.as_mut_ptr().cast(),
            std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
        )
    } == 0
    {
        return Err(format!(
            "Cannot measure allocated Android storage: {}",
            std::io::Error::last_os_error()
        ));
    }
    u64::try_from(unsafe { information.assume_init() }.AllocationSize)
        .map_err(|_| "Invalid allocated Android file size".into())
}

/// Existing SDK/AVD allocations already reduce available bytes. Reserve additional
/// archive and expanded bytes without assuming sparse AVD files are fully allocated.
pub fn require(path: &Path, additional: u64) -> Result<(), String> {
    let available = available(path)?;
    let required = additional
        .checked_add(growth_reserve(path)?)
        .ok_or("Android growth estimate overflow")?
        .checked_add(512 * 1024 * 1024)
        .ok_or("Android disk estimate overflow")?;
    if available < required {
        return Err(format!("Android needs at least {:.1} GiB of additional free space; {:.1} GiB is available. Free disk space and retry.", required as f64 / 1073741824.0, available as f64 / 1073741824.0));
    }
    Ok(())
}

#[cfg(unix)]
fn available(path: &Path) -> Result<u64, String> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut status = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // statvfs writes the complete structure only on success; path is NUL terminated.
    if unsafe { libc::statvfs(path.as_ptr(), status.as_mut_ptr()) } != 0 {
        return Err(format!(
            "Cannot check Android disk space: {}",
            std::io::Error::last_os_error()
        ));
    }
    let status = unsafe { status.assume_init() };
    u64::try_from(u128::from(status.f_bavail) * u128::from(status.f_frsize))
        .map_err(|_| "Android disk size overflow".into())
}

#[cfg(windows)]
fn available(path: &Path) -> Result<u64, String> {
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0;
    // The API accepts optional null total/free outputs and writes the caller's quota.
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(format!(
            "Cannot check Android disk space: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(available)
}
