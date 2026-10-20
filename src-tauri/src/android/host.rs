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

/// Debug native trials may exercise an unqualified host only with the same
/// isolated directory and accepted license required by the product fixture.
/// Normal application builds always use the distribution qualification flag.
pub(super) fn qualified(host: Host) -> Result<bool, String> {
    #[cfg(all(debug_assertions, feature = "android-probe"))]
    if super::fixture::directory()?.is_some() {
        return Ok(true);
    }
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

#[cfg(any(windows, test))]
#[repr(C, align(8))]
#[derive(Default)]
struct HypervisorCapability {
    present: i32,
    reserved: u32,
}

#[cfg(any(windows, test))]
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

#[cfg(test)]
mod tests {
    #[test]
    fn windows_capability_buffer_covers_the_api_minimum() {
        assert!(std::mem::size_of::<super::HypervisorCapability>() >= 8);
        assert!(std::mem::align_of::<super::HypervisorCapability>() >= 8);
    }

    #[test]
    fn windows_capability_accepts_nonzero_bool_payloads() {
        for present in [1, -1] {
            assert!(super::interpret_hypervisor_capability(0, 4, present).is_ok());
        }
    }

    #[test]
    fn windows_capability_distinguishes_query_failure_from_absence() {
        let error =
            super::interpret_hypervisor_capability(0x80070057_u32 as i32, 0, 0).unwrap_err();
        assert!(error.contains("HRESULT=0x80070057"));
        assert!(!error.contains("HypervisorPresent=false"));
        let error = super::interpret_hypervisor_capability(0, 4, 0).unwrap_err();
        assert!(error.contains("HypervisorPresent=false"));
        assert!(error.contains("nested virtualization"));
    }

    #[test]
    fn windows_capability_rejects_unexpected_payload_sizes_before_reading_bool() {
        for written in [0, 3, 8, u32::MAX] {
            for present in [0, 1] {
                let error =
                    super::interpret_hypervisor_capability(0, written, present).unwrap_err();
                assert!(error.contains("unexpected capability size"));
                assert!(error.contains(&format!("written={written}")));
                assert!(!error.contains("HypervisorPresent=false"));
            }
        }
    }

    #[test]
    fn qualified_hosts_allow_android_runtime() {
        use super::Host;
        for host in [Host::MacArm64, Host::LinuxX64, Host::WindowsX64] {
            assert!(
                super::super::bootstrap::distribution(host)
                    .unwrap()
                    .qualified
            );
            assert!(super::qualification(host).is_ok());
        }
    }

    #[test]
    fn downloadable_tools_do_not_qualify_an_untested_intel_mac_runtime() {
        let host = super::Host::MacX64;
        assert!(
            !super::super::bootstrap::distribution(host)
                .unwrap()
                .qualified
        );
        #[cfg(all(debug_assertions, feature = "android-probe"))]
        if super::super::fixture::directory().unwrap().is_some() {
            assert!(super::qualification(host).is_ok());
            return;
        }
        assert!(super::qualification(host)
            .unwrap_err()
            .contains("unavailable"));
    }

    #[test]
    fn host_probe_requires_no_android_installation() {
        let state = super::acceleration().unwrap();
        assert!(!state.backend.is_empty());
        assert_eq!(state.available, state.action.is_none());
    }
}
