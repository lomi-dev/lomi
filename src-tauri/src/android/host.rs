use super::catalog::Host;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Acceleration {
    pub available: bool,
    pub backend: &'static str,
    pub action: Option<String>,
}

pub fn acceleration() -> Result<Acceleration, String> {
    Host::native()?;
    let (backend, result) = native_acceleration();
    Ok(Acceleration {
        available: result.is_ok(),
        backend,
        action: result.err(),
    })
}

pub fn require_acceleration() -> Result<(), String> {
    let state = acceleration()?;
    if state.available {
        Ok(())
    } else {
        Err(state
            .action
            .unwrap_or("Prepare host virtualization first".into()))
    }
}

pub fn require_qualification() -> Result<(), String> {
    qualification(Host::native()?)
}

/// Qualification comes from the pinned distribution manifest.
pub(super) fn qualified(host: Host) -> Result<bool, String> {
    Ok(super::bootstrap::distribution(host)?.qualified)
}

fn qualification(host: Host) -> Result<(), String> {
    if qualified(host)? {
        Ok(())
    } else {
        Err(format!(
            "Android setup and Start are unavailable on {} in this build. Native process, image and input verification is required before enabling this host. Use a Lomi build qualified for this OS and CPU.",
            super::bootstrap::host_key(host)
        ))
    }
}

#[cfg(target_os = "macos")]
fn native_acceleration() -> (&'static str, Result<(), String>) {
    let mut available: i32 = 0;
    let mut size = std::mem::size_of_val(&available);
    // sysctl is a read-only host capability check and does not start a VM.
    let status = unsafe {
        libc::sysctlbyname(
            c"kern.hv_support".as_ptr(),
            (&mut available as *mut i32).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (
        "Hypervisor.framework",
        if status == 0 && available == 1 {
            Ok(())
        } else {
            Err("This Mac does not expose Hypervisor.framework virtualization. Use a supported physical Mac or enable nested virtualization in the host VM, then retry.".into())
        },
    )
}

#[cfg(target_os = "linux")]
fn native_acceleration() -> (&'static str, Result<(), String>) {
    use std::os::fd::AsRawFd;
    let result = std::fs::OpenOptions::new().read(true).write(true).open("/dev/kvm")
        .map_err(|error| format!("KVM is unavailable: {error}. Enable hardware virtualization in firmware, load KVM and ask an administrator to grant access to /dev/kvm; sign in again and retry."))
        .and_then(|file| {
            // KVM_GET_API_VERSION is a read-only ioctl from linux/kvm.h.
            let version = unsafe { libc::ioctl(file.as_raw_fd(), 0xAE00) };
            if version == 12 { Ok(()) } else { Err("The host exposes an unsupported KVM API. Update the host kernel/KVM and retry.".into()) }
        });
    ("KVM", result)
}

#[cfg(windows)]
#[repr(C, align(8))]
#[derive(Default)]
struct HypervisorCapability {
    present: i32,
    reserved: u32,
}

#[cfg(windows)]
fn interpret_hypervisor_capability(status: i32, written: u32, present: i32) -> Result<(), String> {
    if status < 0 {
        return Err(format!(
            "Windows Hypervisor Platform capability query failed (WHvGetCapability HRESULT=0x{:08X}). Update or repair Windows Hypervisor Platform, then retry.",
            status as u32
        ));
    }
    if written != std::mem::size_of::<i32>() as u32 {
        return Err(format!(
            "Windows Hypervisor Platform returned an unexpected capability size (WHvGetCapability HRESULT=0x{:08X}, written={written}; expected 4 bytes). Update or repair Windows Hypervisor Platform, then retry.",
            status as u32
        ));
    }
    if present == 0 {
        return Err("Windows Hypervisor Platform reports no running hypervisor (WHvGetCapability HypervisorPresent=false). Enable CPU virtualization in UEFI/BIOS and Windows Hypervisor Platform in Windows Features, then restart Windows. In a host VM, enable nested virtualization. These changes require an administrator; Lomi cannot enable them automatically.".into());
    }
    Ok(())
}

#[cfg(windows)]
fn native_acceleration() -> (&'static str, Result<(), String>) {
    use windows_sys::Win32::{
        Foundation::FreeLibrary,
        System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32},
    };
    let name: Vec<u16> = "WinHvPlatform.dll".encode_utf16().chain(Some(0)).collect();
    // Load the optional platform DLL from System32 only; a missing Windows feature
    // must produce setup guidance instead of preventing Lomi from opening.
    let library = unsafe {
        LoadLibraryExW(
            name.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    let result = if library.is_null() {
        let error = std::io::Error::last_os_error();
        Err(format!("Windows Hypervisor Platform could not load WinHvPlatform.dll from System32: {error}. Install or repair Windows Hypervisor Platform in Windows Features, then restart Windows. These changes require an administrator; Lomi cannot enable them automatically."))
    } else {
        let query = unsafe { GetProcAddress(library, c"WHvGetCapability".as_ptr().cast()) };
        let result = if let Some(query) = query {
            type Query =
                unsafe extern "system" fn(i32, *mut std::ffi::c_void, u32, *mut u32) -> i32;
            let query: Query = unsafe { std::mem::transmute(query) };
            let mut capability = HypervisorCapability::default();
            let mut written = 0_u32;
            // WHvCapabilityCodeHypervisorPresent = 0 returns a four-byte BOOL,
            // but WHvGetCapability requires an output buffer of at least 8 bytes.
            let status = unsafe {
                query(
                    0,
                    (&mut capability as *mut HypervisorCapability).cast(),
                    std::mem::size_of_val(&capability) as u32,
                    &mut written,
                )
            };
            interpret_hypervisor_capability(status, written, capability.present)
        } else {
            let error = std::io::Error::last_os_error();
            Err(format!("Windows Hypervisor Platform loaded WinHvPlatform.dll, but WHvGetCapability is absent: {error}. Update or repair Windows Hypervisor Platform, then retry."))
        };
        unsafe {
            FreeLibrary(library);
        }
        result
    };
    ("Windows Hypervisor Platform", result)
}
